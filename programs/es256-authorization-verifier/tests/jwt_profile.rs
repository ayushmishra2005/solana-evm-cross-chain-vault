//! The strict JWT profile, exercised through the deployed program.
//!
//! Every token here carries a valid signature from the trusted key, so the
//! native program accepts it and any rejection comes from the verifier's
//! parser running on SBF.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use es256_authorization_verifier::VerifierError;
use es256_jws_adapter::{base64url_encode, signing_input};
use solana_instruction::error::InstructionError;

const HEADER: &str = r#"{"alg":"ES256","typ":"JWT","kid":"verdict-test-01"}"#;

/// Valid claims with short free-text values, so each mutation below stays
/// under the 512-byte signing-input limit and reaches the parser.
fn compact_claims() -> VerdictClaims {
    VerdictClaims {
        jti: "j1".into(),
        agent: "a1".into(),
        policy_version: "p1".into(),
        ..VerdictClaims::default()
    }
}

fn payload() -> String {
    compact_claims().payload_json()
}

/// `text` with exactly one occurrence of `from` replaced.
fn edit(text: &str, from: &str, to: &str) -> String {
    assert_eq!(text.matches(from).count(), 1, "{from:?} must occur once");
    text.replacen(from, to, 1)
}

fn edit_payload(from: &str, to: &str) -> String {
    edit(&payload(), from, to)
}

/// Signs and submits each signing input, asserting the verifier rejects it
/// with the expected error after native verification succeeded.
fn assert_rejections(cases: Vec<(String, Vec<u8>, VerifierError)>) {
    let mut fixture = Fixture::new();
    for (name, input, expected) in cases {
        let verdict = fixture.sign_input(input);
        let (index, error) = instruction_error(fixture.consume(&verdict));
        assert_eq!(
            (index, error),
            (1, InstructionError::Custom(u32::from(expected))),
            "{name}: expected {expected:?}"
        );
    }
    fixture.assert_no_receipt(&es256_authorization_verifier::nonce_hash(NONCE.as_bytes()));
    let valid = fixture.sign(&VerdictClaims::default());
    fixture
        .consume(&valid)
        .expect("the nonce survives every rejection");
}

fn json_case(
    name: &str,
    header: &str,
    payload: &str,
    expected: VerifierError,
) -> (String, Vec<u8>, VerifierError) {
    (
        name.to_owned(),
        signing_input(header.as_bytes(), payload.as_bytes()),
        expected,
    )
}

fn payload_case(
    name: &str,
    payload: String,
    expected: VerifierError,
) -> (String, Vec<u8>, VerifierError) {
    json_case(name, HEADER, &payload, expected)
}

fn header_case(
    name: &str,
    header: &str,
    expected: VerifierError,
) -> (String, Vec<u8>, VerifierError) {
    json_case(name, header, &payload(), expected)
}

#[test]
fn duplicate_fields_are_rejected_whichever_copy_is_approved() {
    use VerifierError::DuplicateField;
    let cases = vec![
        header_case(
            "alg approved then none",
            r#"{"alg":"ES256","alg":"none","kid":"verdict-test-01"}"#,
            DuplicateField,
        ),
        header_case(
            "alg none then approved",
            r#"{"alg":"none","alg":"ES256","kid":"verdict-test-01"}"#,
            DuplicateField,
        ),
        header_case(
            "kid trusted then other",
            r#"{"alg":"ES256","kid":"verdict-test-01","kid":"attacker"}"#,
            DuplicateField,
        ),
        header_case(
            "kid other then trusted",
            r#"{"alg":"ES256","kid":"attacker","kid":"verdict-test-01"}"#,
            DuplicateField,
        ),
        header_case(
            "typ twice",
            r#"{"alg":"ES256","typ":"JWT","typ":"JWT","kid":"verdict-test-01"}"#,
            DuplicateField,
        ),
        payload_case(
            "iss twice",
            edit_payload(
                r#""iss":"https://verdicts.saifuro.com","#,
                r#""iss":"https://verdicts.saifuro.com","iss":"https://evil.example","#,
            ),
            DuplicateField,
        ),
        payload_case(
            "verdict allow then deny",
            edit_payload(
                r#""verdict":"allow""#,
                r#""verdict":"allow","verdict":"deny""#,
            ),
            DuplicateField,
        ),
        payload_case(
            "verdict deny then allow",
            edit_payload(
                r#""verdict":"allow""#,
                r#""verdict":"deny","verdict":"allow""#,
            ),
            DuplicateField,
        ),
        payload_case(
            "nonce twice",
            edit_payload(
                r#""nonce":"n_9d02c6e4""#,
                r#""nonce":"n_9d02c6e4","nonce":"n_fresh""#,
            ),
            DuplicateField,
        ),
        payload_case(
            "mandate twice",
            edit_payload(
                r#""mandate":"mnd_7f3a""#,
                r#""mandate":"mnd_other","mandate":"mnd_7f3a""#,
            ),
            DuplicateField,
        ),
        payload_case(
            "jti twice",
            edit_payload(r#""jti":"j1""#, r#""jti":"j1","jti":"dec_other""#),
            DuplicateField,
        ),
        payload_case(
            "iat twice",
            edit_payload(
                r#""iat":1800000000"#,
                r#""iat":1800000000,"iat":1800000001"#,
            ),
            DuplicateField,
        ),
        // The first copy is validated in full before the second is seen; a
        // complete duplicate request does not fit in 512 bytes and is
        // covered by the parser's unit tests.
        payload_case(
            "empty request then the approved one",
            edit_payload(r#""request":{"#, r#""request":{},"request":{"#),
            VerifierError::MissingField,
        ),
        payload_case(
            "amount approved then inflated",
            edit_payload(r#""amount":340"#, r#""amount":340,"amount":3400"#),
            DuplicateField,
        ),
        payload_case(
            "amount inflated then approved",
            edit_payload(r#""amount":340"#, r#""amount":3400,"amount":340"#),
            DuplicateField,
        ),
        payload_case(
            "recipient twice",
            edit_payload(
                r#""recipient":"vendor:acme_saas""#,
                r#""recipient":"vendor:attacker","recipient":"vendor:acme_saas""#,
            ),
            DuplicateField,
        ),
        payload_case(
            "currency twice",
            edit_payload(
                r#""currency":"USD""#,
                r#""currency":"USD","currency":"EUR""#,
            ),
            DuplicateField,
        ),
    ];
    assert_rejections(cases);
}

#[test]
fn ambiguous_json_is_rejected() {
    use VerifierError::{
        InvalidFieldLength, MalformedJson, MissingField, UnknownField, UnsupportedJsonValue,
    };
    let full = payload();
    let cases = vec![
        payload_case(
            "escaped field name",
            edit_payload(r#""recipient":"#, r#""rec\u0069pient":"#),
            MalformedJson,
        ),
        payload_case(
            "unicode escape in a value",
            edit_payload(r#""vendor:acme_saas""#, r#""vendor:acme\u005fsaas""#),
            MalformedJson,
        ),
        payload_case(
            "escaped quote in a value",
            edit_payload(r#""a1""#, r#""a\"1""#),
            MalformedJson,
        ),
        payload_case(
            "escaped backslash",
            edit_payload(r#""a1""#, r#""a\\1""#),
            MalformedJson,
        ),
        header_case(
            "escaped header name",
            r#"{"alg":"ES256","k\u0069d":"verdict-test-01"}"#,
            MalformedJson,
        ),
        payload_case(
            "array value",
            edit_payload(r#""jti":"j1""#, r#""jti":["j1"]"#),
            UnsupportedJsonValue,
        ),
        payload_case(
            "null value",
            edit_payload(r#""jti":"j1""#, r#""jti":null"#),
            UnsupportedJsonValue,
        ),
        payload_case(
            "boolean in a string field",
            edit_payload(r#""verdict":"allow""#, r#""verdict":true"#),
            UnsupportedJsonValue,
        ),
        payload_case(
            "number in a string field",
            edit_payload(r#""currency":"USD""#, r#""currency":840"#),
            UnsupportedJsonValue,
        ),
        payload_case(
            "string in a number field",
            edit_payload(r#""amount":340"#, r#""amount":"340""#),
            UnsupportedJsonValue,
        ),
        payload_case(
            "object in a string field",
            edit_payload(r#""mandate":"mnd_7f3a""#, r#""mandate":{"id":"mnd_7f3a"}"#),
            UnsupportedJsonValue,
        ),
        payload_case(
            "string where the request object belongs",
            edit_payload(
                r#""request":{"agent":"a1","amount":340,"currency":"USD","recipient":"vendor:acme_saas","category":"saas"}"#,
                r#""request":"approved""#,
            ),
            MalformedJson,
        ),
        payload_case(
            "unknown top-level field",
            edit_payload(r#""jti":"#, r#""sub":"agent","jti":"#),
            UnknownField,
        ),
        payload_case(
            "unknown request field",
            edit_payload(r#""category":"saas""#, r#""category":"saas","memo":"x""#),
            UnknownField,
        ),
        payload_case(
            "top-level claim inside request",
            edit_payload(r#""category":"saas""#, r#""category":"saas","nonce":"n_x""#),
            UnknownField,
        ),
        payload_case(
            "missing nonce",
            edit_payload(r#""nonce":"n_9d02c6e4","#, ""),
            MissingField,
        ),
        payload_case(
            "missing request currency",
            edit_payload(r#""currency":"USD","#, ""),
            MissingField,
        ),
        header_case(
            "missing kid",
            r#"{"alg":"ES256","typ":"JWT"}"#,
            MissingField,
        ),
        header_case(
            "missing alg",
            r#"{"typ":"JWT","kid":"verdict-test-01"}"#,
            MissingField,
        ),
        payload_case("trailing garbage", format!("{full}x"), MalformedJson),
        payload_case("second object", format!("{full}{{}}"), MalformedJson),
        payload_case(
            "truncated",
            full[..full.len() - 1].to_owned(),
            MalformedJson,
        ),
        payload_case("top-level array", format!("[{full}]"), MalformedJson),
        payload_case(
            "equals instead of colon",
            edit_payload(r#""jti":"#, r#""jti"="#),
            MalformedJson,
        ),
        payload_case(
            "semicolon separator",
            edit_payload(r#""j1","#, r#""j1";"#),
            MalformedJson,
        ),
        payload_case(
            "trailing comma",
            edit_payload(r#""category":"saas"}"#, r#""category":"saas",}"#),
            MalformedJson,
        ),
        payload_case(
            "single-quoted string",
            edit_payload(r#""jti":"j1""#, r#""jti":'j1'"#),
            UnsupportedJsonValue,
        ),
        payload_case(
            "empty nonce",
            edit_payload(r#""n_9d02c6e4""#, r#""""#),
            InvalidFieldLength,
        ),
        payload_case(
            "empty jti",
            edit_payload(r#""j1""#, r#""""#),
            InvalidFieldLength,
        ),
        payload_case(
            "nonce over 64 bytes",
            edit_payload(r#""n_9d02c6e4""#, &format!("\"{}\"", "n".repeat(65))),
            InvalidFieldLength,
        ),
        payload_case(
            "category over 32 bytes",
            edit_payload(r#""saas""#, &format!("\"{}\"", "c".repeat(33))),
            InvalidFieldLength,
        ),
        payload_case(
            "currency over 16 bytes",
            edit_payload(r#""USD""#, &format!("\"{}\"", "U".repeat(17))),
            InvalidFieldLength,
        ),
        header_case(
            "kid over 32 bytes",
            &format!(r#"{{"alg":"ES256","kid":"{}"}}"#, "k".repeat(33)),
            InvalidFieldLength,
        ),
        payload_case(
            "tab inside a string",
            edit_payload(r#""j1""#, "\"j\t1\""),
            MalformedJson,
        ),
        payload_case(
            "newline inside a string",
            edit_payload(r#""j1""#, "\"j\n1\""),
            MalformedJson,
        ),
        payload_case(
            "DEL inside a string",
            edit_payload(r#""j1""#, "\"j\u{7f}1\""),
            MalformedJson,
        ),
        payload_case(
            "non-ASCII UTF-8 inside a string",
            edit_payload(r#""vendor:acme_saas""#, "\"vendor:acmé_saas\""),
            MalformedJson,
        ),
    ];
    let mut cases = cases;
    let mut invalid_utf8 = payload().into_bytes();
    let jti = payload().find(r#""j1""#).unwrap();
    invalid_utf8[jti + 1] = 0xFF;
    cases.push((
        "invalid UTF-8 byte".to_owned(),
        signing_input(HEADER.as_bytes(), &invalid_utf8),
        MalformedJson,
    ));
    assert_rejections(cases);
}

#[test]
fn noncanonical_base64url_and_segment_structure_are_rejected() {
    use VerifierError::{InvalidBase64Url, MalformedSigningInput};
    let header = base64url_encode(HEADER.as_bytes());
    // Pad the payload so its encoding ends in a partial group whose unused
    // bits can be set.
    let mut padded = payload();
    while padded.len() % 3 != 1 {
        padded.push(' ');
    }
    let body = base64url_encode(padded.as_bytes());
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let last = body.as_bytes()[body.len() - 1];
    let value = alphabet.iter().position(|symbol| *symbol == last).unwrap();
    assert_eq!(
        value % 16,
        0,
        "canonical encodings leave the low bits clear"
    );
    let mut noncanonical = body.clone();
    noncanonical.pop();
    noncanonical.push(char::from(alphabet[value + 1]));

    let with_payload = |payload: &str| format!("{header}.{payload}").into_bytes();
    let replace_at = |text: &str, index: usize, symbol: char| {
        let mut bytes = text.to_owned().into_bytes();
        bytes[index] = u8::try_from(u32::from(symbol)).unwrap();
        String::from_utf8(bytes).unwrap()
    };

    // Same padded shape, differing only in the issuer: proves the padded
    // payload itself parses, so the other cases fail on their encoding.
    let control = base64url_encode(
        edit(
            &padded,
            "https://verdicts.saifuro.com",
            "https://verdicts.saifuro.co",
        )
        .as_bytes(),
    );
    let mut misaligned = body.clone();
    while misaligned.len() % 4 != 1 {
        misaligned.push('A');
    }

    let cases: Vec<(String, Vec<u8>, VerifierError)> = vec![
        (
            "canonical control".into(),
            with_payload(&control),
            VerifierError::IssuerMismatch,
        ),
        (
            "nonzero trailing bits".into(),
            with_payload(&noncanonical),
            InvalidBase64Url,
        ),
        (
            "standard alphabet plus".into(),
            with_payload(&replace_at(&body, 5, '+')),
            InvalidBase64Url,
        ),
        (
            "standard alphabet slash".into(),
            with_payload(&replace_at(&body, 5, '/')),
            InvalidBase64Url,
        ),
        (
            "character outside any alphabet".into(),
            with_payload(&replace_at(&body, 5, '*')),
            InvalidBase64Url,
        ),
        (
            "padding".into(),
            with_payload(&format!("{body}==")),
            InvalidBase64Url,
        ),
        (
            "whitespace inside a segment".into(),
            with_payload(&replace_at(&body, 5, ' ')),
            InvalidBase64Url,
        ),
        (
            "length 1 mod 4".into(),
            with_payload(&misaligned),
            InvalidBase64Url,
        ),
        (
            "empty header".into(),
            format!(".{body}").into_bytes(),
            MalformedSigningInput,
        ),
        (
            "empty payload".into(),
            format!("{header}.").into_bytes(),
            MalformedSigningInput,
        ),
        (
            "three segments".into(),
            format!("{header}.{body}.e30").into_bytes(),
            MalformedSigningInput,
        ),
        (
            "no separator".into(),
            format!("{header}{body}").into_bytes(),
            MalformedSigningInput,
        ),
        (
            "non-ASCII byte in a segment".into(),
            {
                let mut input = with_payload(&body);
                input[header.len() + 4] = 0xC3;
                input
            },
            InvalidBase64Url,
        ),
    ];
    assert_rejections(cases);
}

#[test]
fn amount_spellings_are_exact_or_rejected() {
    use VerifierError::{AmountMismatch, InvalidNumber, UnsupportedJsonValue};
    let amount = |token: &str| edit_payload(r#""amount":340"#, &format!(r#""amount":{token}"#));
    let cases = vec![
        payload_case("one micro-unit more", amount("340.000001"), AmountMismatch),
        payload_case("zero", amount("0"), AmountMismatch),
        payload_case(
            "seven fractional digits",
            amount("340.0000000"),
            InvalidNumber,
        ),
        payload_case("negative", amount("-340"), InvalidNumber),
        payload_case("exponent", amount("3.4e2"), InvalidNumber),
        payload_case("capital exponent", amount("34E1"), InvalidNumber),
        payload_case("plus sign", amount("+340"), InvalidNumber),
        payload_case("leading zero", amount("0340"), InvalidNumber),
        payload_case("bare trailing point", amount("340."), InvalidNumber),
        payload_case("bare leading point", amount(".5"), InvalidNumber),
        payload_case("two points", amount("340.0.0"), InvalidNumber),
        payload_case(
            "fourteen integer digits",
            amount("10000000000000"),
            InvalidNumber,
        ),
        payload_case(
            "u64::MAX micro-units",
            amount("18446744073709.551615"),
            InvalidNumber,
        ),
        payload_case(
            "twenty-digit integer",
            amount("99999999999999999999"),
            InvalidNumber,
        ),
        payload_case("NaN", amount("NaN"), UnsupportedJsonValue),
        payload_case("Infinity", amount("Infinity"), UnsupportedJsonValue),
    ];
    assert_rejections(cases);
}

#[test]
fn equivalent_amount_spellings_match_the_same_mandate() {
    let mut fixture = Fixture::new();
    for (index, token) in ["340", "340.0", "340.000000"].iter().enumerate() {
        let verdict = fixture.sign(&VerdictClaims {
            amount: (*token).into(),
            nonce: format!("n_amount_{index}"),
            ..VerdictClaims::default()
        });
        fixture
            .consume(&verdict)
            .unwrap_or_else(|error| panic!("{token} should equal 340: {:?}", error.err));
    }
}

#[test]
fn timestamp_spellings_are_rejected() {
    use VerifierError::InvalidNumber;
    let iat = |token: &str| edit_payload(r#""iat":1800000000"#, &format!(r#""iat":{token}"#));
    let cases = vec![
        payload_case("negative", iat("-1800000000"), InvalidNumber),
        payload_case("fraction", iat("1800000000.0"), InvalidNumber),
        payload_case("exponent", iat("1.8e9"), InvalidNumber),
        payload_case("leading zero", iat("01800000000"), InvalidNumber),
        payload_case("zero", iat("0"), InvalidNumber),
        payload_case("thirteen digits", iat("1000000000000"), InvalidNumber),
        payload_case("past year 9999", iat("253402300800"), InvalidNumber),
        payload_case(
            "exp past year 9999",
            edit_payload(r#""exp":1800000300"#, r#""exp":253402301099"#),
            InvalidNumber,
        ),
    ];
    assert_rejections(cases);
}

#[test]
fn unsupported_headers_and_claims_are_rejected() {
    use VerifierError::{UnknownField, UnsupportedAlgorithm, UnsupportedClaim, UnsupportedType};
    let header =
        |extra: &str| format!(r#"{{"alg":"ES256","typ":"JWT","kid":"verdict-test-01",{extra}}}"#);
    let mut cases = vec![];
    for alg in [
        "none", "HS256", "RS256", "ES384", "ES256K", "es256", "EdDSA",
    ] {
        cases.push(header_case(
            &format!("alg {alg}"),
            &format!(r#"{{"alg":"{alg}","kid":"verdict-test-01"}}"#),
            UnsupportedAlgorithm,
        ));
    }
    for typ in ["jwt", "at+jwt", "JOSE"] {
        cases.push(header_case(
            &format!("typ {typ}"),
            &format!(r#"{{"alg":"ES256","typ":"{typ}","kid":"verdict-test-01"}}"#),
            UnsupportedType,
        ));
    }
    for extra in [
        r#""jwk":{"kty":"EC","crv":"P-256","x":"AA","y":"AA"}"#,
        r#""jku":"https://evil.example/jwks.json""#,
        r#""x5c":["MIIB"]"#,
        r#""x5u":"https://evil.example/cert.pem""#,
        r#""x5t":"AA""#,
        r#""x5t#S256":"AA""#,
        r#""crit":["b64"]"#,
        r#""b64":false"#,
    ] {
        cases.push(header_case(extra, &header(extra), UnsupportedClaim));
    }
    cases.push(header_case(
        "cty header",
        &header(r#""cty":"JWT""#),
        UnknownField,
    ));
    for claim in [
        r#""aud":"solana""#,
        r#""nbf":1800000000"#,
        r#""supersedes":"dec_0000""#,
    ] {
        cases.push(payload_case(
            claim,
            edit_payload(r#""jti":"#, &format!(r#"{claim},"jti":"#)),
            UnsupportedClaim,
        ));
    }
    assert_rejections(cases);
}

#[test]
fn profile_accepts_insignificant_whitespace_field_order_and_omitted_typ() {
    let mut fixture = Fixture::new();
    type Variant = (&'static str, fn(String) -> String, &'static str);
    let variants: [Variant; 3] = [
        (
            "whitespace between tokens",
            |json| {
                let json = json.replacen('{', "{ ", 1).replacen(
                    ",\"request\":",
                    " ,\n\t\"request\" :\r",
                    1,
                );
                format!("{json}\n")
            },
            HEADER,
        ),
        (
            "request object first",
            |json| {
                let (head, request) = json.split_once(r#","request":"#).unwrap();
                format!(
                    "{{\"request\":{},{}}}",
                    &request[..request.len() - 1],
                    &head[1..]
                )
            },
            HEADER,
        ),
        (
            "typ omitted",
            |json| json,
            r#"{"kid":"verdict-test-01","alg":"ES256"}"#,
        ),
    ];
    for (index, (name, transform, header)) in variants.into_iter().enumerate() {
        let claims = VerdictClaims {
            nonce: format!("n_variant_{index}"),
            ..VerdictClaims::default()
        };
        let input = signing_input(
            header.as_bytes(),
            transform(claims.payload_json()).as_bytes(),
        );
        let verdict = SignedVerdict::sign_input(&fixture.signer, input, &claims.nonce);
        fixture
            .consume(&verdict)
            .unwrap_or_else(|error| panic!("{name}: {:?}", error.err));
    }
}
