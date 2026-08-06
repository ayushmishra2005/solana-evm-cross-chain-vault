//! Setting up the adapter identity and the position.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::Discriminator;
use anchor_spl::token::spl_token;
use common::{Fixture, MAX_REMOTE_PRINCIPAL, Pubkey, expect_error};
use solana_signer::Signer;
use solevm_remote_leg::{RemoteLegError, TransferKind, TransferStatus};

// Interface parity with the adapter

#[test]
fn the_deposit_selector_matches_the_adapter_instruction() {
    assert_eq!(
        solevm_remote_leg::adapter::DEPOSIT_EXACT,
        solevm_test_strategy::instruction::DepositExact::DISCRIMINATOR
    );
}

#[test]
fn the_withdraw_selector_matches_the_adapter_instruction() {
    assert_eq!(
        solevm_remote_leg::adapter::WITHDRAW_FOR_REMOTE_LEG,
        solevm_test_strategy::instruction::WithdrawForRemoteLeg::DISCRIMINATOR
    );
}

#[test]
fn the_principal_offset_points_at_the_adapter_principal() {
    let state = solevm_test_strategy::AdapterState {
        state_version: 1,
        bump: 1,
        authority_bump: 2,
        deposits_paused: false,
        remote_leg_program: Pubkey::new_unique(),
        remote_config: Pubkey::new_unique(),
        mint: Pubkey::new_unique(),
        token_program: Pubkey::new_unique(),
        adapter_authority: Pubkey::new_unique(),
        adapter_token_vault: Pubkey::new_unique(),
        test_authority: Pubkey::new_unique(),
        principal: 0x0102_0304_0506_0708,
        max_liquid_principal: 0,
        loss_bps: 0,
        initialized_at: 0,
    };

    let mut bytes = solevm_test_strategy::AdapterState::DISCRIMINATOR.to_vec();
    anchor_lang::AnchorSerialize::serialize(&state, &mut bytes).expect("state encodes");

    let offset = solevm_remote_leg::adapter::PRINCIPAL_OFFSET;
    let slot = &bytes[offset..offset + solevm_remote_leg::adapter::PRINCIPAL_LEN];
    assert_eq!(
        u64::from_le_bytes(slot.try_into().unwrap()),
        state.principal
    );
}

// Initialization

#[test]
fn a_valid_setup_stores_the_adapter_and_an_empty_position() {
    let fixture = Fixture::deployed();

    let strategy = fixture.strategy_config();
    assert_eq!(strategy.adapter_program, solevm_test_strategy::ID);
    assert_eq!(strategy.adapter_state, fixture.adapter_state);
    assert_eq!(strategy.adapter_authority, fixture.adapter_authority);
    assert_eq!(strategy.adapter_token_vault, fixture.adapter_vault);
    assert_eq!(strategy.max_remote_principal, MAX_REMOTE_PRINCIPAL);

    let position = fixture.position();
    assert_eq!(position.attributed_principal, 0);
    assert_eq!(position.deployed_principal, 0);
    assert_eq!(position.recalled_custody, 0);
    assert_eq!(position.unattributed_custody, 0);
    assert_eq!(position.cumulative_realized_loss, 0);
    assert_eq!(position.active_transfer_kind, TransferKind::None);
    assert_eq!(position.active_transfer_status, TransferStatus::None);
    assert!(!position.has_active_transfer());
}

#[test]
fn a_second_setup_is_rejected() {
    let mut fixture = Fixture::deployed();
    assert!(
        fixture
            .initialize_strategy_state(MAX_REMOTE_PRINCIPAL)
            .is_err()
    );
}

#[test]
fn a_signer_that_is_not_the_administrator_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let mut accounts = fixture.strategy_accounts();
    accounts.administrator = fixture.outsider.pubkey();
    expect_error(
        fixture.initialize_strategy_state_with(accounts, MAX_REMOTE_PRINCIPAL),
        RemoteLegError::Unauthorized,
    );
}

#[test]
fn a_frozen_leg_cannot_gain_strategy_state() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("leg freezes");

    expect_error(
        fixture.initialize_strategy_state(MAX_REMOTE_PRINCIPAL),
        RemoteLegError::Frozen,
    );
}

#[test]
fn a_zero_principal_ceiling_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();
    expect_error(
        fixture.initialize_strategy_state(0),
        RemoteLegError::InvalidStrategyConfig,
    );
}

#[test]
fn an_adapter_program_that_is_not_executable_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_program = Pubkey::new_unique();
    fixture.prefund(accounts.adapter_program, 1_000_000);
    expect_error(
        fixture.initialize_strategy_state_with(accounts, MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidAdapterProgram,
    );
}

#[test]
fn the_remote_leg_cannot_be_its_own_adapter() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_program = solevm_remote_leg::ID;
    expect_error(
        fixture.initialize_strategy_state_with(accounts, MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidAdapterProgram,
    );
}

#[test]
fn an_adapter_state_owned_by_another_program_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();
    fixture.reassign_account(fixture.adapter_state, spl_token::ID);

    expect_error(
        fixture.initialize_strategy_state(MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidAdapterState,
    );
}

#[test]
fn an_adapter_state_at_the_wrong_address_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let stranger = Pubkey::new_unique();
    let data = fixture.raw_data(fixture.adapter_state);
    fixture.write_owned_account(stranger, data, solevm_test_strategy::ID);

    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_state = stranger;
    expect_error(
        fixture.initialize_strategy_state_with(accounts, MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidAdapterState,
    );
}

#[test]
fn an_adapter_authority_that_is_not_derived_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_authority = Pubkey::new_unique();
    expect_error(
        fixture.initialize_strategy_state_with(accounts, MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidAdapterAuthority,
    );
}

#[test]
fn an_adapter_vault_owned_by_another_authority_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let stranger = Pubkey::new_unique();
    let mint = fixture.mint;
    fixture.write_token_account(stranger, mint, Pubkey::new_unique(), None, None);

    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_token_vault = stranger;
    expect_error(
        fixture.initialize_strategy_state_with(accounts, MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidAdapterVault,
    );
}

#[test]
fn an_adapter_vault_of_another_mint_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let other_mint = Pubkey::new_unique();
    fixture.write_mint(other_mint, 6);
    let stranger = Pubkey::new_unique();
    let authority = fixture.adapter_authority;
    fixture.write_token_account(stranger, other_mint, authority, None, None);

    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_token_vault = stranger;
    expect_error(
        fixture.initialize_strategy_state_with(accounts, MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidAdapterVault,
    );
}

#[test]
fn an_adapter_vault_with_a_delegate_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let vault = fixture.adapter_vault;
    let mint = fixture.mint;
    let authority = fixture.adapter_authority;
    fixture.write_token_account(vault, mint, authority, Some(Pubkey::new_unique()), None);

    expect_error(
        fixture.initialize_strategy_state(MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidAdapterVault,
    );
}

#[test]
fn an_adapter_vault_with_a_close_authority_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let vault = fixture.adapter_vault;
    let mint = fixture.mint;
    let authority = fixture.adapter_authority;
    fixture.write_token_account(vault, mint, authority, None, Some(Pubkey::new_unique()));

    expect_error(
        fixture.initialize_strategy_state(MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidAdapterVault,
    );
}

#[test]
fn an_adapter_vault_equal_to_custody_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_token_vault = fixture.custody;
    expect_error(
        fixture.initialize_strategy_state_with(accounts, MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidAdapterVault,
    );
}

#[test]
fn an_adapter_vault_equal_to_the_outbound_escrow_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let escrow = fixture.escrow;
    let mint = fixture.mint;
    let authority = fixture.adapter_authority;
    fixture.write_token_account(escrow, mint, authority, None, None);

    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_token_vault = escrow;
    expect_error(
        fixture.initialize_strategy_state_with(accounts, MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidAdapterVault,
    );
}

#[test]
fn a_token_program_that_is_not_the_configured_one_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let mut accounts = fixture.strategy_accounts();
    accounts.token_program = Pubkey::new_unique();
    assert!(
        fixture
            .initialize_strategy_state_with(accounts, MAX_REMOTE_PRINCIPAL)
            .is_err()
    );
}

#[test]
fn a_mint_that_is_not_the_configured_one_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let other_mint = Pubkey::new_unique();
    fixture.write_mint(other_mint, 6);
    let mut accounts = fixture.strategy_accounts();
    accounts.asset_mint = other_mint;
    expect_error(
        fixture.initialize_strategy_state_with(accounts, MAX_REMOTE_PRINCIPAL),
        RemoteLegError::InvalidMint,
    );
}

#[test]
fn a_rejected_setup_leaves_no_strategy_state_behind() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();

    let strategy = fixture.strategy();
    let position = fixture.position_key();
    assert!(fixture.initialize_strategy_state(0).is_err());

    assert!(
        fixture
            .svm
            .get_account(&strategy)
            .is_none_or(|a| a.data.is_empty())
    );
    assert!(
        fixture
            .svm
            .get_account(&position)
            .is_none_or(|a| a.data.is_empty())
    );
}
