//! One time setup of the remote leg.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::{
    CONFIG_VERSION, CONTROL_LANE_ID, DEPLOYMENT_ID, DESTINATION_CHAIN_ID, Fixture,
    LOCAL_APPLICATION_ID, Pubkey, REPORT_LANE_ID, SOURCE_APPLICATION_ID, SOURCE_CHAIN_ID,
    START_TIMESTAMP, VAULT_ID, expect_error, expect_rejection,
};
use solevm_remote_leg::state::STATE_VERSION;
use solevm_remote_leg::{RemoteConfig, RemoteLegError};

#[test]
fn a_valid_setup_stores_every_configured_field() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("initialize succeeds");

    let config = fixture.config();
    assert_eq!(config.state_version, STATE_VERSION);
    assert!(!config.frozen);
    assert_eq!(config.administrator, fixture.administrator_key());
    assert_eq!(config.emergency_guardian, fixture.guardian_key());
    assert_eq!(config.transport_verifier, fixture.verifier_key());
    assert_eq!(config.asset_mint, fixture.mint);
    assert_eq!(config.custody_token_account, fixture.custody);
    assert_eq!(config.outbound_escrow, fixture.escrow);
    assert_eq!(config.custody_authority, fixture.custody_authority);
    assert_eq!(
        config.custody_authority_bump,
        fixture.custody_authority_bump
    );
    assert_eq!(config.deployment_id, DEPLOYMENT_ID);
    assert_eq!(config.vault_id, VAULT_ID);
    assert_eq!(config.source_chain_id, SOURCE_CHAIN_ID);
    assert_eq!(config.destination_chain_id, DESTINATION_CHAIN_ID);
    assert_eq!(config.source_application_id, SOURCE_APPLICATION_ID);
    assert_eq!(config.local_application_id, LOCAL_APPLICATION_ID);
    assert_eq!(config.control_lane_id, CONTROL_LANE_ID);
    assert_eq!(config.report_lane_id, REPORT_LANE_ID);
    assert_eq!(config.config_version, CONFIG_VERSION);
    assert_eq!(config.initialized_at, START_TIMESTAMP);
    assert_eq!(config.reserved, [0u8; 64]);
}

#[test]
fn the_stored_bump_reproduces_the_configuration_address() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("initialize succeeds");

    let config = fixture.config();
    let derived = Pubkey::create_program_address(
        &[
            solevm_remote_leg::REMOTE_CONFIG_SEED,
            &config.deployment_id,
            &config.vault_id,
            &[config.bump],
        ],
        &solevm_remote_leg::ID,
    )
    .expect("bump rebuilds the address");
    assert_eq!(derived, fixture.config);
}

#[test]
fn the_stored_custody_bump_reproduces_the_custody_authority() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("initialize succeeds");

    let config = fixture.config();
    let derived = Pubkey::create_program_address(
        &[
            solevm_remote_leg::CUSTODY_AUTHORITY_SEED,
            fixture.config.as_ref(),
            &[config.custody_authority_bump],
        ],
        &solevm_remote_leg::ID,
    )
    .expect("bump rebuilds the address");
    assert_eq!(derived, config.custody_authority);
}

#[test]
fn the_account_uses_the_documented_size() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("initialize succeeds");

    let account = fixture.svm.get_account(&fixture.config).unwrap();
    assert_eq!(account.data.len(), RemoteConfig::LEN);
    assert_eq!(account.owner, solevm_remote_leg::ID);
}

#[test]
fn a_second_setup_of_the_same_vault_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("initialize succeeds");
    expect_rejection(fixture.initialize());
}

#[test]
fn a_default_guardian_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.emergency_guardian = Pubkey::default();
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::InvalidAuthority,
    );
}

#[test]
fn a_default_transport_verifier_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.transport_verifier = Pubkey::default();
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::InvalidAuthority,
    );
}

#[test]
fn an_administrator_that_is_also_the_guardian_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.emergency_guardian = fixture.administrator_key();
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::EqualAuthorities,
    );
}

#[test]
fn a_zero_source_chain_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.source_chain_id = 0;
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::InvalidSourceDomain,
    );
}

#[test]
fn a_zero_destination_chain_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.destination_chain_id = 0;
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::InvalidDestinationDomain,
    );
}

#[test]
fn a_source_chain_equal_to_the_destination_chain_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.destination_chain_id = params.source_chain_id;
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::InvalidDestinationDomain,
    );
}

#[test]
fn a_zero_source_application_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.source_application_id = [0u8; 32];
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::InvalidApplication,
    );
}

#[test]
fn a_zero_local_application_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.local_application_id = [0u8; 32];
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::InvalidApplication,
    );
}

#[test]
fn equal_application_bytes_on_different_chains_initialize() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.local_application_id = params.source_application_id;
    assert_ne!(params.source_chain_id, params.destination_chain_id);

    fixture
        .initialize_with_params(params)
        .expect("equal application bytes are valid across chains");

    let config = fixture.config();
    assert_eq!(config.source_application_id, SOURCE_APPLICATION_ID);
    assert_eq!(config.local_application_id, SOURCE_APPLICATION_ID);
}

#[test]
fn a_zero_deployment_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.deployment_id = [0u8; 32];
    let mut accounts = fixture.default_accounts();
    accounts.remote_config = Fixture::config_address(&params.deployment_id, &params.vault_id);
    expect_error(
        fixture.initialize_with(accounts, params),
        RemoteLegError::InvalidDeployment,
    );
}

#[test]
fn a_zero_vault_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.vault_id = [0u8; 32];
    let mut accounts = fixture.default_accounts();
    accounts.remote_config = Fixture::config_address(&params.deployment_id, &params.vault_id);
    expect_error(
        fixture.initialize_with(accounts, params),
        RemoteLegError::InvalidVault,
    );
}

#[test]
fn a_zero_control_lane_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.control_lane_id = 0;
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::InvalidLane,
    );
}

#[test]
fn a_zero_report_lane_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.report_lane_id = 0;
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::InvalidLane,
    );
}

#[test]
fn a_zero_config_version_is_rejected() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.config_version = 0;
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::InvalidConfigVersion,
    );
}

#[test]
fn a_mint_with_the_wrong_decimals_is_rejected() {
    let mut fixture = Fixture::with_mint_decimals(9);
    expect_error(fixture.initialize(), RemoteLegError::InvalidMintDecimals);
}

#[test]
fn a_custody_account_holding_another_mint_is_rejected() {
    let mut fixture = Fixture::new();
    let other_mint = Pubkey::new_unique();
    fixture.write_mint(other_mint, 6);
    let custody = fixture.custody;
    let authority = fixture.custody_authority;
    fixture.write_token_account(custody, other_mint, authority, None, None);
    expect_error(fixture.initialize(), RemoteLegError::InvalidMint);
}

#[test]
fn a_custody_account_under_another_authority_is_rejected() {
    let mut fixture = Fixture::new();
    let custody = fixture.custody;
    let mint = fixture.mint;
    fixture.write_token_account(custody, mint, Pubkey::new_unique(), None, None);
    expect_error(fixture.initialize(), RemoteLegError::InvalidCustodyAccount);
}

#[test]
fn a_custody_account_with_a_delegate_is_rejected() {
    let mut fixture = Fixture::new();
    let custody = fixture.custody;
    let mint = fixture.mint;
    let authority = fixture.custody_authority;
    fixture.write_token_account(custody, mint, authority, Some(Pubkey::new_unique()), None);
    expect_error(fixture.initialize(), RemoteLegError::InvalidCustodyAccount);
}

#[test]
fn a_custody_account_with_a_close_authority_is_rejected() {
    let mut fixture = Fixture::new();
    let custody = fixture.custody;
    let mint = fixture.mint;
    let authority = fixture.custody_authority;
    fixture.write_token_account(custody, mint, authority, None, Some(Pubkey::new_unique()));
    expect_error(fixture.initialize(), RemoteLegError::InvalidCustodyAccount);
}

#[test]
fn an_outbound_escrow_holding_another_mint_is_rejected() {
    let mut fixture = Fixture::new();
    let other_mint = Pubkey::new_unique();
    fixture.write_mint(other_mint, 6);
    let escrow = fixture.escrow;
    let owner = fixture.escrow_owner;
    fixture.write_token_account(escrow, other_mint, owner, None, None);
    expect_error(fixture.initialize(), RemoteLegError::InvalidOutboundEscrow);
}

#[test]
fn an_outbound_escrow_under_the_custody_authority_is_rejected() {
    let mut fixture = Fixture::new();
    let escrow = fixture.escrow;
    let mint = fixture.mint;
    let authority = fixture.custody_authority;
    fixture.write_token_account(escrow, mint, authority, None, None);
    expect_error(fixture.initialize(), RemoteLegError::InvalidOutboundEscrow);
}

#[test]
fn an_outbound_escrow_that_is_the_custody_account_is_rejected() {
    let mut fixture = Fixture::new();
    let mut accounts = fixture.default_accounts();
    accounts.outbound_escrow = accounts.custody_token_account;
    expect_error(
        fixture.initialize_with_accounts(accounts),
        RemoteLegError::InvalidOutboundEscrow,
    );
}

#[test]
fn an_outbound_escrow_with_a_close_authority_is_rejected() {
    let mut fixture = Fixture::new();
    let escrow = fixture.escrow;
    let mint = fixture.mint;
    let owner = fixture.escrow_owner;
    fixture.write_token_account(escrow, mint, owner, None, Some(Pubkey::new_unique()));
    expect_error(fixture.initialize(), RemoteLegError::InvalidOutboundEscrow);
}

#[test]
fn an_outbound_escrow_with_a_delegate_is_rejected() {
    let mut fixture = Fixture::new();
    let escrow = fixture.escrow;
    let mint = fixture.mint;
    let owner = fixture.escrow_owner;
    fixture.write_token_account(escrow, mint, owner, Some(Pubkey::new_unique()), None);
    expect_error(fixture.initialize(), RemoteLegError::InvalidOutboundEscrow);
}

#[test]
fn setting_up_a_second_vault_of_the_same_deployment_is_allowed() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("initialize succeeds");

    let second_vault = [0x77u8; 32];
    let second_config = Fixture::config_address(&DEPLOYMENT_ID, &second_vault);
    let second_custody_authority = Fixture::custody_authority_address(&second_config).0;

    let custody = Pubkey::new_unique();
    let escrow = Pubkey::new_unique();
    let mint = fixture.mint;
    fixture.write_token_account(custody, mint, second_custody_authority, None, None);
    fixture.write_token_account(escrow, mint, Pubkey::new_unique(), None, None);

    let mut params = fixture.params.clone();
    params.vault_id = second_vault;
    let mut accounts = fixture.default_accounts();
    accounts.remote_config = second_config;
    accounts.custody_token_account = custody;
    accounts.outbound_escrow = escrow;

    fixture
        .initialize_with(accounts, params)
        .expect("a second vault initializes");
}
