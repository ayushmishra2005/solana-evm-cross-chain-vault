//! Sending attributed custody into the adapter through a real CPI.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use common::{ALLOCATE_TRANSFER_ID, Fixture, Pubkey, expect_error, expect_rejection};
use solana_signer::Signer;
use solevm_remote_leg::RemoteLegError;

const ID: [u8; 32] = ALLOCATE_TRANSFER_ID;
const AMOUNT: u64 = 1_000_000;

fn funded() -> Fixture {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ID, AMOUNT);
    fixture
}

#[test]
fn a_valid_deposit_moves_exactly_the_requested_amount() {
    let mut fixture = funded();
    fixture.deploy(AMOUNT).expect("deposit lands");

    assert_eq!(fixture.token_amount(fixture.custody), 0);
    assert_eq!(fixture.token_amount(fixture.adapter_vault), AMOUNT);
    assert_eq!(fixture.adapter().principal, AMOUNT);

    let position = fixture.position();
    assert_eq!(position.attributed_principal, 0);
    assert_eq!(position.deployed_principal, AMOUNT);
    assert_eq!(position.unattributed_custody, 0);
}

#[test]
fn the_adapter_principal_matches_the_deployed_principal() {
    let mut fixture = funded();
    fixture.deploy(400_000).expect("first deposit lands");
    assert_eq!(
        fixture.adapter().principal,
        fixture.position().deployed_principal
    );

    fixture.deploy(600_000).expect("second deposit lands");
    assert_eq!(
        fixture.adapter().principal,
        fixture.position().deployed_principal
    );
    assert_eq!(fixture.position().deployed_principal, AMOUNT);
}

#[test]
fn a_request_above_the_attributed_principal_is_bounded_by_it() {
    let mut fixture = funded();
    fixture.deploy(u64::MAX).expect("deposit lands");

    assert_eq!(fixture.position().deployed_principal, AMOUNT);
    assert_eq!(fixture.position().attributed_principal, 0);
}

#[test]
fn a_deposit_without_attributed_principal_is_rejected() {
    let mut fixture = Fixture::deployed();
    expect_error(
        fixture.deploy(100),
        RemoteLegError::InsufficientAttributedCustody,
    );
}

#[test]
fn unattributed_custody_cannot_be_deployed() {
    let mut fixture = Fixture::deployed();
    fixture.credit(fixture.custody, 5_000);
    fixture.reconcile().expect("reconciliation lands");

    assert_eq!(fixture.position().unattributed_custody, 5_000);
    expect_error(
        fixture.deploy(5_000),
        RemoteLegError::InsufficientAttributedCustody,
    );
    assert_eq!(fixture.token_amount(fixture.adapter_vault), 0);
}

#[test]
fn custody_beyond_the_attributed_principal_stays_behind() {
    let mut fixture = funded();
    fixture.credit(fixture.custody, 250);
    fixture.deploy(u64::MAX).expect("deposit lands");

    assert_eq!(fixture.token_amount(fixture.custody), 250);
    assert_eq!(fixture.position().unattributed_custody, 250);
    assert_eq!(fixture.position().deployed_principal, AMOUNT);
}

#[test]
fn a_paused_adapter_rejects_the_deposit_without_moving_assets() {
    let mut fixture = funded();
    fixture
        .configure_adapter(u64::MAX, 0, true)
        .expect("pause lands");

    expect_rejection(fixture.deploy(AMOUNT));
    assert_eq!(fixture.token_amount(fixture.custody), AMOUNT);
    assert_eq!(fixture.position().attributed_principal, AMOUNT);
    assert_eq!(fixture.adapter().principal, 0);
}

#[test]
fn a_frozen_leg_rejects_a_new_deposit() {
    let mut fixture = funded();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("leg freezes");

    expect_error(fixture.deploy(AMOUNT), RemoteLegError::Frozen);
}

// Only the configured adapter may be reached

#[test]
fn another_adapter_program_is_rejected() {
    let mut fixture = funded();
    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_program = spl_token::ID;
    expect_error(
        fixture.deploy_with(accounts, AMOUNT),
        RemoteLegError::InvalidAdapterProgram,
    );
}

#[test]
fn another_adapter_state_is_rejected() {
    let mut fixture = funded();
    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_state = Pubkey::new_unique();
    expect_error(
        fixture.deploy_with(accounts, AMOUNT),
        RemoteLegError::InvalidAdapterState,
    );
}

#[test]
fn another_adapter_authority_is_rejected() {
    let mut fixture = funded();
    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_authority = Pubkey::new_unique();
    expect_error(
        fixture.deploy_with(accounts, AMOUNT),
        RemoteLegError::InvalidAdapterAuthority,
    );
}

#[test]
fn another_adapter_vault_is_rejected() {
    let mut fixture = funded();
    let stranger = Pubkey::new_unique();
    let mint = fixture.mint;
    let authority = fixture.adapter_authority;
    fixture.write_token_account(stranger, mint, authority, None, None);

    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_token_vault = stranger;
    expect_error(
        fixture.deploy_with(accounts, AMOUNT),
        RemoteLegError::InvalidAdapterVault,
    );
}

#[test]
fn another_custody_account_is_rejected() {
    let mut fixture = funded();
    let stranger = Pubkey::new_unique();
    let mint = fixture.mint;
    let authority = fixture.custody_authority;
    fixture.write_token_account(stranger, mint, authority, None, None);

    let accounts = fixture.strategy_accounts();
    let mut instruction = fixture.deploy_instruction(accounts, AMOUNT);
    // The custody account sits after the three fixed configuration accounts.
    instruction.accounts[4] = AccountMeta::new(stranger, false);

    let payer = fixture.outsider.insecure_clone();
    expect_error(
        fixture.send_as(instruction, &payer, &[&payer]),
        RemoteLegError::InvalidCustodyAccount,
    );
}

#[test]
fn another_mint_is_rejected() {
    let mut fixture = funded();
    let other_mint = Pubkey::new_unique();
    fixture.write_mint(other_mint, 6);

    let mut accounts = fixture.strategy_accounts();
    accounts.asset_mint = other_mint;
    expect_error(
        fixture.deploy_with(accounts, AMOUNT),
        RemoteLegError::InvalidMint,
    );
}

#[test]
fn another_token_program_is_rejected() {
    let mut fixture = funded();
    let mut accounts = fixture.strategy_accounts();
    accounts.token_program = Pubkey::new_unique();
    expect_rejection(fixture.deploy_with(accounts, AMOUNT));
}

#[test]
fn the_adapter_refuses_a_caller_that_is_not_the_remote_leg() {
    let mut fixture = funded();
    let outsider = fixture.outsider.insecure_clone();

    let metas = solevm_test_strategy::accounts::DepositExact {
        remote_custody_authority: outsider.pubkey(),
        adapter_state: fixture.adapter_state,
        remote_custody: fixture.custody,
        adapter_token_vault: fixture.adapter_vault,
        mint: fixture.mint,
        token_program: spl_token::ID,
    }
    .to_account_metas(None);

    let instruction = Instruction {
        program_id: solevm_test_strategy::ID,
        accounts: metas,
        data: solevm_test_strategy::instruction::DepositExact { amount: AMOUNT }.data(),
    };

    expect_rejection(fixture.send_as(instruction, &outsider, &[&outsider]));
    assert_eq!(fixture.token_amount(fixture.custody), AMOUNT);
}

#[test]
fn a_deposit_sent_to_an_arbitrary_program_is_rejected() {
    let mut fixture = funded();
    let impostor = Pubkey::new_unique();
    let data = fixture.raw_data(fixture.adapter_state);
    fixture.write_owned_account(impostor, data, solevm_test_strategy::ID);

    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_state = impostor;
    expect_error(
        fixture.deploy_with(accounts, AMOUNT),
        RemoteLegError::InvalidAdapterState,
    );
}

#[test]
fn a_stale_position_cannot_survive_a_rejected_deposit() {
    let mut fixture = funded();
    let position_before = fixture.raw_data(fixture.position_key());

    fixture
        .configure_adapter(u64::MAX, 0, true)
        .expect("pause lands");
    expect_rejection(fixture.deploy(AMOUNT));

    assert_eq!(fixture.raw_data(fixture.position_key()), position_before);
}

#[test]
fn a_deposit_of_zero_is_rejected() {
    let mut fixture = funded();
    expect_error(
        fixture.deploy(0),
        RemoteLegError::InsufficientAttributedCustody,
    );
}

#[test]
fn the_custody_identity_holds_after_every_deposit() {
    let mut fixture = funded();
    fixture.credit(fixture.custody, 111);

    for amount in [1u64, 999, 500_000, u64::MAX] {
        let _ = fixture.deploy(amount);
        let position = fixture.position();
        assert_eq!(
            fixture.token_amount(fixture.custody),
            position.attributed_principal
                + position.recalled_custody
                + position.unattributed_custody
        );
        assert_eq!(fixture.adapter().principal, position.deployed_principal);
    }
}
