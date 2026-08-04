//! Account substitution attempts against both instructions.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::error::ErrorCode;
use anchor_spl::token::spl_token;
use common::{DEPLOYMENT_ID, Fixture, Pubkey, VAULT_ID, expect_anchor_error, expect_error};
use solevm_remote_leg::RemoteLegError;

/// Token-2022 is out of scope for this milestone.
const TOKEN_2022_ID: Pubkey =
    anchor_lang::prelude::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

fn initialized() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("initialize succeeds");
    fixture
}

#[test]
fn a_configuration_address_off_the_canonical_seeds_is_rejected() {
    let mut fixture = Fixture::new();
    let mut accounts = fixture.default_accounts();
    accounts.remote_config = Fixture::config_address(&DEPLOYMENT_ID, &[0x99u8; 32]);
    expect_anchor_error(
        fixture.initialize_with_accounts(accounts),
        ErrorCode::ConstraintSeeds,
    );
}

#[test]
fn a_configuration_address_that_is_not_a_program_address_is_rejected() {
    let mut fixture = Fixture::new();
    let mut accounts = fixture.default_accounts();
    accounts.remote_config = Pubkey::new_unique();
    expect_anchor_error(
        fixture.initialize_with_accounts(accounts),
        ErrorCode::ConstraintSeeds,
    );
}

#[test]
fn freezing_through_a_substituted_configuration_address_is_rejected() {
    let mut fixture = initialized();
    let administrator = fixture.administrator.insecure_clone();
    let imposter = Pubkey::new_unique();
    assert!(fixture.freeze_with(&administrator, imposter).is_err());
    assert!(!fixture.config().frozen);
}

#[test]
fn freezing_a_configuration_with_a_corrupted_bump_is_rejected() {
    let mut fixture = initialized();
    let real_bump = fixture.config().bump;
    // The bump is the second byte after the eight byte discriminator.
    fixture.overwrite_config_data(|data| data[9] = real_bump.wrapping_sub(1));

    let administrator = fixture.administrator.insecure_clone();
    expect_anchor_error(fixture.freeze(&administrator), ErrorCode::ConstraintSeeds);
}

#[test]
fn freezing_a_configuration_owned_by_another_program_is_rejected() {
    let mut fixture = initialized();
    fixture.reassign_config(Pubkey::new_unique());

    let administrator = fixture.administrator.insecure_clone();
    expect_anchor_error(
        fixture.freeze(&administrator),
        ErrorCode::AccountOwnedByWrongProgram,
    );
}

#[test]
fn freezing_a_configuration_with_a_foreign_discriminator_is_rejected() {
    let mut fixture = initialized();
    fixture.overwrite_config_data(|data| data[0] = data[0].wrapping_add(1));

    let administrator = fixture.administrator.insecure_clone();
    expect_anchor_error(
        fixture.freeze(&administrator),
        ErrorCode::AccountDiscriminatorMismatch,
    );
}

#[test]
fn setting_up_without_the_administrator_signature_is_rejected() {
    let mut fixture = Fixture::new();
    let accounts = fixture.default_accounts();
    let params = fixture.params.clone();
    let mut instruction = fixture.initialize_instruction(accounts, params);
    instruction.accounts[0].is_signer = false;

    let outsider = fixture.outsider.insecure_clone();
    let result = fixture.send_as(instruction, &outsider, &[&outsider]);
    expect_anchor_error(result, ErrorCode::AccountNotSigner);
}

#[test]
fn freezing_without_the_authority_signature_is_rejected() {
    let mut fixture = initialized();
    let administrator = fixture.administrator_key();
    let config = fixture.config;
    let mut instruction = fixture.freeze_instruction(administrator, config);
    instruction.accounts[0].is_signer = false;

    let outsider = fixture.outsider.insecure_clone();
    let result = fixture.send_as(instruction, &outsider, &[&outsider]);
    expect_anchor_error(result, ErrorCode::AccountNotSigner);
    assert!(!fixture.config().frozen);
}

#[test]
fn a_substituted_system_program_is_rejected() {
    let mut fixture = Fixture::new();
    let mut accounts = fixture.default_accounts();
    accounts.system_program = Pubkey::new_unique();
    expect_anchor_error(
        fixture.initialize_with_accounts(accounts),
        ErrorCode::InvalidProgramId,
    );
}

#[test]
fn a_substituted_token_program_is_rejected() {
    let mut fixture = Fixture::new();
    let mut accounts = fixture.default_accounts();
    accounts.token_program = TOKEN_2022_ID;
    expect_anchor_error(
        fixture.initialize_with_accounts(accounts),
        ErrorCode::InvalidProgramId,
    );
}

#[test]
fn a_custody_account_owned_by_another_token_program_is_rejected() {
    let mut fixture = Fixture::new();
    let custody = fixture.custody;
    let mint = fixture.mint;
    let authority = fixture.custody_authority;
    fixture.write_token_account(custody, mint, authority, None, None);
    fixture.reassign_account(custody, TOKEN_2022_ID);
    expect_anchor_error(fixture.initialize(), ErrorCode::AccountOwnedByWrongProgram);
}

#[test]
fn a_mint_owned_by_another_token_program_is_rejected() {
    let mut fixture = Fixture::new();
    let mint = fixture.mint;
    fixture.reassign_account(mint, TOKEN_2022_ID);
    expect_anchor_error(fixture.initialize(), ErrorCode::AccountOwnedByWrongProgram);
}

#[test]
fn a_custody_account_that_is_really_a_mint_is_rejected() {
    let mut fixture = Fixture::new();
    let mut accounts = fixture.default_accounts();
    accounts.custody_token_account = accounts.asset_mint;
    assert!(fixture.initialize_with_accounts(accounts).is_err());
}

#[test]
fn the_same_token_account_used_for_custody_and_escrow_is_rejected() {
    let mut fixture = Fixture::new();
    let mut accounts = fixture.default_accounts();
    accounts.outbound_escrow = accounts.custody_token_account;
    expect_error(
        fixture.initialize_with_accounts(accounts),
        RemoteLegError::InvalidOutboundEscrow,
    );
}

#[test]
fn the_configuration_address_reused_as_a_token_account_is_rejected() {
    let mut fixture = Fixture::new();
    let mut accounts = fixture.default_accounts();
    accounts.custody_token_account = accounts.remote_config;
    assert!(fixture.initialize_with_accounts(accounts).is_err());
}

#[test]
fn setting_up_again_over_a_prepared_account_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("initialize succeeds");

    // A fresh account at the same address must not reopen initialization.
    let owner = fixture.config;
    fixture.write_owned_account(owner, vec![0u8; 8], spl_token::ID);
    assert!(fixture.initialize().is_err());
}

#[test]
fn a_custody_account_bound_to_another_vault_authority_is_rejected() {
    let mut fixture = Fixture::new();
    let other_config = Fixture::config_address(&DEPLOYMENT_ID, &[0x55u8; 32]);
    let other_authority = Fixture::custody_authority_address(&other_config).0;

    let custody = fixture.custody;
    let mint = fixture.mint;
    fixture.write_token_account(custody, mint, other_authority, None, None);
    expect_error(fixture.initialize(), RemoteLegError::InvalidCustodyAccount);
}

#[test]
fn a_custody_authority_derived_from_another_program_is_rejected() {
    let mut fixture = Fixture::new();
    let config = Fixture::config_address(&DEPLOYMENT_ID, &VAULT_ID);
    let foreign = Pubkey::find_program_address(
        &[solevm_remote_leg::CUSTODY_AUTHORITY_SEED, config.as_ref()],
        &Pubkey::new_unique(),
    )
    .0;

    let custody = fixture.custody;
    let mint = fixture.mint;
    fixture.write_token_account(custody, mint, foreign, None, None);
    expect_error(fixture.initialize(), RemoteLegError::InvalidCustodyAccount);
}

#[test]
fn an_uninitialized_custody_account_is_rejected() {
    let mut fixture = Fixture::new();
    let custody = fixture.custody;
    fixture.write_owned_account(custody, vec![0u8; 165], spl_token::ID);
    assert!(fixture.initialize().is_err());
}

#[test]
fn a_failed_setup_leaves_no_configuration_account_behind() {
    let mut fixture = Fixture::new();
    let mut params = fixture.params.clone();
    params.config_version = 0;
    expect_error(
        fixture.initialize_with_params(params),
        RemoteLegError::InvalidConfigVersion,
    );

    assert!(
        fixture
            .svm
            .get_account(&fixture.config)
            .is_none_or(|account| account.owner != solevm_remote_leg::ID)
    );

    fixture.initialize().expect("a valid setup still succeeds");
    assert_eq!(fixture.config().config_version, common::CONFIG_VERSION);
}
