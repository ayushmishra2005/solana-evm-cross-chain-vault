//! Strict parser for the one compact-JWS verdict profile this program accepts.
//!
//! The grammar is a subset of JSON: objects, strings of printable ASCII with
//! no escapes, and unsigned decimal numbers. Only the `request` claim may be
//! a nested object. Unknown and repeated fields are errors, so two parsers
//! can never disagree about which value a field holds.

use crate::errors::VerifierError;
use crate::state::{
    MAX_AGENT_LEN, MAX_CATEGORY_LEN, MAX_CURRENCY_LEN, MAX_JTI_LEN, MAX_KID_LEN, MAX_MANDATE_LEN,
    MAX_NONCE_LEN, MAX_POLICY_LEN, MAX_RECIPIENT_LEN,
};

type ParseResult<T> = core::result::Result<T, VerifierError>;

/// `9999-12-31T23:59:59Z`.
pub const MAX_TIMESTAMP: i64 = 253_402_300_799;
pub const AMOUNT_SCALE_DIGITS: usize = 6;
const AMOUNT_SCALE: u64 = 1_000_000;
const MAX_AMOUNT_INTEGER_DIGITS: usize = 13;
const MAX_TIMESTAMP_DIGITS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header<'a> {
    pub kid: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request<'a> {
    pub agent: &'a [u8],
    pub amount_micros: u64,
    pub currency: &'a [u8],
    pub recipient: &'a [u8],
    pub category: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Claims<'a> {
    pub iss: &'a [u8],
    pub jti: &'a [u8],
    pub iat: i64,
    pub exp: i64,
    pub verdict: &'a [u8],
    pub mandate: &'a [u8],
    pub policy_version: &'a [u8],
    pub nonce: &'a [u8],
    pub request: Request<'a>,
}

/// The two decoded JSON documents of a compact-JWS signing input.
#[derive(Debug)]
pub struct DecodedSigningInput {
    pub header_json: Vec<u8>,
    pub payload_json: Vec<u8>,
}

impl DecodedSigningInput {
    pub fn header(&self) -> ParseResult<Header<'_>> {
        parse_header(&self.header_json)
    }

    pub fn claims(&self) -> ParseResult<Claims<'_>> {
        parse_claims(&self.payload_json)
    }
}

/// Splits `BASE64URL(header) "." BASE64URL(payload)` without altering it.
pub fn decode_signing_input(signing_input: &[u8]) -> ParseResult<DecodedSigningInput> {
    let mut segments = signing_input.split(|byte| *byte == b'.');
    let header = segments
        .next()
        .ok_or(VerifierError::MalformedSigningInput)?;
    let payload = segments
        .next()
        .ok_or(VerifierError::MalformedSigningInput)?;
    if segments.next().is_some() || header.is_empty() || payload.is_empty() {
        return Err(VerifierError::MalformedSigningInput);
    }
    Ok(DecodedSigningInput {
        header_json: decode_base64url(header)?,
        payload_json: decode_base64url(payload)?,
    })
}

/// Unpadded Base64URL whose unused trailing bits are zero, so every byte
/// string has exactly one accepted encoding.
pub fn decode_base64url(input: &[u8]) -> ParseResult<Vec<u8>> {
    if input.len() % 4 == 1 {
        return Err(VerifierError::InvalidBase64Url);
    }
    let mut output = Vec::with_capacity(input.len().saturating_mul(3) / 4);
    for chunk in input.chunks(4) {
        let mut word: u32 = 0;
        for (&symbol, shift) in chunk.iter().zip([18u32, 12, 6, 0]) {
            word |= sextet(symbol)?.wrapping_shl(shift);
        }
        let [_, first, second, third] = word.to_be_bytes();
        match chunk.len() {
            4 => output.extend_from_slice(&[first, second, third]),
            3 if third == 0 => output.extend_from_slice(&[first, second]),
            2 if second == 0 && third == 0 => output.push(first),
            _ => return Err(VerifierError::InvalidBase64Url),
        }
    }
    Ok(output)
}

fn sextet(symbol: u8) -> ParseResult<u32> {
    let value = match symbol {
        b'A'..=b'Z' => symbol.wrapping_sub(b'A'),
        b'a'..=b'z' => symbol.wrapping_sub(b'a').wrapping_add(26),
        b'0'..=b'9' => symbol.wrapping_sub(b'0').wrapping_add(52),
        b'-' => 62,
        b'_' => 63,
        _ => return Err(VerifierError::InvalidBase64Url),
    };
    Ok(u32::from(value))
}

pub fn parse_header(json: &[u8]) -> ParseResult<Header<'_>> {
    let mut alg = None;
    let mut kid = None;
    let mut typ = None;
    let mut cursor = Cursor::new(json);
    cursor.object(|cursor, key| match key {
        b"alg" => fill(&mut alg, || cursor.string()),
        b"kid" => fill(&mut kid, || cursor.string()),
        b"typ" => fill(&mut typ, || cursor.string()),
        // Keys must come from the program's own state, never the token.
        b"jwk" | b"jku" | b"x5c" | b"x5u" | b"x5t" | b"x5t#S256" | b"crit" | b"b64" => {
            Err(VerifierError::UnsupportedClaim)
        }
        _ => Err(VerifierError::UnknownField),
    })?;
    cursor.finish()?;

    if required(alg)? != b"ES256" {
        return Err(VerifierError::UnsupportedAlgorithm);
    }
    if typ.is_some_and(|value| value != b"JWT") {
        return Err(VerifierError::UnsupportedType);
    }
    Ok(Header {
        kid: bounded(required(kid)?, MAX_KID_LEN)?,
    })
}

pub fn parse_claims(json: &[u8]) -> ParseResult<Claims<'_>> {
    let mut iss = None;
    let mut jti = None;
    let mut iat = None;
    let mut exp = None;
    let mut verdict = None;
    let mut mandate = None;
    let mut policy_version = None;
    let mut nonce = None;
    let mut request = None;
    let mut cursor = Cursor::new(json);
    cursor.object(|cursor, key| match key {
        b"iss" => fill(&mut iss, || cursor.string()),
        b"jti" => fill(&mut jti, || cursor.string()),
        b"iat" => fill(&mut iat, || parse_timestamp(cursor.number()?)),
        b"exp" => fill(&mut exp, || parse_timestamp(cursor.number()?)),
        b"verdict" => fill(&mut verdict, || cursor.string()),
        b"mandate" => fill(&mut mandate, || cursor.string()),
        b"policy_version" => fill(&mut policy_version, || cursor.string()),
        b"nonce" => fill(&mut nonce, || cursor.string()),
        b"request" => fill(&mut request, || parse_request(cursor)),
        b"aud" | b"nbf" | b"supersedes" => Err(VerifierError::UnsupportedClaim),
        _ => Err(VerifierError::UnknownField),
    })?;
    cursor.finish()?;

    Ok(Claims {
        iss: required(iss)?,
        jti: bounded(required(jti)?, MAX_JTI_LEN)?,
        iat: required(iat)?,
        exp: required(exp)?,
        verdict: required(verdict)?,
        mandate: bounded(required(mandate)?, MAX_MANDATE_LEN)?,
        policy_version: bounded(required(policy_version)?, MAX_POLICY_LEN)?,
        nonce: bounded(required(nonce)?, MAX_NONCE_LEN)?,
        request: required(request)?,
    })
}

fn parse_request<'a>(cursor: &mut Cursor<'a>) -> ParseResult<Request<'a>> {
    let mut agent = None;
    let mut amount = None;
    let mut currency = None;
    let mut recipient = None;
    let mut category = None;
    cursor.object(|cursor, key| match key {
        b"agent" => fill(&mut agent, || cursor.string()),
        b"amount" => fill(&mut amount, || parse_amount_micros(cursor.number()?)),
        b"currency" => fill(&mut currency, || cursor.string()),
        b"recipient" => fill(&mut recipient, || cursor.string()),
        b"category" => fill(&mut category, || cursor.string()),
        _ => Err(VerifierError::UnknownField),
    })?;

    Ok(Request {
        agent: bounded(required(agent)?, MAX_AGENT_LEN)?,
        amount_micros: required(amount)?,
        currency: bounded(required(currency)?, MAX_CURRENCY_LEN)?,
        recipient: bounded(required(recipient)?, MAX_RECIPIENT_LEN)?,
        category: bounded(required(category)?, MAX_CATEGORY_LEN)?,
    })
}

/// Rejects a repeated field before reading its value.
fn fill<T>(slot: &mut Option<T>, read: impl FnOnce() -> ParseResult<T>) -> ParseResult<()> {
    if slot.is_some() {
        return Err(VerifierError::DuplicateField);
    }
    *slot = Some(read()?);
    Ok(())
}

fn required<T>(slot: Option<T>) -> ParseResult<T> {
    slot.ok_or(VerifierError::MissingField)
}

fn bounded(value: &[u8], max_len: usize) -> ParseResult<&[u8]> {
    if value.is_empty() || value.len() > max_len {
        return Err(VerifierError::InvalidFieldLength);
    }
    Ok(value)
}

/// A positive integer with no sign, fraction, exponent or leading zero.
pub fn parse_timestamp(token: &[u8]) -> ParseResult<i64> {
    if token.len() > MAX_TIMESTAMP_DIGITS || token.first() == Some(&b'0') {
        return Err(VerifierError::InvalidNumber);
    }
    let value = parse_digits(token)?;
    let value = i64::try_from(value).map_err(|_| VerifierError::InvalidNumber)?;
    if value == 0 || value > MAX_TIMESTAMP {
        return Err(VerifierError::InvalidNumber);
    }
    Ok(value)
}

/// A non-negative decimal with at most six fractional digits, scaled by
/// 10^6 so that `340` and `340.000000` are the same value.
pub fn parse_amount_micros(token: &[u8]) -> ParseResult<u64> {
    let mut parts = token.splitn(2, |byte| *byte == b'.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next();

    let leading_zero = integer.len() > 1 && integer.first() == Some(&b'0');
    if integer.len() > MAX_AMOUNT_INTEGER_DIGITS || leading_zero {
        return Err(VerifierError::InvalidNumber);
    }
    let whole = parse_digits(integer)?
        .checked_mul(AMOUNT_SCALE)
        .ok_or(VerifierError::InvalidNumber)?;

    let micros = match fraction {
        None => 0,
        Some(digits) => {
            if digits.len() > AMOUNT_SCALE_DIGITS {
                return Err(VerifierError::InvalidNumber);
            }
            let padding = AMOUNT_SCALE_DIGITS.saturating_sub(digits.len());
            let scale = 10u64
                .checked_pow(u32::try_from(padding).map_err(|_| VerifierError::InvalidNumber)?)
                .ok_or(VerifierError::InvalidNumber)?;
            parse_digits(digits)?
                .checked_mul(scale)
                .ok_or(VerifierError::InvalidNumber)?
        }
    };
    whole
        .checked_add(micros)
        .ok_or(VerifierError::InvalidNumber)
}

/// One or more ASCII digits, accumulated with overflow checks.
fn parse_digits(digits: &[u8]) -> ParseResult<u64> {
    if digits.is_empty() {
        return Err(VerifierError::InvalidNumber);
    }
    digits.iter().try_fold(0u64, |value, digit| {
        if !digit.is_ascii_digit() {
            return Err(VerifierError::InvalidNumber);
        }
        value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(digit.wrapping_sub(b'0'))))
            .ok_or(VerifierError::InvalidNumber)
    })
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn advance(&mut self) -> ParseResult<u8> {
        let byte = self.peek().ok_or(VerifierError::MalformedJson)?;
        self.position = self
            .position
            .checked_add(1)
            .ok_or(VerifierError::MalformedJson)?;
        Ok(byte)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.position = self.position.saturating_add(1);
        }
    }

    fn expect(&mut self, expected: u8) -> ParseResult<()> {
        if self.advance()? != expected {
            return Err(VerifierError::MalformedJson);
        }
        Ok(())
    }

    /// Visits each field of an object; `visit` must consume the value.
    fn object(
        &mut self,
        mut visit: impl FnMut(&mut Self, &'a [u8]) -> ParseResult<()>,
    ) -> ParseResult<()> {
        self.skip_whitespace();
        self.expect(b'{')?;
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.advance()?;
            return Ok(());
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(VerifierError::MalformedJson);
            }
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            visit(self, key)?;
            self.skip_whitespace();
            match self.advance()? {
                b',' => continue,
                b'}' => return Ok(()),
                _ => return Err(VerifierError::MalformedJson),
            }
        }
    }

    /// A string of printable ASCII. Escapes are rejected outright, so a
    /// field name or value has exactly one spelling.
    fn string(&mut self) -> ParseResult<&'a [u8]> {
        if self.peek() != Some(b'"') {
            return Err(VerifierError::UnsupportedJsonValue);
        }
        self.advance()?;
        let start = self.position;
        loop {
            match self.advance()? {
                b'"' => break,
                b'\\' => return Err(VerifierError::MalformedJson),
                0x20..=0x7e => {}
                _ => return Err(VerifierError::MalformedJson),
            }
        }
        let end = self.position.saturating_sub(1);
        self.bytes
            .get(start..end)
            .ok_or(VerifierError::MalformedJson)
    }

    /// The raw token of a number; the caller decides which forms it accepts.
    fn number(&mut self) -> ParseResult<&'a [u8]> {
        let start = self.position;
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'-' | b'+' | b'e' | b'E')
        ) {
            self.advance()?;
        }
        let token = self
            .bytes
            .get(start..self.position)
            .ok_or(VerifierError::MalformedJson)?;
        if token.is_empty() {
            return Err(VerifierError::UnsupportedJsonValue);
        }
        Ok(token)
    }

    fn finish(&mut self) -> ParseResult<()> {
        self.skip_whitespace();
        if self.position != self.bytes.len() {
            return Err(VerifierError::MalformedJson);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]

    use super::*;

    fn code(error: VerifierError) -> u32 {
        error as u32
    }

    const CLAIMS: &[u8] = br#"{"iss":"https://verdicts.saifuro.com","jti":"dec_5b2e","iat":1788424980,"exp":1788425280,"verdict":"allow","mandate":"mnd_7f3a","policy_version":"pol_2026-08-12.1","nonce":"n_9d02c6e4","request":{"agent":"agt_procurement_01","amount":340,"currency":"USD","recipient":"vendor:acme_saas","category":"saas"}}"#;

    #[test]
    fn parses_the_documented_claim_shape() {
        let claims = parse_claims(CLAIMS).unwrap();
        assert_eq!(claims.iat, 1_788_424_980);
        assert_eq!(claims.exp, 1_788_425_280);
        assert_eq!(claims.request.amount_micros, 340_000_000);
        assert_eq!(claims.request.recipient, b"vendor:acme_saas");
        assert_eq!(claims.nonce, b"n_9d02c6e4");
    }

    #[test]
    fn parses_the_documented_header() {
        let header =
            parse_header(br#"{"alg":"ES256","typ":"JWT","kid":"verdict-2026-09"}"#).unwrap();
        assert_eq!(header.kid, b"verdict-2026-09");
    }

    #[test]
    fn rejects_duplicate_fields_at_both_levels() {
        let top = br#"{"alg":"ES256","kid":"a","kid":"b"}"#;
        assert_eq!(
            code(parse_header(top).unwrap_err()),
            code(VerifierError::DuplicateField)
        );
        let nested = String::from_utf8(CLAIMS.to_vec())
            .unwrap()
            .replace(r#""amount":340"#, r#""amount":340,"amount":1"#);
        assert_eq!(
            code(parse_claims(nested.as_bytes()).unwrap_err()),
            code(VerifierError::DuplicateField)
        );
    }

    #[test]
    fn scales_amounts_exactly() {
        assert_eq!(parse_amount_micros(b"0").unwrap(), 0);
        assert_eq!(parse_amount_micros(b"340.5").unwrap(), 340_500_000);
        assert_eq!(parse_amount_micros(b"0.000001").unwrap(), 1);
        assert_eq!(
            parse_amount_micros(b"9999999999999.999999").unwrap(),
            9_999_999_999_999_999_999
        );
        for rejected in [
            &b"01"[..],
            b"1.",
            b".5",
            b"1.0000001",
            b"-1",
            b"1e3",
            b"10000000000000",
        ] {
            assert!(parse_amount_micros(rejected).is_err(), "{rejected:?}");
        }
    }

    #[test]
    fn decodes_only_canonical_base64url() {
        assert_eq!(decode_base64url(b"QQ").unwrap(), b"A");
        assert!(decode_base64url(b"QR").is_err());
        assert!(decode_base64url(b"QQ==").is_err());
        assert!(decode_base64url(b"Q").is_err());
        assert!(decode_base64url(b"Q+8").is_err());
    }

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    fn encode(bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        for chunk in bytes.chunks(3) {
            let mut word = [0u8; 3];
            word[..chunk.len()].copy_from_slice(chunk);
            let value = u32::from_be_bytes([0, word[0], word[1], word[2]]);
            let symbols = [18, 12, 6, 0].map(|shift| ALPHABET[((value >> shift) & 63) as usize]);
            output.extend_from_slice(&symbols[..chunk.len() + 1]);
        }
        output
    }

    /// Whatever decodes re-encodes to exactly the input, so no byte string
    /// has two accepted spellings. A final symbol carries 2 data bits after
    /// one byte and 4 after two, so exactly 4 or 16 symbols may end a group.
    #[test]
    fn every_accepted_encoding_is_the_canonical_one() {
        for first in 0..=255u8 {
            let single = encode(&[first]);
            assert_eq!(decode_base64url(&single).unwrap(), [first]);
            let mut accepted = 0;
            for symbol in ALPHABET {
                let mut candidate = single.clone();
                candidate[1] = *symbol;
                if let Ok(decoded) = decode_base64url(&candidate) {
                    assert_eq!(encode(&decoded), candidate);
                    accepted += 1;
                }
            }
            assert_eq!(accepted, 4);
            for second in [0u8, 1, 0x7f, 0x80, 0xfe, 0xff, first] {
                let pair = encode(&[first, second]);
                assert_eq!(decode_base64url(&pair).unwrap(), [first, second]);
                let mut accepted = 0;
                for symbol in ALPHABET {
                    let mut candidate = pair.clone();
                    candidate[2] = *symbol;
                    if let Ok(decoded) = decode_base64url(&candidate) {
                        assert_eq!(encode(&decoded), candidate);
                        accepted += 1;
                    }
                }
                assert_eq!(accepted, 16);
            }
        }
    }

    #[test]
    fn base64url_rejects_every_byte_outside_the_alphabet() {
        for byte in 0..=255u8 {
            let input = [b'Q', b'Q', byte, b'A'];
            assert_eq!(
                decode_base64url(&input).is_ok(),
                ALPHABET.contains(&byte),
                "{byte:#04x}"
            );
        }
    }

    /// Duplicate checks that need more room than one 512-byte signing
    /// input allows on-chain.
    #[test]
    fn rejects_a_complete_duplicate_request_object() {
        let request = r#""request":{"agent":"agt_procurement_01","amount":340,"currency":"USD","recipient":"vendor:acme_saas","category":"saas"}"#;
        let duplicated = String::from_utf8(CLAIMS.to_vec()).unwrap().replace(
            request,
            &format!(
                r#"{},{}"#,
                request.replace("vendor:acme_saas", "vendor:attacker"),
                request
            ),
        );
        assert_eq!(
            code(parse_claims(duplicated.as_bytes()).unwrap_err()),
            code(VerifierError::DuplicateField)
        );
    }

    #[test]
    fn rejects_every_duplicated_claim() {
        let text = String::from_utf8(CLAIMS.to_vec()).unwrap();
        for field in [
            r#""iss":"https://verdicts.saifuro.com""#,
            r#""jti":"dec_5b2e""#,
            r#""iat":1788424980"#,
            r#""exp":1788425280"#,
            r#""verdict":"allow""#,
            r#""mandate":"mnd_7f3a""#,
            r#""policy_version":"pol_2026-08-12.1""#,
            r#""nonce":"n_9d02c6e4""#,
            r#""agent":"agt_procurement_01""#,
            r#""amount":340"#,
            r#""currency":"USD""#,
            r#""recipient":"vendor:acme_saas""#,
            r#""category":"saas""#,
        ] {
            let duplicated = text.replacen(field, &format!("{field},{field}"), 1);
            assert_eq!(
                code(parse_claims(duplicated.as_bytes()).unwrap_err()),
                code(VerifierError::DuplicateField),
                "{field}"
            );
        }
    }

    #[test]
    fn field_bounds_are_inclusive() {
        let text = String::from_utf8(CLAIMS.to_vec()).unwrap();
        let with_recipient = |length: usize| text.replace("vendor:acme_saas", &"r".repeat(length));
        assert!(parse_claims(with_recipient(MAX_RECIPIENT_LEN).as_bytes()).is_ok());
        assert_eq!(
            code(parse_claims(with_recipient(MAX_RECIPIENT_LEN + 1).as_bytes()).unwrap_err()),
            code(VerifierError::InvalidFieldLength)
        );
        let header = |length: usize| format!(r#"{{"alg":"ES256","kid":"{}"}}"#, "k".repeat(length));
        assert!(parse_header(header(MAX_KID_LEN).as_bytes()).is_ok());
        assert_eq!(
            code(parse_header(header(MAX_KID_LEN + 1).as_bytes()).unwrap_err()),
            code(VerifierError::InvalidFieldLength)
        );
    }

    #[test]
    fn digit_accumulation_is_checked() {
        assert_eq!(parse_digits(b"18446744073709551615").unwrap(), u64::MAX);
        for overflow in [
            &b"18446744073709551616"[..],
            b"99999999999999999999",
            b"100000000000000000000",
        ] {
            assert_eq!(
                code(parse_digits(overflow).unwrap_err()),
                code(VerifierError::InvalidNumber)
            );
        }
        assert!(parse_digits(b"").is_err());
        assert!(parse_digits(b"1 ").is_err());
        assert_eq!(parse_timestamp(b"253402300799").unwrap(), MAX_TIMESTAMP);
        assert!(parse_timestamp(b"999999999999").is_err());
    }

    #[test]
    fn only_json_whitespace_separates_tokens() {
        let text = String::from_utf8(CLAIMS.to_vec()).unwrap();
        let spaced = format!(
            " \t\r\n{}\n",
            text.replace(",\"", " ,\t\"").replace("\":", "\" :\n")
        );
        assert!(parse_claims(spaced.as_bytes()).is_ok());
        for separator in ['\u{0b}', '\u{0c}', '\u{a0}'] {
            let invalid = text.replacen(",\"jti\"", &format!(",{separator}\"jti\""), 1);
            assert!(parse_claims(invalid.as_bytes()).is_err(), "{separator:?}");
        }
        assert_eq!(
            code(parse_claims(b"{}").unwrap_err()),
            code(VerifierError::MissingField)
        );
        assert_eq!(
            code(parse_claims(b"").unwrap_err()),
            code(VerifierError::MalformedJson)
        );
    }

    /// Every single-byte substitution and every truncation of a valid
    /// header and payload returns a result; none panics. Any mutation that
    /// still parses must leave the parse deterministic.
    #[test]
    fn no_single_byte_mutation_or_truncation_panics() {
        let header = br#"{"alg":"ES256","typ":"JWT","kid":"verdict-2026-09"}"#;
        for document in [CLAIMS, &header[..]] {
            for position in 0..document.len() {
                let mut mutated = document.to_vec();
                for byte in 0..=255u8 {
                    mutated[position] = byte;
                    let claims = parse_claims(&mutated).map_err(code);
                    assert_eq!(claims, parse_claims(&mutated).map_err(code));
                    let _ = parse_header(&mutated);
                }
            }
            for length in 0..document.len() {
                assert!(parse_claims(&document[..length]).is_err());
                assert!(parse_header(&document[..length]).is_err());
            }
        }
    }

    #[test]
    fn signing_input_needs_exactly_two_nonempty_segments() {
        for input in [
            &b""[..],
            b".",
            b"e30",
            b"e30.",
            b".e30",
            b"e30..e30",
            b"e30.e30.",
            b"e30.e30.e30",
        ] {
            assert_eq!(
                code(decode_signing_input(input).unwrap_err()),
                code(VerifierError::MalformedSigningInput),
                "{input:?}"
            );
        }
        let decoded = decode_signing_input(b"e30.e30").unwrap();
        assert_eq!(decoded.header_json, b"{}");
        assert_eq!(decoded.payload_json, b"{}");
    }
}
