//! Setup of the risk configuration and the three replay lanes.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::error::ErrorCode;
use solana_signer::Signer;
use solevm_remote_leg::{MessageClass, RemoteLegError, STATE_VERSION};

use common::{
    CONFIG_VERSION, CONTROL_LANE_ID, Fixture, INITIAL_CONFIG_COMMITMENT,
    MAX_DOWNWARD_DEVIATION_BPS, MAX_REMOTE_ALLOCATION_BPS, MAX_REPORT_AGE,
    MAX_UPWARD_DEVIATION_BPS, START_TIMESTAMP, WATERMARK_LAG, expect_anchor_error, expect_error,
    expect_rejection,
};

#[test]
fn control_state_initialization_writes_every_account() {
    let fixture = Fixture::ready();

    let risk = fixture.risk_config();
    assert_eq!(risk.state_version, STATE_VERSION);
    assert_eq!(risk.max_remote_allocation_bps, MAX_REMOTE_ALLOCATION_BPS);
    assert_eq!(risk.max_upward_deviation_bps, MAX_UPWARD_DEVIATION_BPS);
    assert_eq!(risk.max_downward_deviation_bps, MAX_DOWNWARD_DEVIATION_BPS);
    assert_eq!(risk.max_report_age, MAX_REPORT_AGE);
    assert_eq!(risk.config_commitment, INITIAL_CONFIG_COMMITMENT);
    assert_eq!(risk.initialized_at, START_TIMESTAMP);
    assert_eq!(risk.last_update_at, START_TIMESTAMP);
    assert_eq!(risk.reserved, [0u8; 32]);

    for class in MessageClass::ALL {
        let lane = fixture.lane(class);
        assert_eq!(lane.state_version, STATE_VERSION);
        assert_eq!(lane.message_class, class);
        assert_eq!(lane.lane_id, CONTROL_LANE_ID);
        assert_eq!(lane.minimum_acceptable_sequence, 1);
        assert_eq!(lane.highest_consumed_sequence, 0);
        assert_eq!(lane.message_commitment, [0u8; 32]);
        assert_eq!(lane.mandatory_watermark_lag, WATERMARK_LAG);
        assert_eq!(lane.last_accepted_at, 0);
    }
}

#[test]
fn the_risk_version_follows_the_configured_version() {
    let fixture = Fixture::ready();
    assert_eq!(fixture.risk_config().config_version, CONFIG_VERSION);
    assert_eq!(fixture.config().config_version, CONFIG_VERSION);
}

#[test]
fn the_three_lanes_live_at_three_addresses() {
    let fixture = Fixture::ready();
    let allocate = fixture.lane_key(MessageClass::Allocate);
    let recall = fixture.lane_key(MessageClass::Recall);
    let config_update = fixture.lane_key(MessageClass::ConfigUpdate);

    assert_ne!(allocate, recall);
    assert_ne!(recall, config_update);
    assert_ne!(allocate, config_update);
}

#[test]
fn control_state_initialization_runs_only_once() {
    let mut fixture = Fixture::ready();
    expect_rejection(fixture.initialize_control_state());
}

#[test]
fn an_uninitialized_leg_has_no_control_state() {
    let mut fixture = Fixture::new();
    expect_rejection(fixture.initialize_control_state());
}

#[test]
fn a_signer_that_is_not_the_administrator_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("leg initializes");

    let mut accounts = fixture.control_accounts();
    accounts.administrator = fixture.outsider.pubkey();
    expect_error(
        fixture.initialize_control_state_with(accounts, Fixture::control_params()),
        RemoteLegError::Unauthorized,
    );
}

#[test]
fn a_guardian_may_not_initialize_control_state() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("leg initializes");

    let guardian = fixture.guardian.insecure_clone();
    let mut accounts = fixture.control_accounts();
    accounts.administrator = guardian.pubkey();
    let instruction = fixture.control_state_instruction(accounts, Fixture::control_params());
    expect_rejection(fixture.send(instruction, &[&guardian]));
}

#[test]
fn a_configuration_from_another_vault_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("leg initializes");
    let other = fixture.add_second_vault([0x99; 32]);

    let mut accounts = fixture.control_accounts();
    accounts.remote_config = other;
    expect_rejection(fixture.initialize_control_state_with(accounts, Fixture::control_params()));
}

#[test]
fn a_risk_account_at_the_wrong_address_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("leg initializes");

    let mut accounts = fixture.control_accounts();
    accounts.risk_config =
        Fixture::risk_config_address(&Fixture::config_address(&[0x11; 32], &[0x99; 32]));
    expect_anchor_error(
        fixture.initialize_control_state_with(accounts, Fixture::control_params()),
        ErrorCode::ConstraintSeeds,
    );
}

#[test]
fn a_lane_account_at_the_wrong_address_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("leg initializes");

    let mut accounts = fixture.control_accounts();
    accounts.allocate_lane = Fixture::lane_address(&fixture.config, MessageClass::Allocate, 77);
    expect_anchor_error(
        fixture.initialize_control_state_with(accounts, Fixture::control_params()),
        ErrorCode::ConstraintSeeds,
    );
}

#[test]
fn repeating_one_class_for_two_lanes_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("leg initializes");

    let mut accounts = fixture.control_accounts();
    accounts.recall_lane = accounts.allocate_lane;
    expect_anchor_error(
        fixture.initialize_control_state_with(accounts, Fixture::control_params()),
        ErrorCode::ConstraintSeeds,
    );
}

#[test]
fn a_zero_watermark_lag_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("leg initializes");

    let mut params = Fixture::control_params();
    params.mandatory_watermark_lag = 0;
    expect_error(
        fixture.initialize_control_state_params(params),
        RemoteLegError::InvalidWatermark,
    );
}

#[test]
fn basis_points_above_ten_thousand_are_rejected() {
    let setters: [fn(&mut solevm_remote_leg::ControlStateParams); 3] = [
        |params| params.max_remote_allocation_bps = 10_001,
        |params| params.max_upward_deviation_bps = 10_001,
        |params| params.max_downward_deviation_bps = 10_001,
    ];

    for set in setters {
        let mut fixture = Fixture::new();
        fixture.initialize().expect("leg initializes");
        let mut params = Fixture::control_params();
        set(&mut params);
        expect_error(
            fixture.initialize_control_state_params(params),
            RemoteLegError::InvalidBasisPoints,
        );
    }
}

#[test]
fn a_zero_report_age_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("leg initializes");

    let mut params = Fixture::control_params();
    params.max_report_age = 0;
    expect_error(
        fixture.initialize_control_state_params(params),
        RemoteLegError::InvalidReportAge,
    );
}

#[test]
fn a_zero_config_commitment_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("leg initializes");

    let mut params = Fixture::control_params();
    params.config_commitment = [0u8; 32];
    expect_error(
        fixture.initialize_control_state_params(params),
        RemoteLegError::InvalidConfigCommitment,
    );
}

#[test]
fn a_rejected_initialization_leaves_no_account_behind() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("leg initializes");

    let mut params = Fixture::control_params();
    params.max_report_age = 0;
    expect_error(
        fixture.initialize_control_state_params(params),
        RemoteLegError::InvalidReportAge,
    );

    assert!(fixture.svm.get_account(&fixture.risk()).is_none());
    for class in MessageClass::ALL {
        assert!(fixture.svm.get_account(&fixture.lane_key(class)).is_none());
    }

    fixture
        .initialize_control_state()
        .expect("a valid setup still works afterwards");
}

#[test]
fn control_state_does_not_change_the_configuration_account() {
    let mut fixture = Fixture::new();
    fixture.initialize().expect("leg initializes");
    let before = fixture.raw_data(fixture.config);

    fixture
        .initialize_control_state()
        .expect("control state initializes");

    assert_eq!(fixture.raw_data(fixture.config), before);
}
