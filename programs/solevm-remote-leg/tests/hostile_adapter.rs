//! A strategy adapter that turns on the leg can neither take custody nor
//! misreport what it holds, and every attempt leaves no trace.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_spl::token::spl_token::error::TokenError;
use common::{
    ALLOCATE_TRANSFER_ID, Fixture, Pubkey, RECALL_TRANSFER_ID, expect_custom_code, expect_error,
};
use solevm_remote_leg::RemoteLegError;
use solevm_test_strategy::attack;

const AMOUNT: u64 = 1_000_000;

/// Principal deployed, nothing recalled yet, the hostile adapter in place.
fn funded() -> Fixture {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.install_hostile_adapter();
    fixture
}

/// All principal inside the hostile adapter with a recall open against it.
fn recalling() -> Fixture {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("deposit lands");
    fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    fixture.install_hostile_adapter();
    fixture
}

/// Every account an adapter call could touch.
fn touched(fixture: &Fixture) -> Vec<Vec<u8>> {
    let keys: [Pubkey; 5] = [
        fixture.custody,
        fixture.adapter_vault,
        fixture.adapter_state,
        fixture.position_key(),
        fixture.transfer_key(&RECALL_TRANSFER_ID),
    ];
    keys.iter().map(|key| fixture.raw_data(*key)).collect()
}

/// The token program refuses the caller signature, so custody stays put even
/// while it holds assets the attack could take.
#[track_caller]
fn custody_attack_is_refused(code: u64) {
    let mut fixture = recalling();
    let custody = fixture.custody;
    fixture.credit(custody, AMOUNT);
    let before = touched(&fixture);
    expect_custom_code(
        fixture.withdraw(RECALL_TRANSFER_ID, code),
        TokenError::OwnerMismatch as u32,
    );
    assert_eq!(touched(&fixture), before);
}

#[test]
fn an_adapter_cannot_approve_a_custody_delegate() {
    custody_attack_is_refused(attack::APPROVE_DELEGATE);
}

#[test]
fn an_adapter_cannot_take_ownership_of_custody() {
    custody_attack_is_refused(attack::TAKE_OWNERSHIP);
}

#[test]
fn an_adapter_cannot_name_a_custody_close_authority() {
    custody_attack_is_refused(attack::SET_CLOSE_AUTHORITY);
}

#[test]
fn an_adapter_cannot_pull_assets_out_of_custody() {
    custody_attack_is_refused(attack::DRAIN_CUSTODY);
}

#[test]
fn an_adapter_cannot_return_more_than_it_unwound() {
    let mut fixture = recalling();
    let before = touched(&fixture);
    expect_error(
        fixture.withdraw(RECALL_TRANSFER_ID, attack::OVERPAY),
        RemoteLegError::InvalidRealizedLoss,
    );
    assert_eq!(touched(&fixture), before);
}

#[test]
fn an_adapter_cannot_overstate_the_principal_it_received() {
    let mut fixture = funded();
    let before = touched(&fixture);
    expect_error(
        fixture.deploy(attack::OVERSTATE_PRINCIPAL),
        RemoteLegError::InvalidPrincipalDelta,
    );
    assert_eq!(touched(&fixture), before);
}

#[test]
fn the_hostile_build_behaves_for_any_other_amount() {
    let mut fixture = recalling();
    fixture
        .withdraw(RECALL_TRANSFER_ID, 1_000)
        .expect("an ordinary withdrawal lands");
    assert_eq!(fixture.adapter_principal(), AMOUNT - 1_000);
    assert_eq!(fixture.position().recalled_custody, 1_000);
}
