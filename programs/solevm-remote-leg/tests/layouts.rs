//! The account layouts earlier milestones settled on are frozen.
//!
//! Anything stored on chain keeps its exact size and field order, so an
//! upgrade never reinterprets bytes that already exist.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::{AnchorSerialize, Discriminator, Space};
use common::Pubkey;
use solevm_remote_leg::{
    ConsumedMessage, MessageClass, OutboundLane, RecallSentMarker, RemoteConfig, RemotePosition,
    ReplayLane, ReportRecord, RiskConfig, StrategyConfig, TransferKind, TransferRecord,
    TransferStatus,
};

/// Position of every byte a field occupies inside the account data.
///
/// The discriminator sits in front, so serialized offsets shift by eight.
const DISCRIMINATOR_LEN: usize = 8;

// Sizes that must never move

#[test]
fn the_configuration_keeps_its_size() {
    assert_eq!(RemoteConfig::INIT_SPACE, 484);
    assert_eq!(RemoteConfig::LEN, DISCRIMINATOR_LEN + 484);
}

#[test]
fn the_control_accounts_keep_their_sizes() {
    assert_eq!(RiskConfig::INIT_SPACE, 104);
    assert_eq!(RiskConfig::LEN, DISCRIMINATOR_LEN + 104);
    assert_eq!(ReplayLane::INIT_SPACE, 70);
    assert_eq!(ReplayLane::LEN, DISCRIMINATOR_LEN + 70);
    assert_eq!(ConsumedMessage::INIT_SPACE, 47);
    assert_eq!(ConsumedMessage::LEN, DISCRIMINATOR_LEN + 47);
}

#[test]
fn the_strategy_accounts_keep_their_sizes() {
    assert_eq!(StrategyConfig::INIT_SPACE, 178);
    assert_eq!(StrategyConfig::LEN, DISCRIMINATOR_LEN + 178);
    assert_eq!(RemotePosition::INIT_SPACE, 132);
    assert_eq!(RemotePosition::LEN, DISCRIMINATOR_LEN + 132);
    assert_eq!(TransferRecord::INIT_SPACE, 149);
    assert_eq!(TransferRecord::LEN, DISCRIMINATOR_LEN + 149);
}

#[test]
fn the_new_outbound_accounts_have_the_sizes_this_milestone_fixed() {
    assert_eq!(OutboundLane::INIT_SPACE, 78);
    assert_eq!(OutboundLane::LEN, DISCRIMINATOR_LEN + 78);
    assert_eq!(ReportRecord::INIT_SPACE, 193);
    assert_eq!(ReportRecord::LEN, DISCRIMINATOR_LEN + 193);
    assert_eq!(RecallSentMarker::INIT_SPACE, 82);
    assert_eq!(RecallSentMarker::LEN, DISCRIMINATOR_LEN + 82);
}

#[test]
fn every_account_stays_under_the_ten_kilobyte_ceiling() {
    for size in [
        RemoteConfig::LEN,
        RiskConfig::LEN,
        ReplayLane::LEN,
        ConsumedMessage::LEN,
        StrategyConfig::LEN,
        RemotePosition::LEN,
        TransferRecord::LEN,
        OutboundLane::LEN,
        ReportRecord::LEN,
        RecallSentMarker::LEN,
    ] {
        assert!(size <= 10_240, "account of {size} bytes is too large");
    }
}

// Field offsets that must never move

/// Serializes an account the way the runtime stores it.
fn stored<T: AnchorSerialize + Discriminator>(value: &T) -> Vec<u8> {
    let mut bytes = T::DISCRIMINATOR.to_vec();
    value.serialize(&mut bytes).expect("the account encodes");
    bytes
}

/// Finds where a marker value landed, proving one field offset.
#[track_caller]
fn offset_of(bytes: &[u8], marker: &[u8]) -> usize {
    let mut found = None;
    for start in 0..=bytes.len().saturating_sub(marker.len()) {
        if &bytes[start..start + marker.len()] == marker {
            assert!(found.is_none(), "the marker is not unique");
            found = Some(start);
        }
    }
    found.expect("the marker is stored")
}

fn sample_config() -> RemoteConfig {
    RemoteConfig {
        state_version: 1,
        bump: 254,
        custody_authority_bump: 253,
        frozen: false,
        administrator: Pubkey::new_unique(),
        emergency_guardian: Pubkey::new_unique(),
        transport_verifier: Pubkey::new_unique(),
        asset_mint: Pubkey::new_unique(),
        token_program: Pubkey::new_unique(),
        custody_authority: Pubkey::new_unique(),
        custody_token_account: Pubkey::new_unique(),
        outbound_escrow: Pubkey::new_unique(),
        source_chain_id: 0x1111_1111,
        destination_chain_id: 0x2222_2222,
        source_application_id: [0xA1; 32],
        local_application_id: [0xA2; 32],
        deployment_id: [0xA3; 32],
        vault_id: [0xA4; 32],
        control_lane_id: 0x3333_3333,
        report_lane_id: 0x4444_4444,
        config_version: 0x5555_5555_5555_5555,
        initialized_at: 0x6666_6666_6666_6666,
        reserved: [0u8; 64],
    }
}

fn sample_position() -> RemotePosition {
    RemotePosition {
        state_version: 1,
        bump: 254,
        attributed_principal: 0x1111_1111_1111_1111,
        deployed_principal: 0x2222_2222_2222_2222,
        recalled_custody: 0x3333_3333_3333_3333,
        unattributed_custody: 0x4444_4444_4444_4444,
        cumulative_realized_loss: 0x5555_5555_5555_5555,
        active_transfer_id: [0xB1; 32],
        active_transfer_kind: TransferKind::Recall,
        active_transfer_sequence: 0x6666_6666_6666_6666,
        active_transfer_status: TransferStatus::Pending,
        latest_completed_transfer_id: [0xB2; 32],
        latest_completion_at: 0x7777_7777_7777_7777,
        initialized_at: 0x0102_0304_0506_0708,
    }
}

fn sample_transfer() -> TransferRecord {
    TransferRecord {
        state_version: 1,
        bump: 254,
        transfer_kind: TransferKind::Recall,
        message_class: MessageClass::Recall,
        status: TransferStatus::Complete,
        transfer_id: [0xC1; 32],
        message_sequence: 0x1111_1111_1111_1111,
        authorized_amount: 0x2222_2222_2222_2222,
        attributed_amount: 0x3333_3333_3333_3333,
        requested_recall_amount: 0x4444_4444_4444_4444,
        minimum_amount: 0x5555_5555_5555_5555,
        custody_principal_reserved: 0x6666_6666_6666_6666,
        strategy_principal_resolved: 0x7777_7777_7777_7777,
        assets_withdrawn: 0x0102_0304_0506_0708,
        assets_sent: 0x1112_1314_1516_1718,
        realized_loss: 0x2122_2324_2526_2728,
        expected_source_balance: 0x3132_3334_3536_3738,
        created_at: 0x4142_4344_4546_4748,
        completed_at: 0x5152_5354_5556_5758,
    }
}

#[test]
fn the_configuration_keeps_its_field_order() {
    let config = sample_config();
    let bytes = stored(&config);

    assert_eq!(bytes.len(), RemoteConfig::LEN);
    assert_eq!(offset_of(&bytes, config.administrator.as_ref()), 12);
    assert_eq!(
        offset_of(&bytes, config.custody_token_account.as_ref()),
        204
    );
    assert_eq!(offset_of(&bytes, config.outbound_escrow.as_ref()), 236);
    assert_eq!(offset_of(&bytes, &config.deployment_id), 340);
    assert_eq!(offset_of(&bytes, &config.vault_id), 372);
    assert_eq!(offset_of(&bytes, &config.config_version.to_le_bytes()), 412);
}

#[test]
fn the_position_keeps_its_field_order() {
    let position = sample_position();
    let bytes = stored(&position);

    assert_eq!(bytes.len(), RemotePosition::LEN);
    assert_eq!(
        offset_of(&bytes, &position.attributed_principal.to_le_bytes()),
        10
    );
    assert_eq!(
        offset_of(&bytes, &position.deployed_principal.to_le_bytes()),
        18
    );
    assert_eq!(
        offset_of(&bytes, &position.recalled_custody.to_le_bytes()),
        26
    );
    assert_eq!(
        offset_of(&bytes, &position.unattributed_custody.to_le_bytes()),
        34
    );
    assert_eq!(
        offset_of(&bytes, &position.cumulative_realized_loss.to_le_bytes()),
        42
    );
    assert_eq!(offset_of(&bytes, &position.active_transfer_id), 50);
    assert_eq!(
        offset_of(&bytes, &position.latest_completed_transfer_id),
        92
    );
}

#[test]
fn the_replay_lane_keeps_its_field_order() {
    let lane = ReplayLane {
        state_version: 1,
        bump: 254,
        lane_id: 0x1111_1111,
        minimum_acceptable_sequence: 0x2222_2222_2222_2222,
        highest_consumed_sequence: 0x3333_3333_3333_3333,
        message_commitment: [0xA1; 32],
        mandatory_watermark_lag: 0x4444_4444_4444_4444,
        last_accepted_at: 0x5555_5555_5555_5555,
    };
    let bytes = stored(&lane);

    assert_eq!(bytes.len(), ReplayLane::LEN);
    assert_eq!(offset_of(&bytes, &lane.lane_id.to_le_bytes()), 10);
    assert_eq!(
        offset_of(&bytes, &lane.highest_consumed_sequence.to_le_bytes()),
        8 + ReplayLane::HIGHEST_CONSUMED_OFFSET
    );
    assert_eq!(offset_of(&bytes, &lane.message_commitment), 30);
    assert_eq!(
        offset_of(&bytes, &lane.mandatory_watermark_lag.to_le_bytes()),
        62
    );
}

#[test]
fn the_transfer_record_keeps_its_field_order() {
    let record = sample_transfer();
    let bytes = stored(&record);

    assert_eq!(bytes.len(), TransferRecord::LEN);
    assert_eq!(offset_of(&bytes, &record.transfer_id), 13);
    assert_eq!(
        offset_of(&bytes, &record.requested_recall_amount.to_le_bytes()),
        69
    );
    assert_eq!(offset_of(&bytes, &record.assets_sent.to_le_bytes()), 109);
    assert_eq!(offset_of(&bytes, &record.realized_loss.to_le_bytes()), 117);
    assert_eq!(offset_of(&bytes, &record.completed_at.to_le_bytes()), 149);
}

// The new accounts do not touch the reserved bytes of the old ones

#[test]
fn the_configuration_reserve_is_still_untouched() {
    let fixture = common::Fixture::ready_to_publish();
    assert_eq!(fixture.config().reserved, [0u8; 64]);
}

#[test]
fn publishing_never_writes_into_the_configuration_reserve() {
    let mut fixture = common::Fixture::ready_to_publish();
    fixture.publish_report(5).expect("the report lands");
    assert_eq!(fixture.config().reserved, [0u8; 64]);
}
