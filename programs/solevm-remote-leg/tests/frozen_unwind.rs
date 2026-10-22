//! A frozen leg takes on no new exposure but still lets an accepted recall out,
//! and releases any other principal only with the guardian's co-signature.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::{
    ALLOCATE_TRANSFER_ID, Fixture, MAX_REMOTE_PRINCIPAL, RECALL_TRANSFER_ID, expect_error,
};
use solana_signer::Signer;
use solevm_remote_leg::{RemoteLegError, TransferStatus};

const AMOUNT: u64 = 1_000_000;

/// A funded leg with a live recall, frozen right after acceptance.
fn frozen_with_open_recall() -> Fixture {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("deposit lands");
    fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);

    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");
    fixture
}

// New exposure

#[test]
fn a_frozen_leg_takes_no_new_strategy_state() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    expect_error(
        fixture.initialize_strategy_state(MAX_REMOTE_PRINCIPAL),
        RemoteLegError::Frozen,
    );
}

#[test]
fn a_frozen_leg_takes_no_new_allocation() {
    let mut fixture = Fixture::deployed();
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    let bytes = fixture.allocate_bytes(ALLOCATE_TRANSFER_ID, AMOUNT, 1);
    expect_error(
        fixture.allocate(ALLOCATE_TRANSFER_ID, 1, bytes),
        RemoteLegError::Frozen,
    );
}

#[test]
fn a_frozen_leg_attributes_nothing_without_the_guardian() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.credit(fixture.custody, AMOUNT);

    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    expect_error(
        fixture.attribute(ALLOCATE_TRANSFER_ID),
        RemoteLegError::Frozen,
    );
    assert_eq!(fixture.position().attributed_principal, 0);
}

#[test]
fn a_frozen_leg_deploys_nothing_further() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);

    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    expect_error(fixture.deploy(AMOUNT), RemoteLegError::Frozen);
    assert_eq!(fixture.position().deployed_principal, 0);
}

#[test]
fn a_frozen_leg_takes_no_recall_without_the_guardian() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);

    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    let bytes = fixture.recall_bytes(RECALL_TRANSFER_ID, AMOUNT, 1);
    expect_error(
        fixture.recall(RECALL_TRANSFER_ID, 1, bytes),
        RemoteLegError::Frozen,
    );
}

// Safe unwind

#[test]
fn a_frozen_leg_still_reconciles_custody() {
    let mut fixture = frozen_with_open_recall();
    fixture.credit(fixture.custody, 500);
    fixture.reconcile().expect("reconciliation lands");
    assert_eq!(fixture.position().unattributed_custody, 500);
}

#[test]
fn a_frozen_leg_still_unwinds_an_accepted_recall() {
    let mut fixture = frozen_with_open_recall();

    fixture
        .withdraw(RECALL_TRANSFER_ID, AMOUNT)
        .expect("withdrawal lands while frozen");
    assert_eq!(fixture.position().deployed_principal, 0);

    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("send lands while frozen");

    let record = fixture.transfer(&RECALL_TRANSFER_ID);
    assert_eq!(record.status, TransferStatus::Complete);
    assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);
}

#[test]
fn a_frozen_leg_still_closes_a_consumed_record() {
    let mut fixture = Fixture::deployed();
    fixture.accept_updates(6);

    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");
    fixture
        .advance_watermark(&administrator, 3)
        .expect("watermark advances while frozen");

    let record = fixture.record_key(1);
    let instruction = fixture.close_record_instruction(
        administrator.pubkey(),
        fixture.control_lane_key(),
        record,
    );
    let outsider = fixture.outsider.insecure_clone();
    fixture
        .send_as(instruction, &outsider, &[&outsider])
        .expect("record closes while frozen");
}

#[test]
fn a_freeze_during_an_unwind_never_strands_the_assets() {
    let mut fixture = frozen_with_open_recall();
    fixture
        .configure_adapter(AMOUNT / 3, 0, false)
        .expect("the adapter takes the test conditions");

    while fixture.position().deployed_principal > 0 {
        fixture
            .withdraw(RECALL_TRANSFER_ID, AMOUNT)
            .expect("withdrawal lands while frozen");
    }
    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("send lands while frozen");

    assert_eq!(fixture.token_amount(fixture.custody), 0);
    assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);
    assert_eq!(
        fixture.transfer(&RECALL_TRANSFER_ID).status,
        TransferStatus::Complete
    );
}

// Guardian release

/// A funded leg with its principal deployed, frozen before any recall.
fn frozen_with_deployed_principal() -> Fixture {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("deposit lands");

    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");
    fixture
}

/// The next recall on the control lane, ready to send.
fn next_recall(fixture: &Fixture) -> (u64, Vec<u8>) {
    let sequence = fixture.control_lane().highest_consumed_sequence + 1;
    let bytes = fixture.recall_bytes(RECALL_TRANSFER_ID, AMOUNT, sequence);
    (sequence, bytes)
}

#[test]
fn the_guardian_releases_principal_from_a_frozen_leg() {
    let mut fixture = frozen_with_deployed_principal();
    let guardian = fixture.guardian.insecure_clone();

    let (sequence, bytes) = next_recall(&fixture);
    fixture
        .recall_cosigned(RECALL_TRANSFER_ID, sequence, bytes, &guardian)
        .expect("a co-signed recall lands while frozen");
    fixture
        .withdraw(RECALL_TRANSFER_ID, AMOUNT)
        .expect("withdrawal lands while frozen");
    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("send lands while frozen");

    let record = fixture.transfer(&RECALL_TRANSFER_ID);
    assert_eq!(record.status, TransferStatus::Complete);
    assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);
    assert!(fixture.config().frozen);
}

#[test]
fn only_the_guardian_can_cosign_a_frozen_recall() {
    let mut fixture = frozen_with_deployed_principal();
    let administrator = fixture.administrator.insecure_clone();

    let (sequence, bytes) = next_recall(&fixture);
    expect_error(
        fixture.recall_cosigned(RECALL_TRANSFER_ID, sequence, bytes, &administrator),
        RemoteLegError::Unauthorized,
    );
    assert_eq!(
        fixture.control_lane().highest_consumed_sequence,
        sequence - 1
    );
    assert_eq!(fixture.position().deployed_principal, AMOUNT);
}

#[test]
fn the_guardian_lets_a_pre_freeze_allocation_settle_and_return() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);
    let custody = fixture.custody;
    fixture.credit(custody, AMOUNT);
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    let guardian = fixture.guardian.insecure_clone();
    fixture
        .attribute_cosigned(ALLOCATE_TRANSFER_ID, &guardian)
        .expect("a co-signed attribution lands while frozen");
    assert_eq!(fixture.position().attributed_principal, AMOUNT);

    let (sequence, bytes) = next_recall(&fixture);
    fixture
        .recall_cosigned(RECALL_TRANSFER_ID, sequence, bytes, &guardian)
        .expect("a co-signed recall lands while frozen");
    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("send lands while frozen");
    assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);
}

#[test]
fn only_the_guardian_can_cosign_a_frozen_attribution() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);
    let custody = fixture.custody;
    fixture.credit(custody, AMOUNT);
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    expect_error(
        fixture.attribute_cosigned(ALLOCATE_TRANSFER_ID, &administrator),
        RemoteLegError::Unauthorized,
    );
    assert_eq!(fixture.position().attributed_principal, 0);
}
