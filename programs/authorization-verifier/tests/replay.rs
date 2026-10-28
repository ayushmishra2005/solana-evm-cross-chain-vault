//! Replay prevention, atomic rollback and receipt integrity.
//!
//! LiteSVM executes transactions one at a time. The competing-submission
//! test below shows that the second of two transactions prepared
//! concurrently fails on the existing receipt; it does not exercise
//! parallel scheduling. On a validator both would write-lock the same
//! receipt address, so the runtime serializes them the same way.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::solana_program::system_instruction;
use common::*;
use es256_authorization_verifier::{Environment, VerifierError};
use es256_jws_adapter::SyntheticSigner;
use solana_instruction::error::InstructionError;
use solana_signer::Signer;
use solana_transaction::Transaction;

fn assert_replay_rejected(fixture: &mut Fixture, verdict: &SignedVerdict) {
    let address = fixture.receipt_address(&verdict.nonce_hash());
    let before = fixture.svm.get_account(&address).expect("receipt exists");
    assert_eq!(
        instruction_error(fixture.consume(verdict)),
        (1, InstructionError::Custom(ACCOUNT_ALREADY_IN_USE)),
        "the receipt `init` must be what stops the replay"
    );
    assert_eq!(
        fixture.svm.get_account(&address),
        Some(before),
        "the original receipt is unchanged"
    );
}

#[test]
fn same_nonce_in_a_different_signed_payload_is_a_replay() {
    let mut fixture = Fixture::new();
    let first = fixture.sign(&VerdictClaims::default());
    fixture.consume(&first).unwrap();

    let variants = [
        VerdictClaims {
            jti: "dec_other".into(),
            ..VerdictClaims::default()
        },
        VerdictClaims::issued_at(NOW + 30),
        VerdictClaims {
            amount: "340.000000".into(),
            ..VerdictClaims::default()
        },
    ];
    for claims in variants {
        let verdict = fixture.sign(&claims);
        assert_ne!(verdict.signing_input, first.signing_input);
        assert_ne!(verdict.signature, first.signature);
        assert_eq!(verdict.nonce_hash(), first.nonce_hash());
        assert_replay_rejected(&mut fixture, &verdict);
    }
    let receipt = fixture.receipt(&first.nonce_hash()).unwrap();
    assert_eq!(
        receipt.signing_input_hash,
        solana_sha256_hasher::hash(&first.signing_input).to_bytes(),
        "the receipt still records the first authorization"
    );
}

#[test]
fn same_nonce_under_another_trusted_key_is_a_replay() {
    let mut fixture = Fixture::new();
    fixture
        .consume(&fixture.sign(&VerdictClaims::default()))
        .unwrap();

    let second_signer = SyntheticSigner::generate().unwrap();
    fixture.trusted_key = fixture
        .register_key("verdict-test-02", second_signer.public_key().unwrap())
        .unwrap();
    let verdict = SignedVerdict::sign(
        &second_signer,
        &VerdictClaims {
            kid: "verdict-test-02".into(),
            jti: "dec_second_key".into(),
            ..VerdictClaims::default()
        },
    );
    assert_replay_rejected(&mut fixture, &verdict);
}

#[test]
fn same_nonce_after_key_rotation_is_a_replay() {
    let mut fixture = Fixture::new();
    let first = fixture.sign(&VerdictClaims::default());
    fixture.consume(&first).unwrap();

    let old_key = fixture.trusted_key;
    fixture.signer = SyntheticSigner::generate().unwrap();
    fixture.trusted_key = fixture
        .register_key("verdict-test-02", fixture.signer.public_key().unwrap())
        .unwrap();
    fixture.revoke_key(old_key).unwrap();

    let rotated = fixture.sign(&VerdictClaims {
        kid: "verdict-test-02".into(),
        jti: "dec_after_rotation".into(),
        ..VerdictClaims::default()
    });
    assert_replay_rejected(&mut fixture, &rotated);

    let fresh = fixture.sign(&VerdictClaims {
        kid: "verdict-test-02".into(),
        nonce: "n_after_rotation".into(),
        ..VerdictClaims::default()
    });
    fixture
        .consume(&fresh)
        .expect("a fresh nonce under the new key is accepted");
}

#[test]
fn competing_submissions_of_one_nonce_commit_once() {
    let mut fixture = Fixture::new();
    let consumer = fixture.consumer.insecure_clone();
    let blockhash = fixture.svm.latest_blockhash();
    let build = |fixture: &Fixture, verdict: &SignedVerdict| {
        Transaction::new_signed_with_payer(
            &[
                verdict.precompile(),
                fixture.consume_instruction(verdict.nonce_hash()),
            ],
            Some(&consumer.pubkey()),
            &[&consumer],
            blockhash,
        )
    };
    let a = fixture.sign(&VerdictClaims::default());
    let b = fixture.sign(&VerdictClaims {
        jti: "dec_competing".into(),
        ..VerdictClaims::default()
    });
    let first = build(&fixture, &a);
    let second = build(&fixture, &b);
    assert_ne!(first.signatures, second.signatures);

    fixture.svm.send_transaction(first).expect("one commits");
    let failure = fixture.svm.send_transaction(second).unwrap_err();
    assert_eq!(
        failure.err,
        solana_transaction_error::TransactionError::InstructionError(
            1,
            InstructionError::Custom(ACCOUNT_ALREADY_IN_USE)
        )
    );
    let receipt = fixture.receipt(&a.nonce_hash()).unwrap();
    assert_eq!(
        receipt.signing_input_hash,
        solana_sha256_hasher::hash(&a.signing_input).to_bytes()
    );
}

#[test]
fn representative_rejections_never_burn_the_nonce() {
    let mut fixture = Fixture::new();
    let valid = fixture.sign(&VerdictClaims::default());
    type Attempt = Box<dyn Fn(&Fixture) -> Vec<Instruction>>;
    let attempts: Vec<(&str, Attempt)> = vec![
        (
            "native signature failure",
            Box::new(|fixture: &Fixture| {
                let mut verdict = fixture.sign(&VerdictClaims::default());
                verdict.signature[5] ^= 1;
                vec![
                    verdict.precompile(),
                    fixture.consume_instruction(verdict.nonce_hash()),
                ]
            }),
        ),
        (
            "missing precompile",
            Box::new(|fixture: &Fixture| {
                vec![
                    fixture
                        .consume_instruction(fixture.sign(&VerdictClaims::default()).nonce_hash()),
                ]
            }),
        ),
        (
            "parser rejection",
            Box::new(|fixture: &Fixture| {
                let verdict = fixture.sign(&VerdictClaims {
                    amount: "3.4e2".into(),
                    ..VerdictClaims::default()
                });
                vec![
                    verdict.precompile(),
                    fixture.consume_instruction(verdict.nonce_hash()),
                ]
            }),
        ),
        (
            "deny verdict",
            Box::new(|fixture: &Fixture| {
                let verdict = fixture.sign(&VerdictClaims {
                    verdict: "deny".into(),
                    ..VerdictClaims::default()
                });
                vec![
                    verdict.precompile(),
                    fixture.consume_instruction(verdict.nonce_hash()),
                ]
            }),
        ),
        (
            "expired verdict",
            Box::new(|fixture: &Fixture| {
                let verdict = fixture.sign(&VerdictClaims::issued_at(NOW - 300));
                vec![
                    verdict.precompile(),
                    fixture.consume_instruction(verdict.nonce_hash()),
                ]
            }),
        ),
        (
            "mandate mismatch",
            Box::new(|fixture: &Fixture| {
                let verdict = fixture.sign(&VerdictClaims {
                    recipient: "vendor:attacker".into(),
                    ..VerdictClaims::default()
                });
                vec![
                    verdict.precompile(),
                    fixture.consume_instruction(verdict.nonce_hash()),
                ]
            }),
        ),
        (
            "untrusted key",
            Box::new(|fixture: &Fixture| {
                let attacker = SyntheticSigner::generate().unwrap();
                let verdict = SignedVerdict::sign(&attacker, &VerdictClaims::default());
                vec![
                    verdict.precompile(),
                    fixture.consume_instruction(verdict.nonce_hash()),
                ]
            }),
        ),
    ];
    for (name, build) in attempts {
        let instructions = build(&fixture);
        let result = fixture.send_as_consumer(&instructions);
        assert!(result.is_err(), "{name} must fail");
        fixture.assert_no_receipt(&valid.nonce_hash());
    }
    fixture
        .consume(&valid)
        .expect("the nonce is still unused after every rejection");
}

#[test]
fn failure_after_a_successful_consume_rolls_back_the_receipt() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let consumer = fixture.consumer.pubkey();
    let recipient = fixture.outsider.pubkey();
    let consumer_before = fixture.balance(&consumer);
    let recipient_before = fixture.balance(&recipient);

    let instructions = [
        verdict.precompile(),
        fixture.consume_instruction(verdict.nonce_hash()),
        system_instruction::transfer(&consumer, &recipient, 1_000),
        system_instruction::transfer(&consumer, &recipient, consumer_before * 2),
    ];
    let failure = fixture.send_as_consumer(&instructions).unwrap_err();
    assert_eq!(
        failure.err,
        solana_transaction_error::TransactionError::InstructionError(
            3,
            InstructionError::Custom(RESULT_WITH_NEGATIVE_LAMPORTS)
        )
    );

    fixture.assert_no_receipt(&verdict.nonce_hash());
    assert_eq!(
        fixture.balance(&recipient),
        recipient_before,
        "the earlier transfer rolled back"
    );
    // The fee covers the payer's and the precompile's signatures; the
    // receipt rent and both transfers are returned.
    assert_eq!(failure.meta.fee, 10_000);
    assert_eq!(
        consumer_before - fixture.balance(&consumer),
        failure.meta.fee
    );

    fixture
        .consume(&verdict)
        .expect("the rolled-back nonce is still unused");
}

#[test]
fn receipt_must_be_the_pda_of_the_signed_nonce_under_this_config() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let other_nonce = es256_authorization_verifier::nonce_hash(b"n_other");
    let wrong_receipts = [
        fixture.receipt_address(&other_nonce),
        receipt_address(config_address(Environment::Sandbox), &verdict.nonce_hash()),
        Pubkey::find_program_address(
            &[b"consumed", verdict.nonce_hash().as_ref()],
            &es256_authorization_verifier::ID,
        )
        .0,
        Pubkey::new_unique(),
    ];
    for receipt in wrong_receipts {
        let mut consume = fixture.consume_instruction(verdict.nonce_hash());
        consume.accounts[consume_account::RECEIPT].pubkey = receipt;
        assert_anchor_error(
            fixture.send_as_consumer(&[verdict.precompile(), consume]),
            1,
            AnchorError::ConstraintSeeds,
        );
    }

    // Naming the other nonce's receipt with a matching argument reaches the
    // handler, which compares it with the signed nonce.
    assert_verifier_error(
        fixture.send_as_consumer(&[
            verdict.precompile(),
            fixture.consume_instruction(other_nonce),
        ]),
        1,
        VerifierError::NonceHashMismatch,
    );
    fixture.assert_no_receipt(&other_nonce);
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn prefunded_receipt_address_does_not_block_consumption() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let address = fixture.receipt_address(&verdict.nonce_hash());
    // Anyone can transfer lamports to an address before it is used.
    let outsider = fixture.outsider.insecure_clone();
    let lamports = fixture.svm.minimum_balance_for_rent_exemption(0);
    fixture
        .send(
            &[system_instruction::transfer(
                &outsider.pubkey(),
                &address,
                lamports,
            )],
            &[&outsider],
        )
        .unwrap();

    fixture
        .consume(&verdict)
        .expect("Anchor tops up and allocates a prefunded address");
    let receipt = fixture.receipt(&verdict.nonce_hash()).unwrap();
    assert_eq!(receipt.nonce_hash, verdict.nonce_hash());
    assert_eq!(
        fixture.svm.get_account(&address).unwrap().owner,
        es256_authorization_verifier::ID
    );
}

/// States the program could never produce at its own PDA, injected to show
/// that `init` refuses to adopt an existing account rather than overwrite it.
#[test]
fn existing_account_at_the_receipt_address_is_never_overwritten() {
    let owners = [
        solana_sdk_ids::system_program::ID,
        es256_authorization_verifier::ID,
    ];
    for owner in owners {
        let mut fixture = Fixture::new();
        let verdict = fixture.sign(&VerdictClaims::default());
        let address = fixture.receipt_address(&verdict.nonce_hash());
        let squatter = solana_account::Account {
            lamports: 10_000_000,
            data: vec![0xAB; 16],
            owner,
            executable: false,
            rent_epoch: 0,
        };
        fixture.svm.set_account(address, squatter.clone()).unwrap();

        assert_eq!(
            instruction_error(fixture.consume(&verdict)),
            (1, InstructionError::Custom(ACCOUNT_ALREADY_IN_USE)),
            "owner {owner}"
        );
        let after = fixture.svm.get_account(&address).unwrap();
        assert_eq!(after.data, squatter.data, "data untouched");
        assert_eq!(after.owner, squatter.owner, "owner untouched");
        assert_eq!(after.lamports, squatter.lamports, "lamports untouched");
    }
}
