//! Forged signatures, message substitution, low-S handling and key identity,
//! against the real native secp256r1 program.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::*;
use es256_authorization_verifier::VerifierError;
use es256_jws_adapter::{SyntheticSigner, is_low_s, normalize_signature};
use solana_instruction::error::InstructionError;
use solana_precompile_error::PrecompileError;

/// secp256r1 group order `n`.
const ORDER: [u8; 32] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xBC, 0xE6, 0xFA, 0xAD, 0xA7, 0x17, 0x9E, 0x84, 0xF3, 0xB9, 0xCA, 0xC2, 0xFC, 0x63, 0x25, 0x51,
];

fn with_signature(verdict: &SignedVerdict, signature: [u8; 64]) -> SignedVerdict {
    SignedVerdict {
        signature,
        ..verdict.clone()
    }
}

fn with_message(verdict: &SignedVerdict, signing_input: Vec<u8>) -> SignedVerdict {
    SignedVerdict {
        signing_input,
        ..verdict.clone()
    }
}

/// DER `SEQUENCE { INTEGER r, INTEGER s }`, the encoding ES256 does not use.
fn der(signature: &[u8; 64]) -> Vec<u8> {
    fn integer(scalar: &[u8]) -> Vec<u8> {
        let trimmed: Vec<u8> = scalar
            .iter()
            .copied()
            .skip_while(|byte| *byte == 0)
            .collect();
        let mut value = if trimmed.first().is_some_and(|byte| byte & 0x80 != 0) {
            vec![0]
        } else {
            Vec::new()
        };
        value.extend(trimmed);
        let mut encoded = vec![0x02, u8::try_from(value.len()).unwrap()];
        encoded.extend(value);
        encoded
    }
    let mut content = integer(&signature[..32]);
    content.extend(integer(&signature[32..]));
    let mut encoded = vec![0x30, u8::try_from(content.len()).unwrap()];
    encoded.extend(content);
    encoded
}

/// Rewrites the decoded payload JSON of a signing input.
fn edit_payload(signing_input: &[u8], edit: impl Fn(String) -> String) -> Vec<u8> {
    let text = std::str::from_utf8(signing_input).unwrap();
    let (header, payload) = text.split_once('.').unwrap();
    let json = String::from_utf8(es256_jws_adapter::base64url_decode(payload).unwrap()).unwrap();
    format!(
        "{header}.{}",
        es256_jws_adapter::base64url_encode(edit(json).as_bytes())
    )
    .into_bytes()
}

// ---- Forged signatures ------------------------------------------------------

#[test]
fn forged_signatures_fail_in_the_native_program() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let (r, s) = verdict.signature.split_at(32);

    let mut random = [0u8; 64];
    for (index, byte) in random.iter_mut().enumerate() {
        *byte = u8::try_from((index * 89 + 7) % 251).unwrap();
    }
    let concat = |r: &[u8], s: &[u8]| -> [u8; 64] { [r, s].concat().try_into().unwrap() };
    let mut der_prefix = [0u8; 64];
    der_prefix.copy_from_slice(&der(&verdict.signature)[..64]);
    let mut corrupted = verdict.signature;
    corrupted[40] ^= 0x80;

    let cases: [(&str, [u8; 64]); 9] = [
        ("one flipped bit", corrupted),
        ("arbitrary bytes", random),
        ("zero r", concat(&[0; 32], s)),
        ("zero s", concat(r, &[0; 32])),
        ("r equal to the order", concat(&ORDER, s)),
        ("s equal to the order", concat(r, &ORDER)),
        ("r above the order", concat(&[0xFF; 32], s)),
        ("all zero", [0; 64]),
        ("DER encoding where raw r || s is required", der_prefix),
    ];
    for (name, signature) in cases {
        let result = fixture.consume(&with_signature(&verdict, signature));
        assert_eq!(
            instruction_error(result),
            (
                0,
                InstructionError::Custom(PrecompileError::InvalidSignature as u32)
            ),
            "{name}"
        );
    }
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn truncated_signature_fails_in_the_native_program() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let mut body = verdict.public_key.to_vec();
    body.extend_from_slice(&verdict.signature[..63]);
    body.extend_from_slice(&verdict.signing_input);
    let precompile = raw_precompile(
        1,
        0,
        &[Offsets::canonical(verdict.signing_input.len())],
        &body,
    );

    assert_precompile_error(
        fixture.send_as_consumer(&[
            precompile,
            fixture.consume_instruction(verdict.nonce_hash()),
        ]),
        0,
        PrecompileError::InvalidDataOffsets,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn valid_signature_presented_with_another_valid_key_fails_natively() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let other = SyntheticSigner::generate().unwrap().public_key().unwrap();

    assert_precompile_error(
        fixture.consume(&SignedVerdict {
            public_key: other,
            ..verdict.clone()
        }),
        0,
        PrecompileError::InvalidSignature,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn key_that_is_not_a_curve_point_cannot_verify_anything() {
    let mut fixture = Fixture::new();
    // The program checks only the SEC1 prefix; x >= p is not a point.
    let mut not_a_point = [0xFF; 33];
    not_a_point[0] = 0x02;
    fixture.trusted_key = fixture
        .register_key("bad-point", not_a_point)
        .expect("only the prefix is checked on-chain");
    let verdict = SignedVerdict {
        public_key: not_a_point,
        ..fixture.sign(&VerdictClaims {
            kid: "bad-point".into(),
            ..VerdictClaims::default()
        })
    };

    assert_precompile_error(
        fixture.consume(&verdict),
        0,
        PrecompileError::InvalidPublicKey,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

// ---- Message substitution ---------------------------------------------------

/// The trusted key signed the approved verdict; the attacker presents other
/// bytes with that signature.
#[test]
fn altered_messages_fail_natively_under_the_original_signature() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let input = verdict.signing_input.clone();
    let text = String::from_utf8(input.clone()).unwrap();
    let (header, payload) = text.split_once('.').unwrap();
    let other_header = es256_jws_adapter::base64url_encode(
        br#"{"alg":"ES256","typ":"JWT","kid":"verdict-test-02"}"#,
    );
    let signature = es256_jws_adapter::base64url_encode(&verdict.signature);

    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "amount changed",
            edit_payload(&input, |json| {
                json.replace(r#""amount":340"#, r#""amount":3400"#)
            }),
        ),
        (
            "recipient changed",
            edit_payload(&input, |json| json.replace(RECIPIENT, "vendor:attacker")),
        ),
        (
            "mandate changed",
            edit_payload(&input, |json| json.replace(MANDATE_ID, "mnd_other")),
        ),
        (
            "protected header replaced",
            format!("{other_header}.{payload}").into_bytes(),
        ),
        ("payload segment only", payload.as_bytes().to_vec()),
        (
            "complete three-part JWS",
            format!("{header}.{payload}.{signature}").into_bytes(),
        ),
        (
            "SHA-256 digest of the signing input",
            solana_sha256_hasher::hash(&input).to_bytes().to_vec(),
        ),
    ];
    for (name, message) in cases {
        let result = fixture.consume(&with_message(&verdict, message));
        assert_eq!(
            instruction_error(result),
            (
                0,
                InstructionError::Custom(PrecompileError::InvalidSignature as u32)
            ),
            "{name}"
        );
    }
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

/// Even when the trusted key really signed a differently framed message,
/// the verifier only parses `BASE64URL(header) "." BASE64URL(payload)`.
#[test]
fn genuinely_signed_messages_in_other_framings_are_not_parsed_as_claims() {
    let mut fixture = Fixture::new();
    let approved = fixture.sign(&VerdictClaims::default());
    let text = String::from_utf8(approved.signing_input.clone()).unwrap();
    let (_, payload) = text.split_once('.').unwrap();
    let digest = solana_sha256_hasher::hash(&approved.signing_input)
        .to_bytes()
        .to_vec();
    // A complete compact JWS short enough to stay under the length limit.
    let short = es256_jws_adapter::signing_input(br#"{"alg":"ES256"}"#, b"{}");
    let short_signature =
        es256_jws_adapter::base64url_encode(&fixture.sign_input(short.clone()).signature);
    let three_part = format!("{}.{short_signature}", String::from_utf8(short).unwrap());

    let cases: Vec<(&str, Vec<u8>, &[VerifierError])> = vec![
        (
            "payload segment only",
            payload.as_bytes().to_vec(),
            &[VerifierError::MalformedSigningInput],
        ),
        (
            "complete three-part JWS",
            three_part.into_bytes(),
            &[VerifierError::MalformedSigningInput],
        ),
        (
            "raw JSON claims",
            VerdictClaims::default().payload_json().into_bytes(),
            &[VerifierError::MalformedSigningInput],
        ),
        (
            "SHA-256 digest",
            digest,
            &[
                VerifierError::MalformedSigningInput,
                VerifierError::InvalidBase64Url,
            ],
        ),
    ];
    for (name, message, expected) in cases {
        let verdict = fixture.sign_input(message);
        let (index, error) = instruction_error(fixture.consume(&verdict));
        assert_eq!(index, 1, "{name}: the native program accepts");
        assert!(
            expected
                .iter()
                .any(|code| error == InstructionError::Custom(u32::from(*code))),
            "{name}: got {error:?}"
        );
    }
    fixture.assert_no_receipt(&approved.nonce_hash());
}

// ---- Low S ------------------------------------------------------------------

#[test]
fn raw_high_s_is_rejected_natively_and_its_normalized_form_consumes_the_same_nonce() {
    let mut fixture = Fixture::new();
    let claims = VerdictClaims::default();
    let input = claims.signing_input();
    let raw = (0..256)
        .map(|_| fixture.signer.sign(&input).unwrap())
        .find(|signature| !is_low_s(signature).unwrap())
        .expect("OpenSSL produces a high-S signature about half the time");
    let verdict = SignedVerdict {
        signature: raw,
        ..fixture.sign(&claims)
    };

    assert_precompile_error(
        fixture.consume(&verdict),
        0,
        PrecompileError::InvalidSignature,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());

    let normalized = normalize_signature(&raw).unwrap();
    assert_eq!(normalized[..32], raw[..32], "r is unchanged");
    assert_eq!(normalize_signature(&normalized).unwrap(), normalized);
    fixture
        .consume(&with_signature(&verdict, normalized))
        .expect("the low-S twin verifies");

    // The receipt is keyed by the signed nonce, not the signature, so a
    // fresh signature over the same verdict is still a replay.
    let receipt = fixture.receipt(&verdict.nonce_hash()).unwrap();
    assert_eq!(receipt.nonce_hash, verdict.nonce_hash());
    let resigned = fixture.sign(&claims);
    assert_ne!(resigned.signature, normalized);
    assert_eq!(
        instruction_error(fixture.consume(&resigned)),
        (1, InstructionError::Custom(ACCOUNT_ALREADY_IN_USE))
    );
}

// ---- Key identity -----------------------------------------------------------

#[test]
fn registered_kid_with_different_key_bytes_is_rejected() {
    let mut fixture = Fixture::new();
    let attacker = SyntheticSigner::generate().unwrap();
    // Correct kid in the header, natively valid under the attacker's key.
    let verdict = SignedVerdict::sign(&attacker, &VerdictClaims::default());

    assert_verifier_error(
        fixture.consume(&verdict),
        1,
        VerifierError::PublicKeyMismatch,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn trusted_key_registered_under_another_kid_does_not_vouch_for_this_kid() {
    let mut fixture = Fixture::new();
    let public_key = fixture.signer.public_key().unwrap();
    // Same key bytes, but the account is for a different kid.
    fixture.trusted_key = fixture.register_key("verdict-test-02", public_key).unwrap();
    let verdict = fixture.sign(&VerdictClaims::default());

    assert_verifier_error(fixture.consume(&verdict), 1, VerifierError::KeyIdMismatch);
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn registration_rejects_keys_without_a_compressed_sec1_prefix() {
    let mut fixture = Fixture::new();
    let mut public_key = fixture.signer.public_key().unwrap();
    for prefix in [0x00, 0x01, 0x04, 0x05, 0xFF] {
        public_key[0] = prefix;
        let failure = fixture
            .register_key(&format!("prefix-{prefix:02x}"), public_key)
            .expect_err("prefix is refused");
        assert_eq!(
            failure.err,
            solana_transaction_error::TransactionError::InstructionError(
                0,
                InstructionError::Custom(u32::from(VerifierError::InvalidPublicKey))
            ),
            "prefix {prefix:#04x}"
        );
    }
}
