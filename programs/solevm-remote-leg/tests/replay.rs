//! Consumed records, replay watermarks and safe record closure.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::AccountDeserialize;
use anchor_lang::error::ErrorCode;
use solana_signer::Signer;
use solevm_remote_leg::{ConsumedMessage, MessageClass, RemoteLegError};

use common::messages::MessageBuilder;
use common::{
    CONTROL_LANE_ID, Fixture, WATERMARK_LAG, expect_anchor_error, expect_error, expect_rejection,
};

/// The record address for a bump below the canonical one.
fn non_canonical_record(fixture: &Fixture, sequence: u64) -> Pubkey {
    let (_, canonical) = Fixture::record_address(
        &fixture.config,
        MessageClass::ConfigUpdate,
        CONTROL_LANE_ID,
        sequence,
    );
    let seeds = [
        solevm_remote_leg::CONSUMED_MESSAGE_SEED.to_vec(),
        fixture.config.as_ref().to_vec(),
        vec![MessageClass::ConfigUpdate.to_u8()],
        CONTROL_LANE_ID.to_le_bytes().to_vec(),
        sequence.to_le_bytes().to_vec(),
    ];

    for bump in (0..canonical).rev() {
        let tail = [bump];
        let parts: Vec<&[u8]> = seeds
            .iter()
            .map(Vec::as_slice)
            .chain(std::iter::once(&tail[..]))
            .collect();
        if let Ok(address) = Pubkey::create_program_address(&parts, &solevm_remote_leg::ID) {
            return address;
        }
    }
    panic!("no lower bump produces a valid address");
}

use anchor_lang::prelude::Pubkey;

// Record creation

#[test]
fn an_absent_record_address_is_created_and_written() {
    let mut fixture = Fixture::ready();
    assert!(fixture.svm.get_account(&fixture.record_key(1)).is_none());

    let id = fixture.accept_next_update();

    let account = fixture
        .svm
        .get_account(&fixture.record_key(1))
        .expect("record exists");
    assert_eq!(account.owner, solevm_remote_leg::ID);
    assert_eq!(account.data.len(), ConsumedMessage::LEN);

    let record =
        ConsumedMessage::try_deserialize(&mut account.data.as_slice()).expect("record decodes");
    assert_eq!(record.message_id, id);
    assert_eq!(record.sequence, 1);
    assert_eq!(record.message_class, MessageClass::ConfigUpdate);
    assert_eq!(record.lane_id, CONTROL_LANE_ID);
}

#[test]
fn a_prefunded_record_address_does_not_block_the_message() {
    let mut fixture = Fixture::ready();
    let address = fixture.record_key(1);
    fixture.prefund(address, 1);
    assert_eq!(fixture.lamports(address), 1);

    let id = fixture.accept_next_update();

    let account = fixture.svm.get_account(&address).expect("record exists");
    assert_eq!(account.owner, solevm_remote_leg::ID);
    assert_eq!(account.data.len(), ConsumedMessage::LEN);
    assert_eq!(fixture.record(1).message_id, id);
}

#[test]
fn a_record_address_funded_by_a_real_transfer_is_still_usable() {
    let mut fixture = Fixture::ready();
    let address = fixture.record_key(1);

    // A stranger may only leave a rent exempt balance, which is short of the
    // rent the allocated record needs.
    let parked = fixture.empty_account_rent();
    assert!(parked < fixture.record_rent());
    fixture.svm.airdrop(&address, parked).unwrap();

    let id = fixture.accept_next_update();

    assert_eq!(fixture.lamports(address), fixture.record_rent());
    assert_eq!(fixture.record(1).message_id, id);
}

#[test]
fn a_generously_prefunded_record_address_keeps_its_lamports() {
    let mut fixture = Fixture::ready();
    let address = fixture.record_key(1);
    let generous = 5_000_000_000;
    fixture.prefund(address, generous);

    fixture.accept_next_update();

    assert_eq!(fixture.lamports(address), generous);
    assert_eq!(fixture.record(1).sequence, 1);
}

#[test]
fn a_record_address_owned_by_a_stranger_is_rejected() {
    let mut fixture = Fixture::ready();
    let address = fixture.record_key(1);
    fixture.write_owned_account(address, vec![0u8; 8], Pubkey::new_unique());

    let bytes = MessageBuilder::config_update().encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidConsumedMessage,
    );
}

#[test]
fn a_system_owned_record_address_that_already_holds_data_is_rejected() {
    let mut fixture = Fixture::ready();
    let address = fixture.record_key(1);
    fixture.write_owned_account(address, vec![0u8; 8], anchor_lang::system_program::ID);

    let bytes = MessageBuilder::config_update().encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidConsumedMessage,
    );
}

#[test]
fn a_record_address_this_program_already_owns_is_a_replay() {
    let mut fixture = Fixture::ready();
    let address = fixture.record_key(1);
    fixture.write_owned_account(
        address,
        vec![0u8; ConsumedMessage::LEN],
        solevm_remote_leg::ID,
    );

    let bytes = MessageBuilder::config_update().encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::ReplayDetected,
    );
}

#[test]
fn a_record_address_that_is_not_the_canonical_one_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut accounts = fixture.update_accounts(1);
    accounts.consumed_message = Pubkey::new_unique();

    let bytes = MessageBuilder::config_update().encode();
    let verifier = fixture.verifier_keypair();
    expect_error(
        fixture.config_update_with(accounts, bytes, &verifier),
        RemoteLegError::InvalidConsumedMessage,
    );
}

#[test]
fn a_record_address_built_from_a_lower_bump_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut accounts = fixture.update_accounts(1);
    accounts.consumed_message = non_canonical_record(&fixture, 1);
    assert_ne!(accounts.consumed_message, fixture.record_key(1));

    let bytes = MessageBuilder::config_update().encode();
    let verifier = fixture.verifier_keypair();
    expect_error(
        fixture.config_update_with(accounts, bytes, &verifier),
        RemoteLegError::InvalidConsumedMessage,
    );
}

#[test]
fn the_record_of_another_sequence_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut accounts = fixture.update_accounts(1);
    accounts.consumed_message = fixture.record_key(2);

    let bytes = MessageBuilder::config_update().encode();
    let verifier = fixture.verifier_keypair();
    expect_error(
        fixture.config_update_with(accounts, bytes, &verifier),
        RemoteLegError::InvalidConsumedMessage,
    );
}

// Watermark advancement

#[test]
fn the_administrator_advances_the_watermark() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);

    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 2)
        .expect("watermark advances");

    let lane = fixture.lane(MessageClass::ConfigUpdate);
    assert_eq!(lane.minimum_acceptable_sequence, 2);
    assert_eq!(lane.highest_consumed_sequence, 4);
}

#[test]
fn the_guardian_may_not_advance_the_watermark() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);

    let guardian = fixture.guardian.insecure_clone();
    expect_error(
        fixture.advance_watermark(&guardian, MessageClass::ConfigUpdate, 2),
        RemoteLegError::Unauthorized,
    );
}

#[test]
fn the_transport_verifier_may_not_advance_the_watermark() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);

    let verifier = fixture.verifier_keypair();
    expect_error(
        fixture.advance_watermark(&verifier, MessageClass::ConfigUpdate, 2),
        RemoteLegError::Unauthorized,
    );
}

#[test]
fn a_watermark_that_moves_backward_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(6);
    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 3)
        .expect("watermark advances");

    expect_error(
        fixture.advance_watermark(&administrator, MessageClass::ConfigUpdate, 2),
        RemoteLegError::InvalidWatermark,
    );
}

#[test]
fn a_watermark_that_does_not_move_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    let administrator = fixture.administrator.insecure_clone();

    expect_error(
        fixture.advance_watermark(&administrator, MessageClass::ConfigUpdate, 1),
        RemoteLegError::InvalidWatermark,
    );
}

#[test]
fn a_watermark_above_the_highest_consumed_sequence_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    let administrator = fixture.administrator.insecure_clone();

    expect_error(
        fixture.advance_watermark(&administrator, MessageClass::ConfigUpdate, 5),
        RemoteLegError::InvalidWatermark,
    );
}

#[test]
fn a_watermark_that_breaks_the_mandatory_lag_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    let administrator = fixture.administrator.insecure_clone();

    // The lag needs the highest sequence to stay two ahead of the minimum.
    expect_error(
        fixture.advance_watermark(&administrator, MessageClass::ConfigUpdate, 3),
        RemoteLegError::WatermarkLagViolation,
    );
    assert_eq!(WATERMARK_LAG, 2);
}

#[test]
fn advancing_the_watermark_leaves_the_chain_state_alone() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    let before = fixture.lane(MessageClass::ConfigUpdate);
    let risk_before = fixture.raw_data(fixture.risk());
    let config_before = fixture.raw_data(fixture.config);

    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 2)
        .expect("watermark advances");

    let after = fixture.lane(MessageClass::ConfigUpdate);
    assert_eq!(after.message_commitment, before.message_commitment);
    assert_eq!(
        after.highest_consumed_sequence,
        before.highest_consumed_sequence
    );
    assert_eq!(
        after.mandatory_watermark_lag,
        before.mandatory_watermark_lag
    );
    assert_eq!(fixture.raw_data(fixture.risk()), risk_before);
    assert_eq!(fixture.raw_data(fixture.config), config_before);
}

#[test]
fn a_lane_of_another_class_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);

    let administrator = fixture.administrator.insecure_clone();
    let instruction = fixture.watermark_instruction(
        administrator.pubkey(),
        fixture.lane_key(MessageClass::ConfigUpdate),
        None,
        MessageClass::Allocate,
        2,
    );
    expect_error(
        fixture.send(instruction, &[&administrator]),
        RemoteLegError::InvalidLane,
    );
}

#[test]
fn an_untouched_lane_cannot_advance() {
    let mut fixture = Fixture::ready();
    let administrator = fixture.administrator.insecure_clone();

    expect_error(
        fixture.advance_watermark(&administrator, MessageClass::Allocate, 2),
        RemoteLegError::InvalidWatermark,
    );
}

// Record closure

#[test]
fn a_record_below_the_watermark_closes_and_pays_the_administrator() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 2)
        .expect("watermark advances");

    let rent = fixture.lamports(fixture.record_key(1));
    assert!(rent > 0);
    let before = fixture.lamports(administrator.pubkey());

    fixture.close_record(1).expect("record closes");

    assert_eq!(fixture.lamports(administrator.pubkey()), before + rent);
    assert!(!fixture.record_exists(1));
}

#[test]
fn a_record_at_the_watermark_cannot_close() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 2)
        .expect("watermark advances");

    expect_error(fixture.close_record(2), RemoteLegError::RecordNotClosable);
}

#[test]
fn a_record_above_the_watermark_cannot_close() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 2)
        .expect("watermark advances");

    expect_error(fixture.close_record(3), RemoteLegError::RecordNotClosable);
}

#[test]
fn no_record_can_close_before_the_watermark_moves() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);

    for sequence in 1..=4 {
        expect_error(
            fixture.close_record(sequence),
            RemoteLegError::RecordNotClosable,
        );
    }
}

#[test]
fn a_caller_chosen_rent_destination_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 2)
        .expect("watermark advances");

    let thief = fixture.outsider.pubkey();
    expect_error(
        fixture.close_record_to(1, thief),
        RemoteLegError::InvalidRentDestination,
    );
    assert!(fixture.record_exists(1));
}

#[test]
fn closing_a_record_leaves_the_lane_untouched() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 2)
        .expect("watermark advances");

    let before = fixture.raw_data(fixture.lane_key(MessageClass::ConfigUpdate));
    let risk_before = fixture.raw_data(fixture.risk());
    let config_before = fixture.raw_data(fixture.config);

    fixture.close_record(1).expect("record closes");

    assert_eq!(
        fixture.raw_data(fixture.lane_key(MessageClass::ConfigUpdate)),
        before
    );
    assert_eq!(fixture.raw_data(fixture.risk()), risk_before);
    assert_eq!(fixture.raw_data(fixture.config), config_before);
}

#[test]
fn a_closed_record_cannot_be_replayed() {
    let mut fixture = Fixture::ready();
    let (first_bytes, _) = MessageBuilder::config_update().encode_with_id();
    fixture
        .config_update(1, first_bytes.clone())
        .expect("first update lands");
    fixture.accept_updates(3);

    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 2)
        .expect("watermark advances");
    fixture.close_record(1).expect("record closes");
    assert!(!fixture.record_exists(1));

    // The watermark decides first, so the missing record is never consulted.
    expect_error(
        fixture.config_update(1, first_bytes),
        RemoteLegError::SequenceBelowWatermark,
    );
}

#[test]
fn a_lane_from_another_class_cannot_close_a_record() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 2)
        .expect("watermark advances");

    let instruction = fixture.close_record_instruction(
        administrator.pubkey(),
        fixture.lane_key(MessageClass::Allocate),
        fixture.record_key(1),
    );
    let payer = fixture.outsider.insecure_clone();
    expect_error(
        fixture.send_as(instruction, &payer, &[&payer]),
        RemoteLegError::InvalidConsumedMessage,
    );
}

#[test]
fn a_record_that_does_not_exist_cannot_close() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    expect_anchor_error(fixture.close_record(9), ErrorCode::AccountNotInitialized);
}

#[test]
fn closing_twice_is_rejected() {
    let mut fixture = Fixture::ready();
    fixture.accept_updates(4);
    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 2)
        .expect("watermark advances");

    fixture.close_record(1).expect("record closes");
    expect_rejection(fixture.close_record(1));
}
