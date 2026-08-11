//! The adapter layout this program was built against is pinned.
//!
//! The leg reads one fixed offset and sends fixed selectors, which is only
//! safe while the adapter keeps exactly the layout checked here.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::{Discriminator, Space};
use common::{ALLOCATE_TRANSFER_ID, Fixture, Pubkey, expect_error};
use solevm_remote_leg::RemoteLegError;
use solevm_remote_leg::adapter;
use solevm_test_strategy::AdapterState;

const AMOUNT: u64 = 1_000_000;
const EPOCH: u64 = 12;

fn sample_state() -> AdapterState {
    AdapterState {
        state_version: 1,
        bump: 254,
        authority_bump: 253,
        deposits_paused: false,
        remote_leg_program: Pubkey::new_unique(),
        remote_config: Pubkey::new_unique(),
        mint: Pubkey::new_unique(),
        token_program: Pubkey::new_unique(),
        adapter_authority: Pubkey::new_unique(),
        adapter_token_vault: Pubkey::new_unique(),
        test_authority: Pubkey::new_unique(),
        principal: 0x0102_0304_0506_0708,
        max_liquid_principal: 42,
        loss_bps: 7,
        initialized_at: 99,
    }
}

fn serialized(state: &AdapterState) -> Vec<u8> {
    let mut bytes = AdapterState::DISCRIMINATOR.to_vec();
    anchor_lang::AnchorSerialize::serialize(state, &mut bytes).expect("state encodes");
    bytes
}

#[test]
fn the_configured_adapter_is_the_program_this_leg_was_built_against() {
    let fixture = Fixture::deployed();
    assert_eq!(
        fixture.strategy_config().adapter_program,
        solevm_test_strategy::ID
    );
}

#[test]
fn every_call_selector_matches_the_adapter_instruction() {
    assert_eq!(
        adapter::DEPOSIT_EXACT,
        solevm_test_strategy::instruction::DepositExact::DISCRIMINATOR
    );
    assert_eq!(
        adapter::WITHDRAW_FOR_REMOTE_LEG,
        solevm_test_strategy::instruction::WithdrawForRemoteLeg::DISCRIMINATOR
    );
}

#[test]
fn the_pinned_account_tag_matches_the_adapter() {
    assert_eq!(adapter::STATE_DISCRIMINATOR, AdapterState::DISCRIMINATOR);
}

#[test]
fn the_pinned_account_length_matches_the_adapter() {
    assert_eq!(adapter::STATE_LEN, 8 + AdapterState::INIT_SPACE);
    assert_eq!(serialized(&sample_state()).len(), adapter::STATE_LEN);
}

#[test]
fn the_pinned_offset_lands_on_the_principal_field() {
    let state = sample_state();
    let bytes = serialized(&state);

    let slot =
        &bytes[adapter::PRINCIPAL_OFFSET..adapter::PRINCIPAL_OFFSET + adapter::PRINCIPAL_LEN];
    assert_eq!(
        u64::from_le_bytes(slot.try_into().unwrap()),
        state.principal
    );
}

#[test]
fn the_pinned_offset_leaves_room_for_the_whole_field() {
    const { assert!(adapter::PRINCIPAL_OFFSET + adapter::PRINCIPAL_LEN <= adapter::STATE_LEN) };
}

#[test]
fn a_layout_that_moved_the_principal_is_detected() {
    let mut state = sample_state();
    state.principal = 1;
    let bytes = serialized(&state);

    // Any shift of the field breaks the read the leg relies on.
    let shifted = &bytes[adapter::PRINCIPAL_OFFSET - 1..adapter::PRINCIPAL_OFFSET + 7];
    assert_ne!(
        u64::from_le_bytes(shifted.try_into().unwrap()),
        state.principal
    );
}

#[test]
fn the_live_account_carries_the_pinned_layout() {
    let fixture = Fixture::deployed();
    let account = fixture
        .svm
        .get_account(&fixture.adapter_state)
        .expect("adapter state exists");

    assert_eq!(account.owner, solevm_test_strategy::ID);
    assert_eq!(account.data.len(), adapter::STATE_LEN);
    assert_eq!(&account.data[..8], adapter::STATE_DISCRIMINATOR);
}

#[test]
fn an_account_shorter_than_the_layout_is_refused() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.truncate_adapter_state(64);

    expect_error(
        fixture.publish_report(EPOCH),
        RemoteLegError::AdapterStateMismatch,
    );
}

#[test]
fn an_account_too_short_to_hold_the_principal_is_refused() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.truncate_adapter_state(adapter::PRINCIPAL_OFFSET + 1);

    expect_error(
        fixture.publish_report(EPOCH),
        RemoteLegError::AdapterStateMismatch,
    );
}

#[test]
fn an_account_owned_by_a_stranger_is_refused() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.reassign_adapter_state(Pubkey::new_unique());

    expect_error(
        fixture.publish_report(EPOCH),
        RemoteLegError::AdapterStateMismatch,
    );
}

#[test]
fn an_account_with_a_foreign_tag_is_refused() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.overwrite_adapter_tag([9u8; 8]);

    expect_error(
        fixture.publish_report(EPOCH),
        RemoteLegError::AdapterStateMismatch,
    );
}

#[test]
fn a_healthy_adapter_still_reports_its_principal() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("the deposit lands");

    assert_eq!(fixture.adapter_principal(), AMOUNT);
    fixture.publish_report(EPOCH).expect("the report lands");
}
