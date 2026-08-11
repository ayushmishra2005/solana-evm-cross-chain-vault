//! Setting up the one outbound report lane.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::{Fixture, REPORT_LANE_ID, expect_error, expect_rejection};
use solana_signer::Signer;
use solevm_remote_leg::{OutboundLane, RemoteLegError};

#[test]
fn a_new_lane_starts_at_the_first_sequence_with_no_history() {
    let mut fixture = Fixture::deployed();
    fixture
        .initialize_outbound_state()
        .expect("outbound state initializes");

    let lane = fixture.outbound_lane();
    assert_eq!(lane.state_version, 1);
    assert_eq!(lane.lane_id, REPORT_LANE_ID);
    assert_eq!(lane.next_sequence, 1);
    assert_eq!(lane.current_commitment, [0u8; 32]);
    assert_eq!(lane.last_publication_at, 0);
    assert_eq!(lane.total_reports, 0);
    assert_eq!(lane.total_recall_notifications, 0);
    assert!(lane.initialized_at > 0);
}

#[test]
fn the_stored_bump_is_the_canonical_one() {
    let mut fixture = Fixture::deployed();
    fixture
        .initialize_outbound_state()
        .expect("outbound state initializes");

    let (_, bump) = OutboundLane::address(&fixture.config, REPORT_LANE_ID);
    assert_eq!(fixture.outbound_lane().bump, bump);
}

#[test]
fn a_second_initialization_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture
        .initialize_outbound_state()
        .expect("outbound state initializes");

    expect_rejection(fixture.initialize_outbound_state());
}

#[test]
fn a_stranger_cannot_create_the_lane() {
    let mut fixture = Fixture::deployed();
    let outsider = fixture.outsider.pubkey();
    let lane = fixture.outbound_lane_key();

    expect_error(
        fixture.initialize_outbound_state_with(outsider, fixture.config, lane),
        RemoteLegError::Unauthorized,
    );
}

#[test]
fn the_guardian_cannot_create_the_lane() {
    let mut fixture = Fixture::deployed();
    let guardian = fixture.guardian.pubkey();
    let lane = fixture.outbound_lane_key();

    expect_error(
        fixture.initialize_outbound_state_with(guardian, fixture.config, lane),
        RemoteLegError::Unauthorized,
    );
}

#[test]
fn the_transport_verifier_cannot_create_the_lane() {
    let mut fixture = Fixture::deployed();
    let verifier = fixture.verifier.pubkey();
    let lane = fixture.outbound_lane_key();

    expect_error(
        fixture.initialize_outbound_state_with(verifier, fixture.config, lane),
        RemoteLegError::Unauthorized,
    );
}

#[test]
fn a_frozen_leg_refuses_to_create_the_lane() {
    let mut fixture = Fixture::deployed();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("the leg freezes");

    expect_error(fixture.initialize_outbound_state(), RemoteLegError::Frozen);
}

#[test]
fn a_lane_at_the_wrong_address_is_rejected() {
    let mut fixture = Fixture::deployed();
    let administrator = fixture.administrator.pubkey();
    let wrong = OutboundLane::address(&fixture.config, REPORT_LANE_ID + 1).0;

    expect_rejection(fixture.initialize_outbound_state_with(administrator, fixture.config, wrong));
}

#[test]
fn a_lane_derived_from_another_configuration_is_rejected() {
    let mut fixture = Fixture::deployed();
    let administrator = fixture.administrator.pubkey();
    let stranger = Fixture::position_address(&common::Pubkey::new_unique());
    let wrong = OutboundLane::address(&stranger, REPORT_LANE_ID).0;

    expect_rejection(fixture.initialize_outbound_state_with(administrator, fixture.config, wrong));
}

#[test]
fn creating_the_lane_moves_no_assets_and_changes_no_position() {
    let mut fixture = Fixture::deployed();
    fixture.credit(fixture.custody, 5_000);
    let before = fixture.position();
    let custody_before = fixture.token_amount(fixture.custody);

    fixture
        .initialize_outbound_state()
        .expect("outbound state initializes");

    let after = fixture.position();
    assert_eq!(after.attributed_principal, before.attributed_principal);
    assert_eq!(after.deployed_principal, before.deployed_principal);
    assert_eq!(after.recalled_custody, before.recalled_custody);
    assert_eq!(after.unattributed_custody, before.unattributed_custody);
    assert_eq!(fixture.token_amount(fixture.custody), custody_before);
}

#[test]
fn creating_the_lane_leaves_the_configuration_alone() {
    let mut fixture = Fixture::deployed();
    let before = fixture.config();

    fixture
        .initialize_outbound_state()
        .expect("outbound state initializes");

    let after = fixture.config();
    assert_eq!(after.config_version, before.config_version);
    assert_eq!(after.report_lane_id, before.report_lane_id);
    assert_eq!(after.frozen, before.frozen);
    assert_eq!(after.administrator, before.administrator);
}

#[test]
fn a_leg_without_a_lane_cannot_publish() {
    let mut fixture = Fixture::deployed();
    assert!(!fixture.outbound_lane_exists());

    let accounts = fixture.outbound_accounts(1);
    expect_rejection(fixture.publish_report_with(accounts, 1));
}
