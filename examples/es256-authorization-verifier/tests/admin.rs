//! Who may create configs, keys and mandates, and what registration
//! accepts.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use common::*;
use es256_authorization_verifier::{
    Environment, InitializeConfigParams, RegisterMandateParams, RegisterTrustedKeyParams,
    VerifierError, mandate_hash,
};
use es256_jws_adapter::SyntheticSigner;
use solana_instruction::error::InstructionError;
use solana_keypair::Keypair;
use solana_signer::Signer;

fn params(fixture: &Fixture, environment: Environment) -> InitializeConfigParams {
    InitializeConfigParams {
        environment,
        administrator: fixture.administrator.pubkey(),
        consumer: fixture.consumer.pubkey(),
    }
}

fn key_params(kid: &str) -> RegisterTrustedKeyParams {
    RegisterTrustedKeyParams {
        kid: kid.into(),
        public_key: SyntheticSigner::generate().unwrap().public_key().unwrap(),
        not_before: KEY_NOT_BEFORE,
        not_after: KEY_NOT_AFTER,
    }
}

// ---- Config bootstrap ---------------------------------------------------------

#[test]
fn immutable_program_cannot_bootstrap_a_config() {
    let mut fixture = Fixture::deployed();
    let mut program_data = fixture.svm.get_account(&program_data_address()).unwrap();
    program_data.data = common::program_data(None);
    fixture
        .svm
        .set_account(program_data_address(), program_data)
        .unwrap();
    let deployer = fixture.deployer.insecure_clone();

    assert_verifier_error(
        fixture.initialize_config(&deployer, Environment::Production),
        0,
        VerifierError::Unauthorized,
    );
}

#[test]
fn forged_program_data_cannot_bootstrap_a_config() {
    let mut fixture = Fixture::deployed();
    let outsider = fixture.outsider.insecure_clone();
    let forged = Pubkey::new_unique();
    fixture
        .svm
        .set_account(
            forged,
            solana_account::Account {
                lamports: 1_000_000_000,
                data: common::program_data(Some(outsider.pubkey())),
                owner: bpf_loader_upgradeable::ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
    assert_anchor_error(
        fixture.initialize_config_with(
            &outsider,
            forged,
            params(&fixture, Environment::Production),
        ),
        0,
        AnchorError::ConstraintSeeds,
    );

    // The right address, but not owned by the upgradeable loader.
    fixture
        .svm
        .set_account(
            program_data_address(),
            solana_account::Account {
                lamports: 1_000_000_000,
                data: common::program_data(Some(outsider.pubkey())),
                owner: solana_sdk_ids::system_program::ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
    assert_anchor_error(
        fixture.initialize_config(&outsider, Environment::Production),
        0,
        AnchorError::AccountOwnedByWrongProgram,
    );
}

#[test]
fn config_cannot_be_reinitialized() {
    let mut fixture = Fixture::new();
    let deployer = fixture.deployer.insecure_clone();
    let before = fixture.svm.get_account(&fixture.config).unwrap();
    let mut takeover = params(&fixture, Environment::Production);
    takeover.consumer = fixture.outsider.pubkey();

    assert_eq!(
        instruction_error(fixture.initialize_config_with(
            &deployer,
            program_data_address(),
            takeover
        )),
        (0, InstructionError::Custom(ACCOUNT_ALREADY_IN_USE))
    );
    assert_eq!(fixture.svm.get_account(&fixture.config), Some(before));
}

#[test]
fn config_roles_must_be_distinct_and_set() {
    let mut fixture = Fixture::deployed();
    let deployer = fixture.deployer.insecure_clone();
    let administrator = fixture.administrator.pubkey();
    for (administrator, consumer) in [
        (Pubkey::default(), fixture.consumer.pubkey()),
        (administrator, Pubkey::default()),
        (administrator, administrator),
    ] {
        assert_verifier_error(
            fixture.initialize_config_with(
                &deployer,
                program_data_address(),
                InitializeConfigParams {
                    environment: Environment::Production,
                    administrator,
                    consumer,
                },
            ),
            0,
            VerifierError::InvalidRoles,
        );
    }
}

// ---- Administrator authority --------------------------------------------------

#[test]
fn only_the_administrator_manages_keys_and_mandates() {
    let mut fixture = Fixture::new();
    let mut new_mandate = approved_mandate();
    new_mandate.mandate_id = "mnd_new".into();
    new_mandate.mandate_hash = mandate_hash(b"mnd_new");

    for intruder in [
        fixture.outsider.insecure_clone(),
        fixture.consumer.insecure_clone(),
        fixture.deployer.insecure_clone(),
    ] {
        let attempts = [
            fixture.register_key_instruction(
                intruder.pubkey(),
                trusted_key_address(fixture.config, "intruder-key"),
                key_params("intruder-key"),
            ),
            fixture.revoke_key_instruction(intruder.pubkey(), fixture.trusted_key),
            fixture.register_mandate_instruction(intruder.pubkey(), new_mandate.clone()),
            fixture.revoke_mandate_instruction(intruder.pubkey(), fixture.mandate),
        ];
        for attempt in attempts {
            assert_verifier_error(
                fixture.send(&[attempt], &[&intruder]),
                0,
                VerifierError::Unauthorized,
            );
        }
    }

    // Nothing changed.
    let verdict = fixture.sign(&VerdictClaims::default());
    fixture
        .consume(&verdict)
        .expect("key and mandate still live");
}

#[test]
fn administrator_must_sign() {
    let mut fixture = Fixture::new();
    let outsider = fixture.outsider.insecure_clone();
    let administrator = fixture.administrator.pubkey();
    let mut attempts = [
        fixture.revoke_key_instruction(administrator, fixture.trusted_key),
        fixture.revoke_mandate_instruction(administrator, fixture.mandate),
    ];
    for attempt in &mut attempts {
        attempt.accounts[0].is_signer = false;
        assert_anchor_error(
            fixture.send(std::slice::from_ref(attempt), &[&outsider]),
            0,
            AnchorError::AccountNotSigner,
        );
    }
}

#[test]
fn another_configs_administrator_cannot_touch_this_config() {
    let mut fixture = Fixture::new();
    let production = fixture.config;
    let production_key = fixture.trusted_key;
    let production_mandate = fixture.mandate;
    // A sandbox config whose administrator is the outsider.
    let rogue = Keypair::new();
    fixture.svm.airdrop(&rogue.pubkey(), 1_000_000_000).unwrap();
    let deployer = fixture.deployer.insecure_clone();
    fixture
        .initialize_config_with(
            &deployer,
            program_data_address(),
            InitializeConfigParams {
                environment: Environment::Sandbox,
                administrator: rogue.pubkey(),
                consumer: fixture.consumer.pubkey(),
            },
        )
        .unwrap();

    for config in [production, config_address(Environment::Sandbox)] {
        fixture.config = config;
        for attempt in [
            fixture.revoke_key_instruction(rogue.pubkey(), production_key),
            fixture.revoke_mandate_instruction(rogue.pubkey(), production_mandate),
        ] {
            assert_verifier_error(
                fixture.send(&[attempt], &[&rogue]),
                0,
                VerifierError::Unauthorized,
            );
        }
    }
    fixture.config = production;
    fixture
        .consume(&fixture.sign(&VerdictClaims::default()))
        .expect("production state is untouched");
}

// ---- Revocation is permanent --------------------------------------------------

#[test]
fn revoked_mandate_cannot_be_revived() {
    let mut fixture = Fixture::new();
    fixture.revoke_mandate(fixture.mandate).unwrap();
    assert_verifier_error(
        fixture.revoke_mandate(fixture.mandate),
        0,
        VerifierError::AlreadyRevoked,
    );
    let failure = fixture
        .register_mandate(approved_mandate())
        .expect_err("the mandate account is a tombstone");
    assert_eq!(
        failure.err,
        solana_transaction_error::TransactionError::InstructionError(
            0,
            InstructionError::Custom(ACCOUNT_ALREADY_IN_USE)
        )
    );
    let verdict = fixture.sign(&VerdictClaims::default());
    assert_verifier_error(fixture.consume(&verdict), 1, VerifierError::MandateRevoked);
}

// ---- Registration validation --------------------------------------------------

#[test]
fn key_ids_must_be_bounded_profile_text() {
    let mut fixture = Fixture::new();
    for kid in [
        "",
        "kid\"quote",
        "kid\\slash",
        "kid\ttab",
        "kidé",
        &"k".repeat(32),
    ] {
        let expected_ok = kid.len() == 32;
        let address = trusted_key_address(fixture.config, kid);
        let instruction = fixture.register_key_instruction(
            fixture.administrator.pubkey(),
            address,
            key_params(kid),
        );
        let administrator = fixture.administrator.insecure_clone();
        let result = fixture.send(&[instruction], &[&administrator]);
        if expected_ok {
            result.expect("a 32-byte kid is the maximum");
        } else {
            assert_verifier_error(result, 0, VerifierError::InvalidKeyId);
        }
    }
}

/// Regression: a kid longer than a PDA seed used to abort address
/// derivation (`ProgramFailedToComplete`) before the handler could reject
/// it.
#[test]
fn key_id_longer_than_a_seed_fails_with_a_typed_error() {
    let mut fixture = Fixture::new();
    let administrator = fixture.administrator.insecure_clone();
    let address = over_long_key_id_address(fixture.config);
    for length in [33, 64, 200] {
        let kid = "k".repeat(length);
        let instruction =
            fixture.register_key_instruction(administrator.pubkey(), address, key_params(&kid));
        assert_verifier_error(
            fixture.send(&[instruction], &[&administrator]),
            0,
            VerifierError::InvalidKeyId,
        );
        fixture.assert_no_account(&address);
    }

    let instruction = fixture.register_key_instruction(
        administrator.pubkey(),
        Pubkey::new_unique(),
        key_params(&"k".repeat(33)),
    );
    assert_anchor_error(
        fixture.send(&[instruction], &[&administrator]),
        0,
        AnchorError::ConstraintSeeds,
    );
}

/// A kid one byte longer than a registered 32-byte kid shares its first 32
/// bytes, so it must neither reach, alter nor stand in for that key.
#[test]
fn over_long_key_id_cannot_collide_with_a_registered_prefix() {
    let mut fixture = Fixture::new();
    let administrator = fixture.administrator.insecure_clone();
    let kid = "verdict-prefix-0123456789abcdefg";
    let long_kid = format!("{kid}x");
    assert_eq!((kid.len(), long_kid.len()), (32, 33));

    let existing_signer = SyntheticSigner::generate().unwrap();
    let existing = fixture
        .register_key(kid, existing_signer.public_key().unwrap())
        .unwrap();
    let before = fixture.svm.get_account(&existing).unwrap();

    // Registration addressed at the existing key's account.
    let instruction =
        fixture.register_key_instruction(administrator.pubkey(), existing, key_params(&long_kid));
    assert_anchor_error(
        fixture.send(&[instruction], &[&administrator]),
        0,
        AnchorError::ConstraintSeeds,
    );
    // Registration at the address the program derives for the long kid.
    let derived = over_long_key_id_address(fixture.config);
    assert_ne!(derived, existing);
    let instruction =
        fixture.register_key_instruction(administrator.pubkey(), derived, key_params(&long_kid));
    assert_verifier_error(
        fixture.send(&[instruction], &[&administrator]),
        0,
        VerifierError::InvalidKeyId,
    );
    fixture.assert_no_account(&derived);
    assert_eq!(fixture.svm.get_account(&existing).unwrap(), before);

    // A token naming the long kid cannot use the existing key.
    fixture.signer = existing_signer;
    fixture.trusted_key = existing;
    let long_claims = VerdictClaims {
        kid: long_kid,
        ..VerdictClaims::default()
    };
    let verdict = fixture.sign(&long_claims);
    assert_verifier_error(
        fixture.consume(&verdict),
        1,
        VerifierError::InvalidFieldLength,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());

    // The existing key is untouched and still authorizes its own kid.
    let verdict = fixture.sign(&VerdictClaims {
        kid: kid.into(),
        ..VerdictClaims::default()
    });
    fixture.consume(&verdict).expect("existing key still works");
}

#[test]
fn mandates_must_be_valid_and_self_consistent() {
    let mut fixture = Fixture::new();
    let with = |edit: fn(&mut RegisterMandateParams)| {
        let mut params = RegisterMandateParams {
            mandate_hash: mandate_hash(b"mnd_case"),
            mandate_id: "mnd_case".into(),
            ..approved_mandate()
        };
        edit(&mut params);
        params
    };
    let cases: [(&str, RegisterMandateParams, VerifierError); 8] = [
        (
            "zero amount",
            with(|p| p.amount_micros = 0),
            VerifierError::InvalidMandate,
        ),
        (
            "empty currency",
            with(|p| p.currency = String::new()),
            VerifierError::InvalidMandate,
        ),
        (
            "quote in recipient",
            with(|p| p.recipient = "vendor:\"x".into()),
            VerifierError::InvalidMandate,
        ),
        (
            "backslash in agent",
            with(|p| p.agent = "agt\\x".into()),
            VerifierError::InvalidMandate,
        ),
        (
            "policy over 64 bytes",
            with(|p| p.policy_version = "p".repeat(65)),
            VerifierError::InvalidMandate,
        ),
        (
            "non-ASCII category",
            with(|p| p.category = "saaś".into()),
            VerifierError::InvalidMandate,
        ),
        (
            "hash of another id",
            with(|p| p.mandate_hash = mandate_hash(b"mnd_unregistered")),
            VerifierError::MandateHashMismatch,
        ),
        (
            "raw id instead of its hash",
            with(|p| {
                let mut raw = [0u8; 32];
                raw[..8].copy_from_slice(b"mnd_case");
                p.mandate_hash = raw;
            }),
            VerifierError::MandateHashMismatch,
        ),
    ];
    for (name, params, expected) in cases {
        let instruction =
            fixture.register_mandate_instruction(fixture.administrator.pubkey(), params);
        let administrator = fixture.administrator.insecure_clone();
        let (index, error) = instruction_error(fixture.send(&[instruction], &[&administrator]));
        assert_eq!(
            (index, error),
            (0, InstructionError::Custom(u32::from(expected))),
            "{name}"
        );
    }
}
