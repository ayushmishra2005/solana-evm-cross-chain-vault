//! Mutation sweeps run through the deployed program. Each mutated signing
//! input is re-signed by the trusted key, so it passes native verification,
//! and runs against a fresh copy of the configured ledger, so an earlier
//! accepted mutation's receipt can never stand in for a later rejection.
//!
//! The program's own parser, run on the host, predicts every outcome: input
//! it rejects must fail on-chain with exactly that error, and input it
//! accepts must either be consumed or fail a post-parse authorization check.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use es256_authorization_verifier::jwt::decode_signing_input;
use es256_authorization_verifier::{JTI_DOMAIN, VerifierError, domain_hash, nonce_hash};
use litesvm::LiteSVM;
use solana_instruction::error::InstructionError;

const COMPUTE_BUDGET: u64 = 200_000;

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Accepted,
    ParserRejected(u32),
    AuthorizationRejected(u32),
}

/// What the on-chain parser does with `signing_input`: the error it raises,
/// or the accepted `jti`.
fn host_parse(signing_input: &[u8]) -> Result<Vec<u8>, u32> {
    let decoded = decode_signing_input(signing_input).map_err(u32::from)?;
    decoded.header().map_err(u32::from)?;
    Ok(decoded.claims().map_err(u32::from)?.jti.to_vec())
}

/// Errors raised after parsing, by the key, time, mandate and nonce checks.
fn is_authorization_error(code: u32) -> bool {
    (u32::from(VerifierError::KeyIdMismatch)..=u32::from(VerifierError::NonceHashMismatch))
        .contains(&code)
}

/// A configured fixture plus a snapshot of its ledger before any consume.
struct Sweep {
    fixture: Fixture,
    pristine: LiteSVM,
}

impl Sweep {
    fn new() -> Self {
        let fixture = Fixture::new();
        let pristine = fixture.svm.clone();
        Self { fixture, pristine }
    }

    /// Signs and consumes `signing_input` on a fresh copy of the ledger and
    /// checks the result against the host parser's prediction.
    fn run(&mut self, signing_input: Vec<u8>, what: &str) -> Outcome {
        self.fixture.svm = self.pristine.clone();
        let verdict = self.fixture.sign_input(signing_input);
        let predicted = host_parse(&verdict.signing_input);
        let result = self.fixture.consume(&verdict);
        let units = match &result {
            Ok(meta) => meta.compute_units_consumed,
            Err(failure) => failure.meta.compute_units_consumed,
        };
        assert!(units <= COMPUTE_BUDGET, "{what}: {units} CU");

        match (predicted, result) {
            (Err(expected), result) => {
                assert_eq!(
                    instruction_error(result),
                    (1, InstructionError::Custom(expected)),
                    "{what}: expected parser error {expected}"
                );
                self.fixture.assert_no_receipt(&verdict.nonce_hash());
                Outcome::ParserRejected(expected)
            }
            (Ok(jti), Ok(_)) => {
                let receipt = self
                    .fixture
                    .receipt(&verdict.nonce_hash())
                    .unwrap_or_else(|| panic!("{what}: accepted without a receipt"));
                assert_eq!(
                    receipt.signing_input_hash,
                    solana_sha256_hasher::hash(&verdict.signing_input).to_bytes(),
                    "{what}"
                );
                assert_eq!(receipt.jti_hash, domain_hash(JTI_DOMAIN, &jti), "{what}");
                Outcome::Accepted
            }
            (Ok(_), result) => {
                let (index, error) = instruction_error(result);
                let InstructionError::Custom(code) = error else {
                    panic!("{what}: parsed input failed with ({index}, {error:?})");
                };
                assert!(
                    index == 1 && is_authorization_error(code),
                    "{what}: parsed input failed with ({index}, Custom({code}))"
                );
                self.fixture.assert_no_receipt(&verdict.nonce_hash());
                Outcome::AuthorizationRejected(code)
            }
        }
    }
}

/// The byte range of the string value of `field` inside `payload`.
fn string_value_range(payload: &[u8], field: &str) -> std::ops::Range<usize> {
    let key = format!(r#""{field}":""#).into_bytes();
    let start = payload
        .windows(key.len())
        .position(|window| window == key)
        .unwrap()
        + key.len();
    let len = payload[start..].iter().position(|&b| b == b'"').unwrap();
    start..start + len
}

#[test]
fn every_payload_byte_mutation_fails_in_a_controlled_way() {
    let mut sweep = Sweep::new();
    let claims = VerdictClaims::default();
    let header = claims.header_json();
    let payload = claims.payload_json().into_bytes();
    let jti = string_value_range(&payload, "jti");
    let (mut accepted, mut parser, mut authorization) = (0, 0, 0);
    for position in 0..payload.len() {
        for byte in [b'"', b'\\', b'}', b',', 0x00, 0xFF] {
            let mut mutated = payload.clone();
            if mutated[position] == byte {
                continue;
            }
            mutated[position] = byte;
            let input = es256_jws_adapter::signing_input(header.as_bytes(), &mutated);
            let what = format!("payload byte {position} = {byte:#04x}");
            match sweep.run(input, &what) {
                Outcome::Accepted => {
                    // Every other string value is matched against the
                    // mandate or the profile; `jti` is only recorded.
                    assert!(jti.contains(&position), "{what} was accepted");
                    accepted += 1;
                }
                Outcome::ParserRejected(_) => parser += 1,
                Outcome::AuthorizationRejected(_) => authorization += 1,
            }
        }
    }
    assert!(
        accepted > 0 && parser > 0 && authorization > 0,
        "accepted {accepted}, parser {parser}, authorization {authorization}"
    );
}

#[test]
fn every_signing_input_truncation_fails_in_a_controlled_way() {
    let mut sweep = Sweep::new();
    let input = VerdictClaims::default().signing_input();
    for length in 1..input.len() {
        let what = format!("length {length}");
        let outcome = sweep.run(input[..length].to_vec(), &what);
        assert!(
            matches!(outcome, Outcome::ParserRejected(_)),
            "{what}: {outcome:?}"
        );
    }
}

#[test]
fn every_signing_input_byte_substitution_fails_in_a_controlled_way() {
    let mut sweep = Sweep::new();
    let input = VerdictClaims::default().signing_input();
    for position in 0..input.len() {
        for byte in [b'.', b'=', b'+', 0x00, 0x80] {
            if input[position] == byte {
                continue;
            }
            let mut mutated = input.clone();
            mutated[position] = byte;
            let what = format!("input byte {position} = {byte:#04x}");
            let outcome = sweep.run(mutated, &what);
            assert!(
                matches!(outcome, Outcome::ParserRejected(_)),
                "{what}: {outcome:?}"
            );
        }
    }
}

/// Regression: the sweeps once shared one ledger and one nonce, so the
/// first accepted `jti` mutation left a receipt and every later mutation
/// failed at receipt `init` (`AccountAlreadyInUse`) before the parser ran.
#[test]
fn an_accepted_mutation_cannot_mask_a_later_parser_rejection() {
    let mut sweep = Sweep::new();
    let claims = VerdictClaims::default();
    let header = claims.header_json();
    let payload = claims.payload_json().into_bytes();
    let receipt = nonce_hash(NONCE.as_bytes());

    let mut valid = payload.clone();
    valid[string_value_range(&payload, "jti").start] = b',';
    let valid = es256_jws_adapter::signing_input(header.as_bytes(), &valid);
    assert_eq!(sweep.run(valid, "jti mutation"), Outcome::Accepted);
    assert!(sweep.fixture.receipt(&receipt).is_some());

    // A backslash in `category`, a field after `jti`.
    let mut malformed = payload.clone();
    malformed[string_value_range(&payload, "category").start] = b'\\';
    let malformed = es256_jws_adapter::signing_input(header.as_bytes(), &malformed);

    // On the consumed ledger the receipt `init` fails first.
    let verdict = sweep.fixture.sign_input(malformed.clone());
    assert_eq!(
        instruction_error(sweep.fixture.consume(&verdict)),
        (1, InstructionError::Custom(ACCOUNT_ALREADY_IN_USE))
    );

    // On a fresh ledger the parser rejects it, and nothing is recorded.
    let malformed_json = u32::from(VerifierError::MalformedJson);
    assert_eq!(host_parse(&malformed), Err(malformed_json));
    assert_eq!(
        sweep.run(malformed, "category mutation"),
        Outcome::ParserRejected(malformed_json)
    );
    sweep.fixture.assert_no_receipt(&receipt);
}
