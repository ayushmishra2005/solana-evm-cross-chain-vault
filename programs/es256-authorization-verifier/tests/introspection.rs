//! Attacks on how `consume_authorization` finds and reads the native
//! secp256r1 verification.
//!
//! Each test states which layer must reject: the native program (failure at
//! the precompile's index) or the verifier (failure at consume's index after
//! the native program accepted).

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::system_instruction;
use common::*;
use es256_authorization_verifier::VerifierError;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_keypair::Keypair;
use solana_precompile_error::PrecompileError;
use solana_signer::Signer;

fn transfer(fixture: &Fixture) -> Instruction {
    system_instruction::transfer(&fixture.consumer.pubkey(), &fixture.outsider.pubkey(), 1)
}

/// An approved verdict the attacker would like the verifier to read.
fn approved(fixture: &Fixture) -> SignedVerdict {
    fixture.sign(&VerdictClaims::default())
}

/// A different verdict the trusted key really signed: same nonce, wrong
/// amount, so it can never be consumed against the approved mandate.
fn unapproved(fixture: &Fixture) -> SignedVerdict {
    fixture.sign(&VerdictClaims {
        amount: "999".into(),
        ..VerdictClaims::default()
    })
}

fn body(verdict: &SignedVerdict) -> Vec<u8> {
    let mut body = verdict.public_key.to_vec();
    body.extend_from_slice(&verdict.signature);
    body.extend_from_slice(&verdict.signing_input);
    body
}

// ---- Program identity -------------------------------------------------------

#[test]
fn ed25519_verification_in_the_precompile_slot_is_not_accepted() {
    let mut fixture = Fixture::new();
    let verdict = approved(&fixture);
    let ed25519_key = Keypair::new();
    let signature: [u8; 64] = ed25519_key
        .sign_message(&verdict.signing_input)
        .as_ref()
        .try_into()
        .unwrap();
    let ed25519 = solana_ed25519_program::new_ed25519_instruction_with_signature(
        &verdict.signing_input,
        &signature,
        &ed25519_key.pubkey().to_bytes(),
    );
    let instructions = [ed25519, fixture.consume_instruction(verdict.nonce_hash())];

    // The ed25519 program verifies its own signature, so the failure is the
    // verifier's.
    assert_verifier_error(
        fixture.send_as_consumer(&instructions),
        1,
        VerifierError::MissingPrecompile,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn lookalike_data_under_another_program_is_not_accepted() {
    let mut fixture = Fixture::new();
    fixture.add_cpi_forwarder();
    let verdict = approved(&fixture);
    let mut lookalike = verdict.precompile();
    lookalike.program_id = FORWARDER_ID;
    let instructions = [lookalike, fixture.consume_instruction(verdict.nonce_hash())];

    assert_verifier_error(
        fixture.send_as_consumer(&instructions),
        1,
        VerifierError::MissingPrecompile,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn substituted_instructions_sysvar_is_rejected_by_account_validation() {
    let mut fixture = Fixture::new();
    let verdict = approved(&fixture);
    let fake = Pubkey::new_unique();
    fixture
        .svm
        .set_account(
            fake,
            solana_account::Account {
                lamports: 1_000_000,
                data: vec![0; 64],
                owner: solana_sdk_ids::sysvar::ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();

    for substitute in [fake, solana_sdk_ids::sysvar::clock::ID] {
        let mut consume = fixture.consume_instruction(verdict.nonce_hash());
        consume.accounts[consume_account::INSTRUCTIONS_SYSVAR].pubkey = substitute;
        assert_anchor_error(
            fixture.send_as_consumer(&[verdict.precompile(), consume]),
            1,
            AnchorError::ConstraintAddress,
        );
    }
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn consume_invoked_by_cpi_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.add_cpi_forwarder();
    let verdict = approved(&fixture);
    let instructions = [
        verdict.precompile(),
        fixture.forwarded_consume_instruction(verdict.nonce_hash()),
    ];

    assert_verifier_error(
        fixture.send_as_consumer(&instructions),
        1,
        VerifierError::NotTopLevel,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

// ---- Instruction order ------------------------------------------------------

#[test]
fn precompile_after_consume_is_not_accepted() {
    let mut fixture = Fixture::new();
    let verdict = approved(&fixture);
    let instructions = [
        fixture.consume_instruction(verdict.nonce_hash()),
        verdict.precompile(),
    ];

    assert_verifier_error(
        fixture.send_as_consumer(&instructions),
        0,
        VerifierError::MissingPrecompile,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn precompile_must_immediately_precede_consume() {
    let mut fixture = Fixture::new();
    let verdict = approved(&fixture);
    let separators = [
        transfer(&fixture),
        ComputeBudgetInstruction::set_compute_unit_limit(200_000),
    ];
    for separator in separators {
        let instructions = [
            verdict.precompile(),
            separator,
            fixture.consume_instruction(verdict.nonce_hash()),
        ];
        assert_verifier_error(
            fixture.send_as_consumer(&instructions),
            2,
            VerifierError::InvalidInstructionOrder,
        );
    }
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn only_compute_budget_instructions_may_precede_the_precompile() {
    let mut fixture = Fixture::new();
    let verdict = approved(&fixture);
    let instructions = [
        ComputeBudgetInstruction::set_compute_unit_limit(200_000),
        transfer(&fixture),
        verdict.precompile(),
        fixture.consume_instruction(verdict.nonce_hash()),
    ];

    assert_verifier_error(
        fixture.send_as_consumer(&instructions),
        3,
        VerifierError::InvalidInstructionOrder,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn compute_budget_prefix_is_accepted() {
    let mut fixture = Fixture::new();
    let verdict = approved(&fixture);
    let instructions = [
        ComputeBudgetInstruction::set_compute_unit_limit(100_000),
        ComputeBudgetInstruction::set_compute_unit_price(1),
        verdict.precompile(),
        fixture.consume_instruction(verdict.nonce_hash()),
    ];

    fixture
        .send_as_consumer(&instructions)
        .expect("compute-budget prefix is allowed");
    assert!(fixture.receipt(&verdict.nonce_hash()).is_some());
}

#[test]
fn a_second_precompile_anywhere_is_rejected() {
    let mut fixture = Fixture::new();
    let verdict = approved(&fixture);
    let other = unapproved(&fixture);

    let before = [
        other.precompile(),
        verdict.precompile(),
        fixture.consume_instruction(verdict.nonce_hash()),
    ];
    assert_verifier_error(
        fixture.send_as_consumer(&before),
        2,
        VerifierError::InvalidInstructionOrder,
    );

    let after = [
        verdict.precompile(),
        fixture.consume_instruction(verdict.nonce_hash()),
        other.precompile(),
    ];
    assert_verifier_error(
        fixture.send_as_consumer(&after),
        1,
        VerifierError::InvalidInstructionOrder,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn two_consumes_in_one_transaction_are_rejected() {
    let mut fixture = Fixture::new();
    let first = approved(&fixture);
    let second = fixture.sign(&VerdictClaims {
        nonce: "n_second".into(),
        ..VerdictClaims::default()
    });

    let same_nonce = [
        first.precompile(),
        fixture.consume_instruction(first.nonce_hash()),
        fixture.consume_instruction(first.nonce_hash()),
    ];
    assert_verifier_error(
        fixture.send_as_consumer(&same_nonce),
        1,
        VerifierError::InvalidInstructionOrder,
    );

    let two_pairs = [
        first.precompile(),
        fixture.consume_instruction(first.nonce_hash()),
        second.precompile(),
        fixture.consume_instruction(second.nonce_hash()),
    ];
    assert_verifier_error(
        fixture.send_as_consumer(&two_pairs),
        1,
        VerifierError::InvalidInstructionOrder,
    );
    fixture.assert_no_receipt(&first.nonce_hash());
    fixture.assert_no_receipt(&second.nonce_hash());
}

#[test]
fn unrelated_instruction_after_consume_is_permitted() {
    let mut fixture = Fixture::new();
    let verdict = approved(&fixture);
    let instructions = [
        verdict.precompile(),
        fixture.consume_instruction(verdict.nonce_hash()),
        transfer(&fixture),
    ];

    fixture
        .send_as_consumer(&instructions)
        .expect("later instructions that are not this program or the precompile are allowed");
    assert!(fixture.receipt(&verdict.nonce_hash()).is_some());
}

// ---- Native layout manipulation ----------------------------------------------

/// Layouts the native program rejects before the verifier runs.
#[test]
fn malformed_layouts_fail_natively() {
    let mut fixture = Fixture::new();
    let verdict = approved(&fixture);
    let len = verdict.signing_input.len();
    let canonical = Offsets::canonical(len);
    let full = body(&verdict);

    let cases: Vec<(&str, Instruction, PrecompileError)> = vec![
        (
            "zero signatures",
            raw_precompile(0, 0, &[canonical], &full),
            PrecompileError::InvalidInstructionDataSize,
        ),
        (
            "nine signatures",
            raw_precompile(9, 0, &[canonical], &full),
            PrecompileError::InvalidInstructionDataSize,
        ),
        (
            "truncated offsets",
            raw_precompile(1, 0, &[], &[0; 10]),
            PrecompileError::InvalidInstructionDataSize,
        ),
        (
            "message longer than the data",
            raw_precompile(
                1,
                0,
                &[Offsets {
                    message_len: u16::try_from(len + 1).unwrap(),
                    ..canonical
                }],
                &full,
            ),
            PrecompileError::InvalidDataOffsets,
        ),
        (
            "signature offset past the data",
            raw_precompile(
                1,
                0,
                &[Offsets {
                    signature_offset: u16::MAX - 1,
                    ..canonical
                }],
                &full,
            ),
            PrecompileError::InvalidDataOffsets,
        ),
        (
            "public key offset past the data",
            raw_precompile(
                1,
                0,
                &[Offsets {
                    public_key_offset: u16::MAX - 1,
                    ..canonical
                }],
                &full,
            ),
            PrecompileError::InvalidDataOffsets,
        ),
        (
            "reference to a missing instruction",
            raw_precompile(
                1,
                0,
                &[Offsets {
                    message_instruction: 7,
                    ..canonical
                }],
                &full,
            ),
            PrecompileError::InvalidDataOffsets,
        ),
        (
            "truncated signing input",
            raw_precompile(1, 0, &[canonical], &full[..full.len() - 1]),
            PrecompileError::InvalidDataOffsets,
        ),
        (
            "truncated public key",
            raw_precompile(1, 0, &[canonical], &full[..20]),
            PrecompileError::InvalidDataOffsets,
        ),
        (
            "signature read from inside the key",
            raw_precompile(
                1,
                0,
                &[Offsets {
                    signature_offset: 17,
                    ..canonical
                }],
                &full,
            ),
            PrecompileError::InvalidSignature,
        ),
    ];
    for (name, precompile, expected) in cases {
        let instructions = [
            precompile,
            fixture.consume_instruction(verdict.nonce_hash()),
        ];
        let result = fixture.send_as_consumer(&instructions);
        let (index, error) = instruction_error(result);
        assert_eq!(
            (index, error),
            (
                0,
                solana_instruction::error::InstructionError::Custom(expected as u32)
            ),
            "{name}"
        );
    }
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

/// Layouts the native program accepts, because they verify a real
/// signature, but which do not pin the bytes the verifier reads.
#[test]
fn natively_valid_noncanonical_layouts_are_rejected_by_the_verifier() {
    let mut fixture = Fixture::new();
    fixture.add_cpi_forwarder();
    let verdict = approved(&fixture);
    let signed = unapproved(&fixture);
    let len = verdict.signing_input.len();
    let canonical = Offsets::canonical(len);
    let full = body(&verdict);

    let self_by_index = Offsets {
        signature_instruction: 0,
        public_key_instruction: 0,
        message_instruction: 0,
        ..canonical
    };

    let shifted = |offset: u16| Offsets {
        signature_offset: 49 + offset,
        public_key_offset: 16 + offset,
        message_offset: 113 + offset,
        ..canonical
    };

    let mut swapped_body = verdict.signature.to_vec();
    swapped_body.extend_from_slice(&verdict.public_key);
    swapped_body.extend_from_slice(&verdict.signing_input);
    let swapped = Offsets {
        signature_offset: 16,
        public_key_offset: 80,
        ..canonical
    };

    let cases: Vec<(&str, Instruction, Vec<Instruction>, VerifierError)> = vec![
        (
            "nonzero padding byte",
            raw_precompile(1, 1, &[canonical], &full),
            Vec::new(),
            VerifierError::InvalidPrecompileInstruction,
        ),
        (
            "absolute self-reference instead of u16::MAX",
            raw_precompile(1, 0, &[self_by_index], &full),
            Vec::new(),
            VerifierError::InvalidPrecompileInstruction,
        ),
        (
            "two entries verifying the same signature",
            raw_precompile(2, 0, &[shifted(14), shifted(14)], &full),
            Vec::new(),
            VerifierError::InvalidPrecompileInstruction,
        ),
        (
            "key and signature swapped",
            raw_precompile(1, 0, &[swapped], &swapped_body),
            Vec::new(),
            VerifierError::InvalidPrecompileInstruction,
        ),
        (
            "verified message hidden after the claims the verifier would read",
            {
                // Native verifies a short message the key really signed;
                // offset 113 holds the approved claims, which no one signed.
                let hidden = fixture.sign_input(b"e30.e30".to_vec());
                let mut body = hidden.public_key.to_vec();
                body.extend_from_slice(&hidden.signature);
                body.extend_from_slice(&verdict.signing_input);
                body.extend_from_slice(&hidden.signing_input);
                let offsets = Offsets {
                    message_offset: u16::try_from(113 + len).unwrap(),
                    message_len: u16::try_from(hidden.signing_input.len()).unwrap(),
                    ..canonical
                };
                raw_precompile(1, 0, &[offsets], &body)
            },
            Vec::new(),
            VerifierError::InvalidPrecompileInstruction,
        ),
        (
            "verified message taken from a later instruction",
            {
                let mut body = signed.public_key.to_vec();
                body.extend_from_slice(&signed.signature);
                body.extend_from_slice(&verdict.signing_input);
                let offsets = Offsets {
                    message_offset: 0,
                    message_len: u16::try_from(signed.signing_input.len()).unwrap(),
                    message_instruction: 2,
                    ..canonical
                };
                raw_precompile(1, 0, &[offsets], &body)
            },
            vec![Instruction {
                program_id: FORWARDER_ID,
                accounts: Vec::new(),
                data: signed.signing_input.clone(),
            }],
            VerifierError::InvalidPrecompileInstruction,
        ),
        (
            "message overlapping the public key",
            {
                let signature_over_key = fixture.sign_input(verdict.public_key.to_vec()).signature;
                let mut body = verdict.public_key.to_vec();
                body.extend_from_slice(&signature_over_key);
                let offsets = Offsets {
                    message_offset: 16,
                    message_len: 33,
                    ..canonical
                };
                raw_precompile(1, 0, &[offsets], &body)
            },
            Vec::new(),
            VerifierError::InvalidSigningInputLength,
        ),
        (
            "empty message",
            {
                let empty = fixture.sign_input(Vec::new());
                let mut body = empty.public_key.to_vec();
                body.extend_from_slice(&empty.signature);
                raw_precompile(1, 0, &[Offsets::canonical(0)], &body)
            },
            Vec::new(),
            VerifierError::InvalidSigningInputLength,
        ),
        (
            "message one byte over the limit",
            {
                let oversized = fixture.sign_input(vec![b'A'; 513]);
                raw_precompile(1, 0, &[Offsets::canonical(513)], &body(&oversized))
            },
            Vec::new(),
            VerifierError::InvalidSigningInputLength,
        ),
        (
            "trailing byte after the message",
            {
                let mut data = full.clone();
                data.push(b'x');
                raw_precompile(1, 0, &[canonical], &data)
            },
            Vec::new(),
            VerifierError::InvalidPrecompileInstruction,
        ),
        (
            "account meta on the precompile",
            {
                let mut precompile = verdict.precompile();
                precompile
                    .accounts
                    .push(AccountMeta::new_readonly(fixture.outsider.pubkey(), false));
                precompile
            },
            Vec::new(),
            VerifierError::InvalidPrecompileInstruction,
        ),
    ];
    for (name, precompile, trailing, expected) in cases {
        let mut instructions = vec![
            precompile,
            fixture.consume_instruction(verdict.nonce_hash()),
        ];
        instructions.extend(trailing);
        let (index, error) = instruction_error(fixture.send_as_consumer(&instructions));
        assert_eq!(
            (index, error),
            (
                1,
                solana_instruction::error::InstructionError::Custom(u32::from(expected))
            ),
            "{name}: the native program must accept and the verifier must reject"
        );
    }
    fixture.assert_no_receipt(&verdict.nonce_hash());
    fixture
        .consume(&verdict)
        .expect("the canonical layout of the same verdict still works");
}
