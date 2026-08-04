//! The one way emergency stop.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::{Fixture, expect_error};
use solevm_remote_leg::{RemoteConfig, RemoteLegError};

fn initialized() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("initialize succeeds");
    fixture
}

#[test]
fn the_administrator_may_freeze() {
    let mut fixture = initialized();
    assert!(!fixture.config().frozen);

    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze succeeds");

    assert!(fixture.config().frozen);
}

#[test]
fn the_guardian_may_freeze() {
    let mut fixture = initialized();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("freeze succeeds");

    assert!(fixture.config().frozen);
}

#[test]
fn any_other_signer_may_not_freeze() {
    let mut fixture = initialized();
    let outsider = fixture.outsider.insecure_clone();
    expect_error(fixture.freeze(&outsider), RemoteLegError::Unauthorized);
    assert!(!fixture.config().frozen);
}

#[test]
fn the_transport_verifier_may_not_freeze() {
    let mut fixture = initialized();
    let verifier = fixture.verifier_keypair();
    fixture.fund(&verifier);
    expect_error(fixture.freeze(&verifier), RemoteLegError::Unauthorized);
    assert!(!fixture.config().frozen);
}

#[test]
fn freezing_twice_is_rejected() {
    let mut fixture = initialized();
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze succeeds");

    expect_error(
        fixture.freeze(&administrator),
        RemoteLegError::AlreadyFrozen,
    );
    assert!(fixture.config().frozen);
}

#[test]
fn the_guardian_may_not_freeze_again_after_the_administrator() {
    let mut fixture = initialized();
    let administrator = fixture.administrator.insecure_clone();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&administrator).expect("freeze succeeds");

    expect_error(fixture.freeze(&guardian), RemoteLegError::AlreadyFrozen);
}

#[test]
fn freezing_leaves_the_custody_balance_alone() {
    let mut fixture = initialized();
    let before = fixture.token_amount(fixture.custody);
    let escrow_before = fixture.token_amount(fixture.escrow);

    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze succeeds");

    assert_eq!(fixture.token_amount(fixture.custody), before);
    assert_eq!(fixture.token_amount(fixture.escrow), escrow_before);
}

#[test]
fn freezing_changes_no_field_other_than_the_frozen_flag() {
    let mut fixture = initialized();
    let before = fixture.config();

    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze succeeds");
    let after = fixture.config();

    assert!(!before.frozen);
    assert!(after.frozen);
    assert_eq!(after.state_version, before.state_version);
    assert_eq!(after.bump, before.bump);
    assert_eq!(after.custody_authority_bump, before.custody_authority_bump);
    assert_eq!(after.administrator, before.administrator);
    assert_eq!(after.emergency_guardian, before.emergency_guardian);
    assert_eq!(after.transport_verifier, before.transport_verifier);
    assert_eq!(after.asset_mint, before.asset_mint);
    assert_eq!(after.token_program, before.token_program);
    assert_eq!(after.custody_authority, before.custody_authority);
    assert_eq!(after.custody_token_account, before.custody_token_account);
    assert_eq!(after.outbound_escrow, before.outbound_escrow);
    assert_eq!(after.source_chain_id, before.source_chain_id);
    assert_eq!(after.destination_chain_id, before.destination_chain_id);
    assert_eq!(after.source_application_id, before.source_application_id);
    assert_eq!(after.local_application_id, before.local_application_id);
    assert_eq!(after.deployment_id, before.deployment_id);
    assert_eq!(after.vault_id, before.vault_id);
    assert_eq!(after.control_lane_id, before.control_lane_id);
    assert_eq!(after.report_lane_id, before.report_lane_id);
    assert_eq!(after.config_version, before.config_version);
    assert_eq!(after.initialized_at, before.initialized_at);
    assert_eq!(after.reserved, before.reserved);
}

#[test]
fn freezing_keeps_the_account_size() {
    let mut fixture = initialized();
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze succeeds");

    let account = fixture.svm.get_account(&fixture.config).unwrap();
    assert_eq!(account.data.len(), RemoteConfig::LEN);
    assert_eq!(account.owner, solevm_remote_leg::ID);
}

#[test]
fn a_rejected_freeze_leaves_the_stored_state_unchanged() {
    let mut fixture = initialized();
    let before = fixture.svm.get_account(&fixture.config).unwrap().data;

    let outsider = fixture.outsider.insecure_clone();
    expect_error(fixture.freeze(&outsider), RemoteLegError::Unauthorized);

    let after = fixture.svm.get_account(&fixture.config).unwrap().data;
    assert_eq!(after, before);
}

#[test]
fn freezing_one_vault_leaves_another_vault_open() {
    let mut fixture = initialized();
    let second = fixture.add_second_vault([0x66u8; 32]);

    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze succeeds");

    assert!(fixture.config().frozen);
    assert!(!fixture.config_at(second).frozen);
}
