//! Replay and destination-authentication checks for the live E2E driver.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

use solevm_e2e::ops::WireOp;
use solevm_e2e::world::World;

const ALLOCATE_HEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../contracts/solevm-vault/testdata/control-lane/evm-allocate.hex"
));
const AMOUNT: u64 = 1_000_000;
const ALLOCATE_ID: &str = "0x7777777777777777777777777777777777777777777777777777777777777777";

fn healthy_script() -> Vec<WireOp> {
    vec![
        WireOp::ScheduleControl {
            bytes: ALLOCATE_HEX.trim().to_string(),
            at: 1,
        },
        WireOp::ScheduleAsset {
            transfer: String::from(ALLOCATE_ID),
            amount: AMOUNT,
            at: 2,
        },
        WireOp::RunUntilIdle,
        WireOp::ApplyControl,
        WireOp::ApplyAsset,
        WireOp::Attribute {
            transfer: String::from(ALLOCATE_ID),
        },
        WireOp::Deploy { amount: AMOUNT },
    ]
}

#[test]
fn a_healthy_path_attributes_only_after_both_arrivals() {
    let mut world = World::new();
    world
        .apply(&WireOp::ScheduleControl {
            bytes: ALLOCATE_HEX.trim().to_string(),
            at: 1,
        })
        .unwrap();
    world.apply(&WireOp::RunUntilIdle).unwrap();
    world.apply(&WireOp::ApplyControl).unwrap();
    let after_message = world.observe();
    assert!(after_message.last_accepted);
    assert_eq!(after_message.principal, 0);
    assert!(after_message.transfer_open);

    world
        .apply(&WireOp::ScheduleAsset {
            transfer: String::from(ALLOCATE_ID),
            amount: AMOUNT,
            at: 2,
        })
        .unwrap();
    world.apply(&WireOp::RunUntilIdle).unwrap();
    world.apply(&WireOp::ApplyAsset).unwrap();
    let after_assets = world.observe();
    assert_eq!(after_assets.principal, 0);
    assert_eq!(after_assets.custody, AMOUNT);

    world
        .apply(&WireOp::Attribute {
            transfer: String::from(ALLOCATE_ID),
        })
        .unwrap();
    let after_attr = world.observe();
    assert_eq!(after_attr.principal, AMOUNT);
    assert_eq!(after_attr.unattributed, 0);
    assert!(!after_attr.transfer_open);
}

#[test]
fn replaying_the_same_ops_reproduces_state() {
    let mut first = World::new();
    first.apply_all(&healthy_script()).unwrap();
    let mut second = World::new();
    second.apply_all(&healthy_script()).unwrap();
    assert_eq!(first.observe(), second.observe());
}

fn reject_mutated_allocate(edit: impl FnOnce(&mut protocol_types::Message)) {
    let bytes = solevm_e2e::hexutil::decode(ALLOCATE_HEX.trim()).unwrap();
    let mut message = protocol_types::decode_message(&bytes).unwrap();
    edit(&mut message);
    let encoded = protocol_types::encode_message(&message).unwrap();
    let mut world = World::new();
    world
        .apply(&WireOp::ScheduleControl {
            bytes: solevm_e2e::hexutil::encode(&encoded),
            at: 1,
        })
        .unwrap();
    world.apply(&WireOp::RunUntilIdle).unwrap();
    world.apply(&WireOp::ApplyControl).unwrap();
    let observed = world.observe();
    assert!(!observed.last_accepted);
    assert_eq!(observed.principal, 0);
    assert!(!observed.transfer_open);
}

#[test]
fn a_wrong_destination_chain_is_rejected() {
    reject_mutated_allocate(|message| {
        message.header.destination_chain = protocol_types::ChainId::new(901);
    });
}

#[test]
fn a_wrong_source_application_is_rejected() {
    reject_mutated_allocate(|message| {
        message.header.source_application = protocol_types::ApplicationId::new([0x99; 32]);
    });
}

#[test]
fn a_wrong_lane_id_is_rejected() {
    reject_mutated_allocate(|message| {
        message.header.lane_id = protocol_types::LaneId::new(9);
    });
}

#[test]
fn a_wrong_previous_commitment_is_rejected() {
    reject_mutated_allocate(|message| {
        message.header.previous_commitment = protocol_types::Commitment::new([0xAB; 32]);
    });
}
