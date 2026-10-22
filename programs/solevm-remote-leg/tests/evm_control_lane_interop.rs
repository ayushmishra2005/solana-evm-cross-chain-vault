//! Exact EVM Allocate, Recall and Resolve bytes on the shared Solana control lane.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::messages::next_commitment;
use common::{ALLOCATE_TRANSFER_ID, Fixture, RECALL_TRANSFER_ID, expect_error};
use solevm_remote_leg::{RemoteLegError, TransferStatus, message};

const ALLOCATE_HEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/solevm-vault/testdata/control-lane/evm-allocate.hex"
));
const RECALL_HEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/solevm-vault/testdata/control-lane/evm-recall.hex"
));
const ADMIT_HEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/solevm-vault/testdata/control-lane/evm-admit.hex"
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

#[test]
fn exact_evm_admit_bytes_release_a_frozen_allocation_only_with_the_guardian() {
    let allocate = from_hex(ALLOCATE_HEX);
    let admit = from_hex(ADMIT_HEX);
    assert_eq!(admit.len(), protocol_types::layout::RESOLVE_MESSAGE_LEN);

    let decoded = protocol_types::decode_message(&admit).expect("admit decodes");
    assert_eq!(decoded.message_type(), protocol_types::MessageType::Resolve);
    assert_eq!(decoded.header.sequence.get(), 2);
    let allocate_id = message::message_id(&allocate).expect("allocate id");
    assert_eq!(
        decoded.header.previous_commitment.to_bytes(),
        next_commitment([0u8; 32], allocate_id)
    );
    let protocol_types::Body::Resolve(body) = decoded.body else {
        panic!("admit carries a resolve body");
    };
    assert_eq!(*body.transfer_id.as_bytes(), ALLOCATE_TRANSFER_ID);
    assert_eq!(body.target_sequence.get(), 1);
    assert_eq!(body.action, protocol_types::ResolveAction::Admit);

    let mut fixture = Fixture::deployed();
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");
    fixture
        .allocate(ALLOCATE_TRANSFER_ID, 1, allocate)
        .expect("the frozen leg holds the exact EVM allocation");
    assert_eq!(
        fixture.transfer(&ALLOCATE_TRANSFER_ID).status,
        TransferStatus::Quarantined
    );
    fixture.credit(fixture.custody, 1_000_000);

    expect_error(
        fixture.resolve(ALLOCATE_TRANSFER_ID, 2, admit.clone()),
        RemoteLegError::Frozen,
    );
    let guardian = fixture.guardian.insecure_clone();
    fixture
        .resolve_cosigned(ALLOCATE_TRANSFER_ID, 2, admit, &guardian)
        .expect("exact EVM admit bytes are accepted with the guardian");
    fixture
        .attribute_cosigned(ALLOCATE_TRANSFER_ID, &guardian)
        .expect("attribution lands");

    assert_eq!(
        fixture.transfer(&ALLOCATE_TRANSFER_ID).status,
        TransferStatus::Complete
    );
    assert_eq!(fixture.position().attributed_principal, 1_000_000);
    let admit_id = message::message_id(&from_hex(ADMIT_HEX)).expect("admit id");
    let lane = fixture.control_lane();
    assert_eq!(lane.highest_consumed_sequence, 2);
    assert_eq!(
        lane.message_commitment,
        next_commitment(next_commitment([0u8; 32], allocate_id), admit_id)
    );
}
