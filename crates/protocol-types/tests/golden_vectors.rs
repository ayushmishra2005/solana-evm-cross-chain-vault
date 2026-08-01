//! Fixed expected bytes for every message type.
//!
//! The constants were produced by a separate generator, not by this crate.
//! They are the reference for future decoders on other runtimes.

#![allow(clippy::unwrap_used)]

mod common;

use common::{CHAIN_PREVIOUS, from_hex, to_hex};
use protocol_types::{MessageType, decode_message, next_commitment};

struct Vector {
    kind: MessageType,
    message_len: usize,
    message: &'static str,
    body: &'static str,
    body_hash: &'static str,
    message_id: &'static str,
    next_commitment: &'static str,
}

const ALLOCATE: Vector = Vector {
    kind: MessageType::Allocate,
    message_len: 343,
    message: "53564531000100010100000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff1096072587eba139c7c62e7e0bf95cf18e88ee6bd7ae813700d807fdc797629d811111111111111111111111111111111111111111111111111111111111111111000000000000000000000000000f4240000000000000000000000000004c4b40000000000000000000000000000f3e58000000006553f4c00000000000000004",
    body: "1111111111111111111111111111111111111111111111111111111111111111000000000000000000000000000f4240000000000000000000000000004c4b40000000000000000000000000000f3e58000000006553f4c00000000000000004",
    body_hash: "96072587eba139c7c62e7e0bf95cf18e88ee6bd7ae813700d807fdc797629d81",
    message_id: "df4d3cb70be20986d19b4b53c17d89d1583e16d4cf688dce128cf7e36edf3de0",
    next_commitment: "6d27119bf60d8774fbd0c29e7da80311ffc0880f928f485ea9aa627ce5bed586",
};

const RECALL: Vector = Vector {
    kind: MessageType::Recall,
    message_len: 327,
    message: "53564531000100010200000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff10455881245b59df016fb249abdc88ac9976aa87aa90e5b87eca1dfdf12791549c3333333333333333333333333333333333333333333333333333333333333333000000000000000000000000000b71b0000000000000000000000000000b4aa0000000006553f8440000000000000009",
    body: "3333333333333333333333333333333333333333333333333333333333333333000000000000000000000000000b71b0000000000000000000000000000b4aa0000000006553f8440000000000000009",
    body_hash: "455881245b59df016fb249abdc88ac9976aa87aa90e5b87eca1dfdf12791549c",
    message_id: "e0fa56e15104e1d2a64b58444ff5b86f9233ac1cff611f826040e8b2f0f88cb0",
    next_commitment: "39cf103b11f516f82b44cbb6823c11363183b7d3dc10180623759675d797f821",
};

const REMOTE_REPORT: Vector = Vector {
    kind: MessageType::RemoteReport,
    message_len: 400,
    message: "53564531000100010300000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff100e4d8b8768b55f6677fe54446a60c1d1e532453dd44880185f48a55a43255010000000000000000c000000000000000000000000001e8480000000000000000000000000001f47d0000000000000000000000000000005dc00000000000000000000000000000019777777777777777777777777777777777777777777777777777777777777777701000000006553f08800000000000000048888888888888888888888888888888888888888888888888888888888888888",
    body: "000000000000000c000000000000000000000000001e8480000000000000000000000000001f47d0000000000000000000000000000005dc00000000000000000000000000000019777777777777777777777777777777777777777777777777777777777777777701000000006553f08800000000000000048888888888888888888888888888888888888888888888888888888888888888",
    body_hash: "0e4d8b8768b55f6677fe54446a60c1d1e532453dd44880185f48a55a43255010",
    message_id: "7cbb454a17532a932ea3939a553d77a7156920c8ef648fe8def3f7000cdfb25c",
    next_commitment: "f05852255eccb4699ea5bf117a6f77f00a3e9489679ce9a80e53186d4b0f4f15",
};

const RECALL_SENT: Vector = Vector {
    kind: MessageType::RecallSent,
    message_len: 375,
    message: "53564531000100010400000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff10ca2a3bbe730bbb44698903ec9e331384cb6511beb3e6591d390cf3e5baaa7c9499999999999999999999999999999999999999999999999999999999999999990000000000000000000000000007a12000000000000000000000000000079950000000000000000000000000000007d0bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb000000006553f0e20000000000000004",
    body: "99999999999999999999999999999999999999999999999999999999999999990000000000000000000000000007a12000000000000000000000000000079950000000000000000000000000000007d0bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb000000006553f0e20000000000000004",
    body_hash: "ca2a3bbe730bbb44698903ec9e331384cb6511beb3e6591d390cf3e5baaa7c94",
    message_id: "774163b1ebde24135caaad565474e328b4b25beac78be7c5dc48d45f385a99c5",
    next_commitment: "82f79229da2ccb5a039ed3714a5ac31370cf02a10bdba9d40d5a888605e6c128",
};

const CONFIG_UPDATE: Vector = Vector {
    kind: MessageType::ConfigUpdate,
    message_len: 317,
    message: "53564531000100010500000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff1001042485064a2b918d0679c166f39acf59c94c6cae6ca29fdbd83579ae00004a00000000000000050000000000000004177000c803e80000000000000e1000000000655542bccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    body: "00000000000000050000000000000004177000c803e80000000000000e1000000000655542bccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    body_hash: "01042485064a2b918d0679c166f39acf59c94c6cae6ca29fdbd83579ae00004a",
    message_id: "76ad099206289b0449bb5db992bf0c91ab0c3568968739a4e2e6ca4a76041d13",
    next_commitment: "0d6eeed4406a8370dd890e631d27287fb17b9e11203bb94fe2d6748ed22a2339",
};

const VECTORS: [&Vector; 5] = [
    &ALLOCATE,
    &RECALL,
    &REMOTE_REPORT,
    &RECALL_SENT,
    &CONFIG_UPDATE,
];

#[test]
fn every_fixture_encodes_to_its_expected_bytes() {
    for vector in VECTORS {
        let encoded = common::encoded(vector.kind);
        assert_eq!(to_hex(&encoded), vector.message, "{:?}", vector.kind);
    }
}

#[test]
fn every_expected_body_is_the_tail_of_its_expected_message() {
    for vector in VECTORS {
        let message = from_hex(vector.message);
        let body = from_hex(vector.body);
        let start = protocol_types::layout::HEADER_LEN;
        assert_eq!(
            message.get(start..),
            Some(body.as_slice()),
            "{:?}",
            vector.kind
        );
    }
}

#[test]
fn every_expected_body_hash_matches_its_expected_body() {
    for vector in VECTORS {
        let body = from_hex(vector.body);
        assert_eq!(
            to_hex(&protocol_types::keccak256(&body)),
            vector.body_hash,
            "{:?}",
            vector.kind
        );
    }
}

#[test]
fn every_fixture_produces_its_expected_message_id() {
    for vector in VECTORS {
        let id = common::message(vector.kind).message_id().unwrap();
        assert_eq!(
            to_hex(id.as_bytes()),
            vector.message_id,
            "{:?}",
            vector.kind
        );
    }
}

#[test]
fn every_fixture_produces_its_expected_next_commitment() {
    for vector in VECTORS {
        let id = common::message(vector.kind).message_id().unwrap();
        let link = next_commitment(CHAIN_PREVIOUS, id);
        assert_eq!(
            to_hex(link.as_bytes()),
            vector.next_commitment,
            "{:?}",
            vector.kind
        );
    }
}

#[test]
fn every_expected_message_decodes_back_to_its_fixture() {
    for vector in VECTORS {
        let decoded = decode_message(&from_hex(vector.message));
        assert_eq!(
            decoded,
            Ok(common::message(vector.kind)),
            "{:?}",
            vector.kind
        );
    }
}

#[test]
fn every_expected_message_has_the_declared_length() {
    for vector in VECTORS {
        assert_eq!(
            from_hex(vector.message).len(),
            vector.kind.message_len(),
            "{:?}",
            vector.kind
        );
        assert_eq!(
            vector.message_len,
            vector.kind.message_len(),
            "{:?}",
            vector.kind
        );
    }
}

#[test]
fn every_expected_message_fits_the_transport_budget() {
    for vector in VECTORS {
        assert!(
            vector.message_len <= protocol_types::layout::MESSAGE_SIZE_TARGET,
            "{:?} is {} bytes",
            vector.kind,
            vector.message_len
        );
    }
}

#[test]
fn the_expected_lengths_are_the_reviewed_numbers() {
    assert_eq!(ALLOCATE.message_len, 343);
    assert_eq!(RECALL.message_len, 327);
    assert_eq!(REMOTE_REPORT.message_len, 400);
    assert_eq!(RECALL_SENT.message_len, 375);
    assert_eq!(CONFIG_UPDATE.message_len, 317);
}

#[test]
fn the_expected_messages_and_ids_are_all_distinct() {
    let mut seen = Vec::new();
    for vector in VECTORS {
        assert!(!seen.contains(&vector.message), "duplicate message bytes");
        assert!(!seen.contains(&vector.message_id), "duplicate message id");
        seen.push(vector.message);
        seen.push(vector.message_id);
    }
}
