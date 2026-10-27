//! `consume_authorization` executed on LiteSVM beside the real native
//! secp256r1 precompile.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use anchor_lang::solana_program::instruction::Instruction;
use common::*;
use es256_authorization_verifier::{Environment, JTI_DOMAIN, VerifierError, domain_hash};
use es256_jws_adapter::{SyntheticSigner, high_s_twin, is_low_s, secp256r1_instruction};
use solana_instruction::error::InstructionError;
use solana_precompile_error::PrecompileError;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

#[test]
fn valid_allow_verdict_creates_one_receipt() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());

    let meta = fixture
        .consume(&verdict)
        .expect("valid verdict is consumed");

    let receipt = fixture
        .receipt(&verdict.nonce_hash())
        .expect("receipt exists");
    assert_eq!(receipt.config, fixture.config);
    assert_eq!(receipt.nonce_hash, verdict.nonce_hash());
    assert_eq!(
        receipt.signing_input_hash,
        solana_sha256_hasher::hash(&verdict.signing_input).to_bytes()
    );
    assert_eq!(receipt.jti_hash, domain_hash(JTI_DOMAIN, b"dec_5b2e"));
    assert_eq!(receipt.mandate, fixture.mandate);
    assert_eq!(receipt.consumed_at, NOW);
    assert_eq!(receipt.expires_at, NOW + 300);
    assert!(meta.compute_units_consumed > 0);
}

#[test]
fn consumed_nonce_cannot_be_consumed_again() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    fixture.consume(&verdict).expect("first consume succeeds");
    let receipt = fixture
        .svm
        .get_account(&fixture.receipt_address(&verdict.nonce_hash()));

    // A fresh blockhash makes this a new transaction, so only the receipt
    // account can stop it.
    // The receipt `init` fails in the system program: AccountAlreadyInUse.
    assert_eq!(
        instruction_error(fixture.consume(&verdict)),
        (1, InstructionError::Custom(0)),
        "the consume instruction itself fails"
    );
    assert_eq!(
        fixture
            .svm
            .get_account(&fixture.receipt_address(&verdict.nonce_hash())),
        receipt,
        "the original receipt is untouched"
    );
}

#[test]
fn signed_context_must_match_the_approved_mandate() {
    type Mutation = fn(&mut VerdictClaims);
    let cases: [(Mutation, VerifierError); 7] = [
        (|c| c.amount = "341".into(), VerifierError::AmountMismatch),
        (
            |c| c.currency = "EUR".into(),
            VerifierError::CurrencyMismatch,
        ),
        (
            |c| c.recipient = "vendor:other".into(),
            VerifierError::RecipientMismatch,
        ),
        (
            |c| c.mandate = "mnd_other".into(),
            VerifierError::MandateMismatch,
        ),
        (
            |c| c.policy_version = "pol_other".into(),
            VerifierError::PolicyMismatch,
        ),
        (
            |c| c.agent = "agt_other".into(),
            VerifierError::AgentMismatch,
        ),
        (
            |c| c.category = "travel".into(),
            VerifierError::CategoryMismatch,
        ),
    ];
    let mut fixture = Fixture::new();
    for (mutate, expected) in cases {
        let mut claims = VerdictClaims::default();
        mutate(&mut claims);
        let verdict = fixture.sign(&claims);
        assert_verifier_error(fixture.consume(&verdict), 1, expected);
        assert!(fixture.receipt(&verdict.nonce_hash()).is_none());
    }
}

#[test]
fn equal_decimal_amount_matches_the_approved_amount() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims {
        amount: "340.000000".into(),
        ..VerdictClaims::default()
    });
    fixture
        .consume(&verdict)
        .expect("same value, different spelling");
}

#[test]
fn consume_without_a_precompile_fails() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let consumer = fixture.consumer.insecure_clone();

    let alone = [fixture.consume_instruction(verdict.nonce_hash())];
    assert_verifier_error(
        fixture.send(&alone, &[&consumer]),
        0,
        VerifierError::MissingPrecompile,
    );

    let transfer = anchor_lang::solana_program::system_instruction::transfer(
        &consumer.pubkey(),
        &fixture.outsider.pubkey(),
        1,
    );
    let preceded_by_transfer = [transfer, fixture.consume_instruction(verdict.nonce_hash())];
    assert_verifier_error(
        fixture.send(&preceded_by_transfer, &[&consumer]),
        1,
        VerifierError::MissingPrecompile,
    );
    assert!(fixture.receipt(&verdict.nonce_hash()).is_none());
}

#[test]
fn corrupted_signature_is_rejected_by_the_native_program() {
    let mut fixture = Fixture::new();
    let mut verdict = fixture.sign(&VerdictClaims::default());
    verdict.signature[10] ^= 0x01;

    assert_precompile_error(
        fixture.consume(&verdict),
        0,
        PrecompileError::InvalidSignature,
    );
    assert!(fixture.receipt(&verdict.nonce_hash()).is_none());
}

#[test]
fn non_canonical_precompile_layout_is_rejected_by_the_verifier() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    // The native program ignores bytes past the referenced message, so this
    // still verifies natively; the verifier must refuse it.
    let mut precompile = verdict.precompile();
    precompile.data.push(0);
    let instructions = [
        precompile,
        fixture.consume_instruction(verdict.nonce_hash()),
    ];
    let consumer = fixture.consumer.insecure_clone();

    assert_verifier_error(
        fixture.send(&instructions, &[&consumer]),
        1,
        VerifierError::InvalidPrecompileInstruction,
    );
    assert!(fixture.receipt(&verdict.nonce_hash()).is_none());
}

#[test]
fn signature_from_an_untrusted_key_cannot_authorize() {
    let mut fixture = Fixture::new();
    let attacker = SyntheticSigner::generate().unwrap();
    let forged = SignedVerdict::sign(&attacker, &VerdictClaims::default());

    // Natively valid under the attacker's own key, but not the trusted one.
    assert_verifier_error(
        fixture.consume(&forged),
        1,
        VerifierError::PublicKeyMismatch,
    );

    // Presented under the trusted key, the native program rejects it.
    let mut substituted = forged.clone();
    substituted.public_key = fixture.signer.public_key().unwrap();
    assert_precompile_error(
        fixture.consume(&substituted),
        0,
        PrecompileError::InvalidSignature,
    );
    assert!(fixture.receipt(&forged.nonce_hash()).is_none());
}

#[test]
fn revoked_key_cannot_authorize() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    fixture
        .revoke_key(fixture.trusted_key)
        .expect("administrator revokes");

    assert_verifier_error(fixture.consume(&verdict), 1, VerifierError::KeyRevoked);
    assert!(fixture.receipt(&verdict.nonce_hash()).is_none());
}

#[test]
fn rejected_attempt_does_not_burn_the_nonce() {
    let mut fixture = Fixture::new();
    let wrong_amount = fixture.sign(&VerdictClaims {
        amount: "341".into(),
        ..VerdictClaims::default()
    });
    assert_verifier_error(
        fixture.consume(&wrong_amount),
        1,
        VerifierError::AmountMismatch,
    );
    assert!(
        fixture
            .svm
            .get_account(&fixture.receipt_address(&wrong_amount.nonce_hash()))
            .is_none(),
        "the failed transaction created no account"
    );

    let valid = fixture.sign(&VerdictClaims::default());
    assert_eq!(valid.nonce_hash(), wrong_amount.nonce_hash());
    fixture
        .consume(&valid)
        .expect("the same nonce is still unused");
}

#[test]
fn high_s_signature_needs_normalization() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    assert!(is_low_s(&verdict.signature).unwrap());

    let mut high = verdict.clone();
    high.signature = high_s_twin(&verdict.signature).unwrap();
    assert_precompile_error(fixture.consume(&high), 0, PrecompileError::InvalidSignature);

    fixture
        .consume(&verdict)
        .expect("the normalized twin is accepted");
}

#[test]
fn nonce_hash_argument_must_match_the_signed_nonce() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let other_hash = es256_authorization_verifier::nonce_hash(b"n_other");
    let instructions = [
        verdict.precompile(),
        fixture.consume_instruction(other_hash),
    ];
    let consumer = fixture.consumer.insecure_clone();

    assert_verifier_error(
        fixture.send(&instructions, &[&consumer]),
        1,
        VerifierError::NonceHashMismatch,
    );
    assert!(fixture.receipt(&other_hash).is_none());
}

#[test]
fn only_the_configured_consumer_can_consume() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let mut consume = fixture.consume_instruction(verdict.nonce_hash());
    consume.accounts[0].pubkey = fixture.outsider.pubkey();
    let outsider = fixture.outsider.insecure_clone();

    assert_verifier_error(
        fixture.send(&[verdict.precompile(), consume], &[&outsider]),
        1,
        VerifierError::Unauthorized,
    );
}

#[test]
fn only_the_upgrade_authority_initializes_a_config() {
    let mut fixture = Fixture::deployed();
    let outsider = fixture.outsider.insecure_clone();
    assert_verifier_error(
        fixture.initialize_config(&outsider, Environment::Production),
        0,
        VerifierError::Unauthorized,
    );
    let deployer = fixture.deployer.insecure_clone();
    fixture
        .initialize_config(&deployer, Environment::Production)
        .expect("upgrade authority initializes");
}

#[test]
fn production_config_refuses_sandbox_keys() {
    let mut fixture = Fixture::new();
    let public_key = SyntheticSigner::generate().unwrap().public_key().unwrap();
    let failure = fixture
        .register_key("sandbox-2026-09", public_key)
        .expect_err("sandbox kid is refused");
    assert_eq!(
        failure.err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(u32::from(VerifierError::InvalidKeyId))
        )
    );
}

#[test]
fn typical_verdict_fits_transaction_and_compute_limits() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let instructions: [Instruction; 2] = [
        secp256r1_instruction(
            &verdict.public_key,
            &verdict.signature,
            &verdict.signing_input,
        )
        .unwrap(),
        fixture.consume_instruction(verdict.nonce_hash()),
    ];
    let consumer = fixture.consumer.insecure_clone();
    let transaction = fixture.transaction(&instructions, &[&consumer]);
    let size = transaction_size(&transaction);
    let meta = fixture.svm.send_transaction(transaction).expect("consumed");

    println!(
        "signing input {} bytes, transaction {size} bytes, {} compute units",
        verdict.signing_input.len(),
        meta.compute_units_consumed
    );
    assert!(size <= 1_232);
    assert!(meta.compute_units_consumed <= 200_000);
}
