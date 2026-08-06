//! The program and the shared protocol crate agree on every canonical value.
//!
//! The constants below are the published golden vectors. They are repeated
//! here so the Solana workspace pins them on its own.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use protocol_types::{Commitment, MessageId, MessageType, decode_message, keccak256, layout};
use solevm_remote_leg::{MessageClass, message};

use common::Fixture;
use common::messages::MessageBuilder;

/// Previous commitment the published chaining vectors start from.
const CHAIN_PREVIOUS: [u8; 32] = [0xF3; 32];

struct Vector {
    kind: MessageType,
    message_len: usize,
    message: &'static str,
    body_hash: &'static str,
    message_id: &'static str,
    next_commitment: &'static str,
}

const ALLOCATE: Vector = Vector {
    kind: MessageType::Allocate,
    message_len: 343,
    message: "53564531000100010100000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff1096072587eba139c7c62e7e0bf95cf18e88ee6bd7ae813700d807fdc797629d811111111111111111111111111111111111111111111111111111111111111111000000000000000000000000000f4240000000000000000000000000004c4b40000000000000000000000000000f3e58000000006553f4c00000000000000004",
    body_hash: "96072587eba139c7c62e7e0bf95cf18e88ee6bd7ae813700d807fdc797629d81",
    message_id: "df4d3cb70be20986d19b4b53c17d89d1583e16d4cf688dce128cf7e36edf3de0",
    next_commitment: "6d27119bf60d8774fbd0c29e7da80311ffc0880f928f485ea9aa627ce5bed586",
};

const RECALL: Vector = Vector {
    kind: MessageType::Recall,
    message_len: 327,
    message: "53564531000100010200000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff10455881245b59df016fb249abdc88ac9976aa87aa90e5b87eca1dfdf12791549c3333333333333333333333333333333333333333333333333333333333333333000000000000000000000000000b71b0000000000000000000000000000b4aa0000000006553f8440000000000000009",
    body_hash: "455881245b59df016fb249abdc88ac9976aa87aa90e5b87eca1dfdf12791549c",
    message_id: "e0fa56e15104e1d2a64b58444ff5b86f9233ac1cff611f826040e8b2f0f88cb0",
    next_commitment: "39cf103b11f516f82b44cbb6823c11363183b7d3dc10180623759675d797f821",
};

const CONFIG_UPDATE: Vector = Vector {
    kind: MessageType::ConfigUpdate,
    message_len: 317,
    message: "53564531000100010500000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff1001042485064a2b918d0679c166f39acf59c94c6cae6ca29fdbd83579ae00004a00000000000000050000000000000004177000c803e80000000000000e1000000000655542bccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    body_hash: "01042485064a2b918d0679c166f39acf59c94c6cae6ca29fdbd83579ae00004a",
    message_id: "76ad099206289b0449bb5db992bf0c91ab0c3568968739a4e2e6ca4a76041d13",
    next_commitment: "0d6eeed4406a8370dd890e631d27287fb17b9e11203bb94fe2d6748ed22a2339",
};

const VECTORS: [&Vector; 3] = [&ALLOCATE, &RECALL, &CONFIG_UPDATE];

fn from_hex(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "hex needs an even length");
    (0..text.len() / 2)
        .map(|index| {
            u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex digit pair")
        })
        .collect()
}

fn wide(text: &str) -> [u8; 32] {
    from_hex(text).try_into().expect("32 byte value")
}

#[test]
fn the_program_reproduces_every_golden_message_id() {
    for vector in VECTORS {
        let bytes = from_hex(vector.message);
        assert_eq!(bytes.len(), vector.message_len, "{:?}", vector.kind);
        assert_eq!(
            message::message_id(&bytes).expect("id is computed"),
            wide(vector.message_id),
            "{:?}",
            vector.kind
        );
    }
}

#[test]
fn the_program_reproduces_every_golden_chain_link() {
    for vector in VECTORS {
        let id = wide(vector.message_id);
        assert_eq!(
            message::next_commitment(&CHAIN_PREVIOUS, &id),
            wide(vector.next_commitment),
            "{:?}",
            vector.kind
        );
    }
}

#[test]
fn the_shared_keccak_reproduces_every_golden_body_hash() {
    for vector in VECTORS {
        let bytes = from_hex(vector.message);
        assert_eq!(
            keccak256(&bytes[layout::HEADER_LEN..]),
            wide(vector.body_hash),
            "{:?}",
            vector.kind
        );
    }
}

#[test]
fn every_golden_message_still_decodes_through_the_shared_codec() {
    for vector in VECTORS {
        let bytes = from_hex(vector.message);
        let message = decode_message(&bytes).expect("golden message decodes");
        assert_eq!(message.message_type(), vector.kind);
    }
}

#[test]
fn every_protocol_length_is_unchanged() {
    assert_eq!(layout::HEADER_LEN, 247);
    assert_eq!(layout::ALLOCATE_MESSAGE_LEN, 343);
    assert_eq!(layout::RECALL_MESSAGE_LEN, 327);
    assert_eq!(layout::REMOTE_REPORT_MESSAGE_LEN, 400);
    assert_eq!(layout::RECALL_SENT_MESSAGE_LEN, 375);
    assert_eq!(layout::CONFIG_UPDATE_MESSAGE_LEN, 317);
    assert_eq!(layout::MAX_MESSAGE_LEN, 400);
    assert_eq!(message::MAX_MESSAGE_LEN, layout::MAX_MESSAGE_LEN);
}

#[test]
fn the_program_and_the_shared_crate_agree_on_a_fresh_message() {
    let (bytes, id) = MessageBuilder::config_update().encode_with_id();
    assert_eq!(message::message_id(&bytes).expect("id is computed"), id);

    let previous = [0x21; 32];
    assert_eq!(
        message::next_commitment(&previous, &id),
        protocol_types::next_commitment(Commitment::new(previous), MessageId::new(id)).to_bytes()
    );
}

#[test]
fn the_chain_accepts_a_message_only_when_its_shared_hashes_agree() {
    let mut fixture = Fixture::ready();
    let (bytes, id) = MessageBuilder::config_update().encode_with_id();
    fixture.config_update(1, bytes).expect("update lands");

    // The record proves the on chain Keccak matched the host Keccak.
    assert_eq!(fixture.record(1).message_id, id);
}

#[test]
fn the_chain_decodes_an_allocation_through_the_shared_codec() {
    let mut fixture = Fixture::deployed();
    let bytes = fixture.allocate_bytes(common::ALLOCATE_TRANSFER_ID, 1_000_000, 1);
    let id = message::message_id(&bytes).expect("id is computed");
    let decoded = decode_message(&bytes).expect("the shared codec decodes");
    assert_eq!(decoded.message_type(), MessageType::Allocate);

    fixture
        .allocate(common::ALLOCATE_TRANSFER_ID, 1, bytes)
        .expect("allocation lands");

    assert_eq!(
        fixture.asset_record(MessageClass::Allocate, 1).message_id,
        id
    );
    assert_eq!(fixture.lane(MessageClass::Allocate).message_commitment, {
        let previous = [0u8; 32];
        message::next_commitment(&previous, &id)
    });
}

#[test]
fn the_chain_decodes_a_recall_through_the_shared_codec() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(common::ALLOCATE_TRANSFER_ID, 1_000_000);

    let bytes = fixture.recall_bytes(common::RECALL_TRANSFER_ID, 1_000_000, 1);
    let id = message::message_id(&bytes).expect("id is computed");
    assert_eq!(
        decode_message(&bytes)
            .expect("the shared codec decodes")
            .message_type(),
        MessageType::Recall
    );

    fixture
        .recall(common::RECALL_TRANSFER_ID, 1, bytes)
        .expect("recall lands");

    assert_eq!(fixture.asset_record(MessageClass::Recall, 1).message_id, id);
}

#[test]
fn the_program_holds_no_second_decoder() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0;

    for entry in walk(&source) {
        let text = std::fs::read_to_string(&entry).expect("source file reads");
        assert!(
            !text.contains("Keccak256"),
            "{} builds its own hash",
            entry.display()
        );
        assert!(
            !text.contains("MAGIC_OFFSET"),
            "{} reads the wire format directly",
            entry.display()
        );
        checked += 1;
    }

    assert!(checked >= 6, "expected to scan the whole program source");
}

fn walk(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory).expect("source directory reads") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else if path.extension().is_some_and(|kind| kind == "rs") {
            files.push(path);
        }
    }
    files
}
