//! Exact EVM Allocate and Recall bytes on the shared Solana control lane.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::messages::next_commitment;
use common::{ALLOCATE_TRANSFER_ID, Fixture, RECALL_TRANSFER_ID};
use solevm_remote_leg::message;

const ALLOCATE_HEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/solevm-vault/testdata/control-lane/evm-allocate.hex"
));
const RECALL_HEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/solevm-vault/testdata/control-lane/evm-recall.hex"
));

fn from_hex(text: &str) -> Vec<u8> {
    let text = text.trim();
    let text = text.strip_prefix("0x").unwrap_or(text);
    assert!(text.len().is_multiple_of(2), "hex needs an even length");
    (0..text.len() / 2)
        .map(|index| {
            u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex digit pair")
        })
        .collect()
}

#[test]
fn exact_evm_allocate_then_recall_bytes_are_accepted() {
    let allocate = from_hex(ALLOCATE_HEX);
    let recall = from_hex(RECALL_HEX);
    assert_eq!(allocate.len(), protocol_types::layout::ALLOCATE_MESSAGE_LEN);
    assert_eq!(recall.len(), protocol_types::layout::RECALL_MESSAGE_LEN);

    let decoded_allocate = protocol_types::decode_message(&allocate).expect("allocate decodes");
    let decoded_recall = protocol_types::decode_message(&recall).expect("recall decodes");
    assert_eq!(
        decoded_allocate.message_type(),
        protocol_types::MessageType::Allocate
    );
    assert_eq!(
        decoded_recall.message_type(),
        protocol_types::MessageType::Recall
    );
    assert_eq!(decoded_allocate.header.sequence.get(), 1);
    assert_eq!(
        decoded_allocate.header.previous_commitment.to_bytes(),
        [0u8; 32]
    );
    assert_eq!(decoded_recall.header.sequence.get(), 2);

    let allocate_id = message::message_id(&allocate).expect("allocate id");
    assert_eq!(
        decoded_recall.header.previous_commitment.to_bytes(),
        next_commitment([0u8; 32], allocate_id)
    );

    let mut fixture = Fixture::deployed();
    fixture
        .allocate(ALLOCATE_TRANSFER_ID, 1, allocate)
        .expect("exact EVM allocate bytes are accepted");
    fixture.credit(fixture.custody, 1_000_000);
    fixture
        .attribute(ALLOCATE_TRANSFER_ID)
        .expect("attribution lands");

    fixture
        .recall(RECALL_TRANSFER_ID, 2, recall)
        .expect("exact EVM recall bytes are accepted");

    let recall_id = message::message_id(&from_hex(RECALL_HEX)).expect("recall id");
    let lane = fixture.control_lane();
    assert_eq!(lane.highest_consumed_sequence, 2);
    assert_eq!(
        lane.message_commitment,
        next_commitment(next_commitment([0u8; 32], allocate_id), recall_id)
    );
}
