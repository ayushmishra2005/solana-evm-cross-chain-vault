//! Validly signed verdicts that must still be refused because the context
//! does not match what the administrator approved, and account
//! substitutions around `consume_authorization`.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::prelude::Pubkey;
use anchor_lang::{AccountSerialize, Discriminator};
use common::*;
use es256_authorization_verifier::{
    Environment, RegisterMandateParams, VerifierConfig, VerifierError, mandate_hash,
};
use solana_signer::Signer;

fn assert_refused(fixture: &mut Fixture, claims: VerdictClaims, expected: VerifierError) {
    let verdict = fixture.sign(&claims);
    assert_verifier_error(fixture.consume(&verdict), 1, expected);
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn issuer_must_match_exactly() {
    let mut fixture = Fixture::new();
    for iss in [
        "https://verdicts.saifuro.co",
        "https://verdicts.saifuro.com/",
        "http://verdicts.saifuro.com",
        "HTTPS://VERDICTS.SAIFURO.COM",
        "https://evil.example",
        "",
    ] {
        let claims = VerdictClaims {
            iss: iss.into(),
            ..VerdictClaims::default()
        };
        assert_refused(&mut fixture, claims, VerifierError::IssuerMismatch);
    }
}

#[test]
fn only_an_allow_verdict_authorizes() {
    let mut fixture = Fixture::new();
    for verdict in [
        "deny", "review", "escalate", "Allow", "ALLOW", "allow ", "allowed",
    ] {
        let claims = VerdictClaims {
            verdict: verdict.into(),
            ..VerdictClaims::default()
        };
        assert_refused(&mut fixture, claims, VerifierError::VerdictNotAllow);
    }
}

#[test]
fn every_signed_context_field_must_match_case_sensitively() {
    type Mutation = fn(&mut VerdictClaims);
    let cases: [(&str, Mutation, VerifierError); 10] = [
        (
            "mandate id",
            |c| c.mandate = "mnd_7F3A".into(),
            VerifierError::MandateMismatch,
        ),
        (
            "amount",
            |c| c.amount = "339.999999".into(),
            VerifierError::AmountMismatch,
        ),
        (
            "currency",
            |c| c.currency = "EUR".into(),
            VerifierError::CurrencyMismatch,
        ),
        (
            "currency case",
            |c| c.currency = "usd".into(),
            VerifierError::CurrencyMismatch,
        ),
        (
            "recipient",
            |c| c.recipient = "vendor:attacker".into(),
            VerifierError::RecipientMismatch,
        ),
        (
            "recipient case",
            |c| c.recipient = "Vendor:acme_saas".into(),
            VerifierError::RecipientMismatch,
        ),
        (
            "policy version",
            |c| c.policy_version = "pol_2026-08-12.2".into(),
            VerifierError::PolicyMismatch,
        ),
        (
            "agent",
            |c| c.agent = "agt_procurement_02".into(),
            VerifierError::AgentMismatch,
        ),
        (
            "category",
            |c| c.category = "travel".into(),
            VerifierError::CategoryMismatch,
        ),
        (
            "category case",
            |c| c.category = "SaaS".into(),
            VerifierError::CategoryMismatch,
        ),
    ];
    let mut fixture = Fixture::new();
    for (name, mutate, expected) in cases {
        let mut claims = VerdictClaims::default();
        mutate(&mut claims);
        let verdict = fixture.sign(&claims);
        let (index, error) = instruction_error(fixture.consume(&verdict));
        assert_eq!(
            (index, error),
            (
                1,
                solana_instruction::error::InstructionError::Custom(u32::from(expected))
            ),
            "{name}"
        );
    }
    fixture.assert_no_receipt(&es256_authorization_verifier::nonce_hash(NONCE.as_bytes()));
}

#[test]
fn revoked_mandate_cannot_be_consumed() {
    let mut fixture = Fixture::new();
    fixture.revoke_mandate(fixture.mandate).unwrap();
    assert_refused(
        &mut fixture,
        VerdictClaims::default(),
        VerifierError::MandateRevoked,
    );
}

#[test]
fn unregistered_mandate_account_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.mandate = mandate_address(fixture.config, &mandate_hash(b"mnd_unregistered"));
    let verdict = fixture.sign(&VerdictClaims {
        mandate: "mnd_unregistered".into(),
        ..VerdictClaims::default()
    });
    assert_anchor_error(
        fixture.consume(&verdict),
        1,
        AnchorError::AccountNotInitialized,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

/// The consumer picks the mandate account, but it cannot pick terms: the
/// signed claims must equal whichever approved account it names.
#[test]
fn choosing_a_different_approved_mandate_does_not_change_the_terms() {
    let mut fixture = Fixture::new();
    let larger = fixture
        .register_mandate(RegisterMandateParams {
            mandate_hash: mandate_hash(b"mnd_large"),
            mandate_id: "mnd_large".into(),
            amount_micros: 10_000_000_000,
            ..approved_mandate()
        })
        .unwrap();
    let approved_mandate_account = fixture.mandate;

    // A verdict for the small mandate presented against the large one.
    fixture.mandate = larger;
    assert_refused(
        &mut fixture,
        VerdictClaims::default(),
        VerifierError::MandateMismatch,
    );
    // A verdict naming the large mandate but signed for the small amount.
    assert_refused(
        &mut fixture,
        VerdictClaims {
            mandate: "mnd_large".into(),
            ..VerdictClaims::default()
        },
        VerifierError::AmountMismatch,
    );
    // A verdict for the large mandate presented against the small one.
    fixture.mandate = approved_mandate_account;
    assert_refused(
        &mut fixture,
        VerdictClaims {
            mandate: "mnd_large".into(),
            amount: "10000".into(),
            ..VerdictClaims::default()
        },
        VerifierError::MandateMismatch,
    );
}

#[test]
fn accounts_from_another_config_are_rejected() {
    let mut fixture = Fixture::new();
    let production = fixture.config;
    let production_key = fixture.trusted_key;
    let production_mandate = fixture.mandate;
    fixture.switch_to_new_config(Environment::Sandbox);
    let public_key = fixture.signer.public_key().unwrap();
    let sandbox_key = fixture.register_key("sandbox-test-01", public_key).unwrap();
    let sandbox_mandate = fixture.register_mandate(approved_mandate()).unwrap();
    fixture.config = production;
    let verdict = fixture.sign(&VerdictClaims::default());

    for (trusted_key, mandate) in [
        (sandbox_key, production_mandate),
        (production_key, sandbox_mandate),
    ] {
        fixture.trusted_key = trusted_key;
        fixture.mandate = mandate;
        assert_verifier_error(fixture.consume(&verdict), 1, VerifierError::Unauthorized);
    }

    // The sandbox config with production key and mandate.
    fixture.config = config_address(Environment::Sandbox);
    fixture.trusted_key = production_key;
    fixture.mandate = production_mandate;
    let mut consume = fixture.consume_instruction(verdict.nonce_hash());
    consume.accounts[consume_account::RECEIPT].pubkey =
        receipt_address(production, &verdict.nonce_hash());
    assert_anchor_error(
        fixture.send_as_consumer(&[verdict.precompile(), consume]),
        1,
        AnchorError::ConstraintSeeds,
    );
    assert_verifier_error(fixture.consume(&verdict), 1, VerifierError::Unauthorized);

    assert!(fixture.receipt(&verdict.nonce_hash()).is_none());
    fixture.config = production;
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn account_of_the_wrong_type_is_rejected() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let trusted_key = fixture.trusted_key;
    fixture.trusted_key = fixture.mandate;
    assert_anchor_error(
        fixture.consume(&verdict),
        1,
        AnchorError::AccountDiscriminatorMismatch,
    );
    fixture.trusted_key = trusted_key;
    fixture.mandate = fixture.config;
    assert_anchor_error(
        fixture.consume(&verdict),
        1,
        AnchorError::AccountDiscriminatorMismatch,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

/// Accounts the program could never create, injected into LiteSVM to
/// check that validation does not rely on that alone.
#[test]
fn forged_config_accounts_are_rejected() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let forged = VerifierConfig {
        administrator: fixture.outsider.pubkey(),
        consumer: fixture.outsider.pubkey(),
        environment: Environment::Production,
        bump: 255,
    };
    let mut data = Vec::new();
    forged.try_serialize(&mut data).unwrap();
    assert_eq!(&data[..8], VerifierConfig::DISCRIMINATOR);

    let program_owned = Pubkey::new_unique();
    let system_owned = Pubkey::new_unique();
    for (address, owner) in [
        (program_owned, es256_authorization_verifier::ID),
        (system_owned, solana_sdk_ids::system_program::ID),
    ] {
        fixture
            .svm
            .set_account(
                address,
                solana_account::Account {
                    lamports: 10_000_000,
                    data: data.clone(),
                    owner,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    let outsider = fixture.outsider.insecure_clone();
    let mut consume = fixture.consume_instruction(verdict.nonce_hash());
    consume.accounts[consume_account::CONSUMER].pubkey = outsider.pubkey();
    consume.accounts[consume_account::CONFIG].pubkey = program_owned;
    assert_anchor_error(
        fixture.send(&[verdict.precompile(), consume.clone()], &[&outsider]),
        1,
        AnchorError::ConstraintSeeds,
    );
    consume.accounts[consume_account::CONFIG].pubkey = system_owned;
    assert_anchor_error(
        fixture.send(&[verdict.precompile(), consume], &[&outsider]),
        1,
        AnchorError::AccountOwnedByWrongProgram,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn consumer_must_sign() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let mut consume = fixture.consume_instruction(verdict.nonce_hash());
    consume.accounts[consume_account::CONSUMER].is_signer = false;
    let outsider = fixture.outsider.insecure_clone();

    assert_anchor_error(
        fixture.send(&[verdict.precompile(), consume], &[&outsider]),
        1,
        AnchorError::AccountNotSigner,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn administrator_cannot_act_as_consumer() {
    let mut fixture = Fixture::new();
    let verdict = fixture.sign(&VerdictClaims::default());
    let mut consume = fixture.consume_instruction(verdict.nonce_hash());
    consume.accounts[consume_account::CONSUMER].pubkey = fixture.administrator.pubkey();
    let administrator = fixture.administrator.insecure_clone();

    assert_verifier_error(
        fixture.send(&[verdict.precompile(), consume], &[&administrator]),
        1,
        VerifierError::Unauthorized,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}
