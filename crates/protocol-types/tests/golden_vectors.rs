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
    message: &'static str,
    body: &'static str,
    body_hash: &'static str,
    message_id: &'static str,
    next_commitment: &'static str,
}

const ALLOCATE: Vector = Vector {
    kind: MessageType::Allocate,
    message: "5356453100010001000100000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff100000008013380365901ee89d2cda5c644d403ab7855e04bde7d4cb8c4619ea5d93d8a65811111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222222222222222222222222222222222222000000000000000000000000000f4240000000000000000000000000004c4b40000000000000000000000000000f3e58000000006553f4c00000000000000004",
    body: "11111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222222222222222222222222222222222222000000000000000000000000000f4240000000000000000000000000004c4b40000000000000000000000000000f3e58000000006553f4c00000000000000004",
    body_hash: "13380365901ee89d2cda5c644d403ab7855e04bde7d4cb8c4619ea5d93d8a658",
    message_id: "5c1386538d74b14ff2c6f8f22166c7a5cfaa6be2ceab5a4b7e778d8f02a4c9c5",
    next_commitment: "bda9524cc7f907a9d587527f2b89d583813ad305493626af8c3a9f60eae18f0a",
};

const RECALL: Vector = Vector {
    kind: MessageType::Recall,
    message: "5356453100010001000200000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff1000000070c96cd2c2755c562dd1c197054254b85a78a0d8ba73ddfad832f9f45b10d20e0333333333333333333333333333333333333333333333333333333333333333334444444444444444444444444444444444444444444444444444444444444444000000000000000000000000000b71b0000000000000000000000000000b4aa0000000006553f8440000000000000009",
    body: "33333333333333333333333333333333333333333333333333333333333333334444444444444444444444444444444444444444444444444444444444444444000000000000000000000000000b71b0000000000000000000000000000b4aa0000000006553f8440000000000000009",
    body_hash: "c96cd2c2755c562dd1c197054254b85a78a0d8ba73ddfad832f9f45b10d20e03",
    message_id: "4e13963cc6513817c7e62a8246714e64eef2ba020743eef9feed6936ebcf61d6",
    next_commitment: "eea37ba1bbfa85332cf4e489641f5a19db44769c3f32bbc83d7855f079916386",
};

const REMOTE_REPORT: Vector = Vector {
    kind: MessageType::RemoteReport,
    message: "5356453100010001000300000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff10000000e0818c32311333a72eb97ccdc489db42bae66f4fff188795b987e65f53cc326f655555555555555555555555555555555555555555555555555555555555555555000000000000000c6666666666666666666666666666666666666666666666666666666666666666000000000000000000000000001e8480000000000000000000000000001f47d0000000000000000000000000000005dc0000000000000000000000000000001977777777777777777777777777777777777777777777777777777777777777770100000000000000000000006553f0c400000000000000048888888888888888888888888888888888888888888888888888888888888888",
    body: "5555555555555555555555555555555555555555555555555555555555555555000000000000000c6666666666666666666666666666666666666666666666666666666666666666000000000000000000000000001e8480000000000000000000000000001f47d0000000000000000000000000000005dc0000000000000000000000000000001977777777777777777777777777777777777777777777777777777777777777770100000000000000000000006553f0c400000000000000048888888888888888888888888888888888888888888888888888888888888888",
    body_hash: "818c32311333a72eb97ccdc489db42bae66f4fff188795b987e65f53cc326f65",
    message_id: "9b056f787501741a4e0eae869db1ba4aabfb79394b9bed61b2d21063ec33e5a5",
    next_commitment: "8b3a0a151086b00dd9dc2d5cda96abf9c194e23b7713d6c5f54cda683fad0911",
};

const RECALL_SENT: Vector = Vector {
    kind: MessageType::RecallSent,
    message: "5356453100010001000400000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff10000000a01799ed362a0bd3fb909593a38b591f326d8d5f6317846b209ecc3451df8bf5959999999999999999999999999999999999999999999999999999999999999999aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa0000000000000000000000000007a12000000000000000000000000000079950000000000000000000000000000007d0bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb000000006553f11e0000000000000004",
    body: "9999999999999999999999999999999999999999999999999999999999999999aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa0000000000000000000000000007a12000000000000000000000000000079950000000000000000000000000000007d0bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb000000006553f11e0000000000000004",
    body_hash: "1799ed362a0bd3fb909593a38b591f326d8d5f6317846b209ecc3451df8bf595",
    message_id: "c195ce0f469926edc28dfb0a03687e7c50a68a74dbd23adc519dae867086cb42",
    next_commitment: "5d1c2e02813888027811fd5b8113a6e3d6f331ec0256128ca7a91d13963f19a8",
};

const CONFIG_UPDATE: Vector = Vector {
    kind: MessageType::ConfigUpdate,
    message: "5356453100010001000500000000000100000002000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d00e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e00000007000000000000002ac1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000006553f100000000006553f13c000000006553ff10000000486ccec51e3e48a9efacfc38094064c96cb23320d4cf61bd922a6975eb42089df400000000000000050000000000000004177000c803e800000000000000000e1000000000655542bccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    body: "00000000000000050000000000000004177000c803e800000000000000000e1000000000655542bccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    body_hash: "6ccec51e3e48a9efacfc38094064c96cb23320d4cf61bd922a6975eb42089df4",
    message_id: "68240d52636fb7846819c1deabf84074048723239eb23eb279c0e4f14d637c4a",
    next_commitment: "40044287bf9eabb9d1e9e80ff1e4b0ffa917c6a8fd9d76533ff7c15fc57bfdfd",
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
    }
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
