#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::prelude::Pubkey;
use common::deployment::*;
use common::*;
use es256_authorization_verifier::{
    DEPLOYMENT_PROFILE_VERSION, InitializeDeploymentAuthorityParams, RegisterRiskKeyParams,
    VerifierError,
};
use solana_signer::Signer;

fn params(administrator: Pubkey) -> InitializeDeploymentAuthorityParams {
    InitializeDeploymentAuthorityParams {
        administrator,
        network_domain: NETWORK_DOMAIN,
    }
}

#[test]
fn the_upgrade_authority_binds_one_namespace_to_one_authority() {
    let (fixture, profile) = Fixture::deployment_profile();
    let authority = fixture.deployment_authority(profile.config);
    assert_eq!(authority.version, DEPLOYMENT_PROFILE_VERSION);
    assert_eq!(authority.administrator, fixture.administrator.pubkey());
    assert_eq!(authority.remote_leg_program, FORWARDER_ID);
    assert_eq!(authority.remote_config, profile.remote_config);
    assert_eq!(authority.network_domain, NETWORK_DOMAIN);
    assert_eq!(authority.policy_generation, 0);

    let risk_key = fixture.risk_key_account(profile.risk_key);
    assert_eq!(risk_key.config, profile.config);
    assert_eq!(risk_key.public_key, fixture.signer.public_key().unwrap());
    assert!(!risk_key.revoked);
}

#[test]
fn only_the_upgrade_authority_may_initialize() {
    let mut fixture = Fixture::deployed();
    fixture.add_cpi_forwarder();
    let remote_config = fixture.add_remote_config(FORWARDER_ID);
    for signer in [
        fixture.administrator.insecure_clone(),
        fixture.outsider.insecure_clone(),
    ] {
        let result = fixture.initialize_deployment_authority(&signer, FORWARDER_ID, remote_config);
        assert_verifier_error(outcome(result), 0, VerifierError::Unauthorized);
    }
    fixture.assert_no_account(&deployment_authority_address(FORWARDER_ID, remote_config));
}

#[test]
fn an_immutable_program_cannot_initialize() {
    let mut fixture = Fixture::deployed();
    fixture.add_cpi_forwarder();
    let remote_config = fixture.add_remote_config(FORWARDER_ID);
    let mut program_data = fixture.svm.get_account(&program_data_address()).unwrap();
    program_data.data = common::program_data(None);
    fixture
        .svm
        .set_account(program_data_address(), program_data)
        .unwrap();
    let deployer = fixture.deployer.insecure_clone();
    let result = fixture.initialize_deployment_authority(&deployer, FORWARDER_ID, remote_config);
    assert_verifier_error(outcome(result), 0, VerifierError::Unauthorized);
}

#[test]
fn a_substituted_program_data_account_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture.add_cpi_forwarder();
    let remote_config = fixture.add_remote_config(FORWARDER_ID);
    let fake = Pubkey::new_unique();
    fixture
        .svm
        .set_account(
            fake,
            solana_account::Account {
                lamports: 1_000_000_000,
                data: common::program_data(Some(fixture.outsider.pubkey())),
                owner: anchor_lang::solana_program::bpf_loader_upgradeable::ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
    let outsider = fixture.outsider.insecure_clone();
    let instruction = initialize_deployment_authority_instruction(
        outsider.pubkey(),
        fake,
        FORWARDER_ID,
        remote_config,
        params(fixture.administrator.pubkey()),
    );
    let result = fixture.send(&[instruction], &[&outsider]);
    assert_anchor_error(result, 0, AnchorError::ConstraintSeeds);
}

#[test]
fn the_remote_leg_must_be_another_executable_program() {
    let mut fixture = Fixture::deployed();
    fixture.add_cpi_forwarder();
    let deployer = fixture.deployer.insecure_clone();

    // A data account in the program slot.
    let not_a_program = fixture.add_remote_config(FORWARDER_ID);
    let remote_config = fixture.add_remote_config(not_a_program);
    let result = fixture.initialize_deployment_authority(&deployer, not_a_program, remote_config);
    assert_verifier_error(
        outcome(result),
        0,
        VerifierError::InvalidDeploymentAuthority,
    );

    // The verifier cannot authorize deployments into itself.
    let own_config = fixture.add_remote_config(es256_authorization_verifier::ID);
    let result = fixture.initialize_deployment_authority(
        &deployer,
        es256_authorization_verifier::ID,
        own_config,
    );
    assert_verifier_error(
        outcome(result),
        0,
        VerifierError::InvalidDeploymentAuthority,
    );
}

#[test]
fn the_remote_config_must_be_a_live_account_of_the_remote_leg() {
    let mut fixture = Fixture::deployed();
    fixture.add_cpi_forwarder();
    let deployer = fixture.deployer.insecure_clone();

    let foreign = fixture.add_remote_config(anchor_lang::system_program::ID);
    let result = fixture.initialize_deployment_authority(&deployer, FORWARDER_ID, foreign);
    assert_verifier_error(
        outcome(result),
        0,
        VerifierError::InvalidDeploymentAuthority,
    );

    let empty = Pubkey::new_unique();
    fixture
        .svm
        .set_account(
            empty,
            solana_account::Account {
                lamports: 1_000_000,
                data: Vec::new(),
                owner: FORWARDER_ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
    let result = fixture.initialize_deployment_authority(&deployer, FORWARDER_ID, empty);
    assert_verifier_error(
        outcome(result),
        0,
        VerifierError::InvalidDeploymentAuthority,
    );

    let missing = Pubkey::new_unique();
    let result = fixture.initialize_deployment_authority(&deployer, FORWARDER_ID, missing);
    assert_verifier_error(
        outcome(result),
        0,
        VerifierError::InvalidDeploymentAuthority,
    );
}

#[test]
fn a_default_administrator_or_zero_network_domain_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture.add_cpi_forwarder();
    let remote_config = fixture.add_remote_config(FORWARDER_ID);
    let deployer = fixture.deployer.insecure_clone();
    for params in [
        params(Pubkey::default()),
        InitializeDeploymentAuthorityParams {
            administrator: fixture.administrator.pubkey(),
            network_domain: [0; 32],
        },
    ] {
        let instruction = initialize_deployment_authority_instruction(
            deployer.pubkey(),
            program_data_address(),
            FORWARDER_ID,
            remote_config,
            params,
        );
        let result = fixture.send(&[instruction], &[&deployer]);
        assert_verifier_error(result, 0, VerifierError::InvalidDeploymentAuthority);
    }
}

#[test]
fn a_namespace_is_bound_once_and_cannot_be_rebound() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let deployer = fixture.deployer.insecure_clone();
    let instruction = initialize_deployment_authority_instruction(
        deployer.pubkey(),
        program_data_address(),
        FORWARDER_ID,
        profile.remote_config,
        params(fixture.outsider.pubkey()),
    );
    let result = fixture.send(&[instruction], &[&deployer]);
    let (index, _) = instruction_error(result);
    assert_eq!(index, 0);
    assert_eq!(
        fixture.deployment_authority(profile.config).administrator,
        fixture.administrator.pubkey()
    );
}

#[test]
fn only_the_administrator_registers_risk_keys() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let other = es256_jws_adapter::SyntheticSigner::generate()
        .unwrap()
        .public_key()
        .unwrap();
    for signer in [
        fixture.deployer.insecure_clone(),
        fixture.outsider.insecure_clone(),
    ] {
        let instruction = register_risk_key_instruction(
            signer.pubkey(),
            profile.config,
            RegisterRiskKeyParams {
                public_key: other,
                not_before: KEY_NOT_BEFORE,
                not_after: KEY_NOT_AFTER,
            },
        );
        let result = fixture.send(&[instruction], &[&signer]);
        assert_verifier_error(result, 0, VerifierError::Unauthorized);
    }
    fixture.assert_no_account(&risk_key_address(profile.config, &other));
}

#[test]
fn a_payment_profile_config_cannot_hold_risk_keys() {
    let mut fixture = Fixture::new();
    let public_key = fixture.signer.public_key().unwrap();
    let v1_config = fixture.config;
    let result = fixture.register_risk_key(v1_config, public_key, KEY_NOT_BEFORE, KEY_NOT_AFTER);
    assert_anchor_error(
        outcome(result),
        0,
        AnchorError::AccountDiscriminatorMismatch,
    );
}

#[test]
fn risk_key_parameters_are_validated() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let valid = es256_jws_adapter::SyntheticSigner::generate()
        .unwrap()
        .public_key()
        .unwrap();
    let mut uncompressed = valid;
    uncompressed[0] = 0x04;
    let result =
        fixture.register_risk_key(profile.config, uncompressed, KEY_NOT_BEFORE, KEY_NOT_AFTER);
    assert_verifier_error(outcome(result), 0, VerifierError::InvalidPublicKey);
    for (not_before, not_after) in [(NOW, NOW), (NOW + 1, NOW), (-1, NOW)] {
        let result = fixture.register_risk_key(profile.config, valid, not_before, not_after);
        assert_verifier_error(outcome(result), 0, VerifierError::InvalidKeyWindow);
    }
}

#[test]
fn a_revoked_risk_key_stays_revoked() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    fixture
        .revoke_risk_key(profile.config, profile.risk_key)
        .unwrap();
    assert!(fixture.risk_key_account(profile.risk_key).revoked);
    assert_verifier_error(
        fixture.revoke_risk_key(profile.config, profile.risk_key),
        0,
        VerifierError::AlreadyRevoked,
    );
    let public_key = fixture.signer.public_key().unwrap();
    let result =
        fixture.register_risk_key(profile.config, public_key, KEY_NOT_BEFORE, KEY_NOT_AFTER);
    let (index, _) = instruction_error(outcome(result));
    assert_eq!(index, 0);
    assert!(fixture.risk_key_account(profile.risk_key).revoked);
}

#[test]
fn only_the_administrator_revokes_and_only_its_own_keys() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let outsider = fixture.outsider.insecure_clone();
    let instruction =
        revoke_risk_key_instruction(outsider.pubkey(), profile.config, profile.risk_key);
    assert_verifier_error(
        fixture.send(&[instruction], &[&outsider]),
        0,
        VerifierError::Unauthorized,
    );

    // The same administrator's key under another namespace.
    let other_config = fixture.add_remote_config(FORWARDER_ID);
    let deployer = fixture.deployer.insecure_clone();
    let other = fixture
        .initialize_deployment_authority(&deployer, FORWARDER_ID, other_config)
        .unwrap();
    assert_verifier_error(
        fixture.revoke_risk_key(other, profile.risk_key),
        0,
        VerifierError::Unauthorized,
    );
    assert!(!fixture.risk_key_account(profile.risk_key).revoked);
}

#[test]
fn the_policy_generation_only_advances() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    for expected in 1..=3 {
        fixture.advance_policy_generation(profile.config).unwrap();
        assert_eq!(
            fixture
                .deployment_authority(profile.config)
                .policy_generation,
            expected
        );
    }
    let outsider = fixture.outsider.insecure_clone();
    let instruction = advance_policy_generation_instruction(outsider.pubkey(), profile.config);
    assert_verifier_error(
        fixture.send(&[instruction], &[&outsider]),
        0,
        VerifierError::Unauthorized,
    );

    let mut account = fixture.svm.get_account(&profile.config).unwrap();
    let generation = account.data.len() - 8;
    account.data[generation..].copy_from_slice(&u64::MAX.to_le_bytes());
    fixture.svm.set_account(profile.config, account).unwrap();
    assert_verifier_error(
        fixture.advance_policy_generation(profile.config),
        0,
        VerifierError::PolicyGenerationExhausted,
    );
    assert_eq!(
        fixture
            .deployment_authority(profile.config)
            .policy_generation,
        u64::MAX
    );
}

#[test]
fn payment_profile_accounts_are_not_deployment_authorities() {
    let mut fixture = Fixture::new();
    let v1_config = fixture.config;
    let trusted_key = fixture.trusted_key;
    let administrator = fixture.administrator.insecure_clone();
    for instruction in [
        advance_policy_generation_instruction(administrator.pubkey(), v1_config),
        revoke_risk_key_instruction(administrator.pubkey(), v1_config, trusted_key),
    ] {
        assert_anchor_error(
            fixture.send(&[instruction], &[&administrator]),
            0,
            AnchorError::AccountDiscriminatorMismatch,
        );
    }
}
