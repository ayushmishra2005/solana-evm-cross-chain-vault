//! Mutation sweeps run through the deployed program: every outcome must be
//! success or a typed error, never an abort, and never above budget.
//!
//! Each mutated token is re-signed by the trusted key so it passes native
//! verification and reaches the parser.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use es256_authorization_verifier::VerifierError;
use solana_instruction::error::InstructionError;

/// Asserts the outcome is a success, a verifier error or a replay of an
/// already-consumed nonce, and returns whether it succeeded.
fn assert_controlled(result: TxResult, what: &str) -> bool {
    let first = u32::from(VerifierError::Unauthorized);
    let last = u32::from(VerifierError::NonceHashMismatch);
    match result {
        Ok(meta) => {
            assert!(meta.compute_units_consumed <= 200_000, "{what}");
            true
        }
        Err(failure) => {
            assert!(failure.meta.compute_units_consumed <= 200_000, "{what}");
            match failure.err {
                solana_transaction_error::TransactionError::InstructionError(
                    1,
                    InstructionError::Custom(code),
                ) if (first..=last).contains(&code) || code == ACCOUNT_ALREADY_IN_USE => false,
                other => panic!("{what}: uncontrolled failure {other:?}"),
            }
        }
    }
}

#[test]
fn every_payload_byte_mutation_fails_in_a_controlled_way() {
    let mut fixture = Fixture::new();
    let claims = VerdictClaims::default();
    let header = claims.header_json();
    let payload = claims.payload_json().into_bytes();
    let mut accepted = 0;
    for position in 0..payload.len() {
        for byte in [b'"', b'\\', b'}', b',', 0x00, 0xFF] {
            let mut mutated = payload.clone();
            if mutated[position] == byte {
                continue;
            }
            mutated[position] = byte;
            let verdict = fixture.sign_input(es256_jws_adapter::signing_input(
                header.as_bytes(),
                &mutated,
            ));
            let what = format!("payload byte {position} = {byte:#04x}");
            if assert_controlled(fixture.consume(&verdict), &what) {
                accepted += 1;
            }
        }
    }
    // Only mutations inside free-text values that are not checked against
    // the mandate (`jti`) could still be accepted; the nonce allows one.
    assert!(accepted <= 1, "{accepted} mutated verdicts were accepted");
}

#[test]
fn every_signing_input_truncation_fails_in_a_controlled_way() {
    let mut fixture = Fixture::new();
    let input = VerdictClaims::default().signing_input();
    for length in 1..input.len() {
        let verdict = fixture.sign_input(input[..length].to_vec());
        assert!(
            !assert_controlled(fixture.consume(&verdict), &format!("length {length}")),
            "a truncated verdict was accepted at {length}"
        );
    }
    fixture.assert_no_receipt(&es256_authorization_verifier::nonce_hash(NONCE.as_bytes()));
}

#[test]
fn every_signing_input_byte_substitution_fails_in_a_controlled_way() {
    let mut fixture = Fixture::new();
    let input = VerdictClaims::default().signing_input();
    for position in 0..input.len() {
        for byte in [b'.', b'=', b'+', 0x00, 0x80] {
            if input[position] == byte {
                continue;
            }
            let mut mutated = input.clone();
            mutated[position] = byte;
            let verdict = fixture.sign_input(mutated);
            assert!(
                !assert_controlled(
                    fixture.consume(&verdict),
                    &format!("input byte {position} = {byte:#04x}")
                ),
                "an encoding-level mutation was accepted at {position}"
            );
        }
    }
}
