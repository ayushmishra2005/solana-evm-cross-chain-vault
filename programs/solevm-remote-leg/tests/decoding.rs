//! Canonical decoding and domain binding of inbound messages.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use protocol_types::{Body, MessageType, decode_message, layout};
use solevm_remote_leg::RemoteLegError;

use common::messages::{MessageBuilder, patch, patch_body};
use common::{
    CONTROL_LANE_ID, DEPLOYMENT_ID, DESTINATION_CHAIN_ID, Fixture, LOCAL_APPLICATION_ID,
    SOURCE_APPLICATION_ID, SOURCE_CHAIN_ID, VAULT_ID, expect_error,
};

// Every class decodes through the one shared implementation

#[test]
fn a_canonical_allocate_message_decodes() {
    let bytes = MessageBuilder::allocate().encode();
    assert_eq!(bytes.len(), layout::ALLOCATE_MESSAGE_LEN);

    let message = decode_message(&bytes).expect("allocate decodes");
    assert_eq!(message.message_type(), MessageType::Allocate);
    assert!(matches!(message.body, Body::Allocate(_)));
    assert_eq!(message.header.lane_id.get(), CONTROL_LANE_ID);
    assert_eq!(message.header.deployment_id.as_bytes(), &DEPLOYMENT_ID);
}

#[test]
fn a_canonical_recall_message_decodes() {
    let bytes = MessageBuilder::recall().encode();
    assert_eq!(bytes.len(), layout::RECALL_MESSAGE_LEN);

    let message = decode_message(&bytes).expect("recall decodes");
    assert_eq!(message.message_type(), MessageType::Recall);
    assert!(matches!(message.body, Body::Recall(_)));
    assert_eq!(message.header.vault_id.as_bytes(), &VAULT_ID);
}

#[test]
fn a_canonical_config_update_message_decodes() {
    let bytes = MessageBuilder::config_update().encode();
    assert_eq!(bytes.len(), layout::CONFIG_UPDATE_MESSAGE_LEN);

    let message = decode_message(&bytes).expect("config update decodes");
    assert_eq!(message.message_type(), MessageType::ConfigUpdate);
    assert!(matches!(message.body, Body::ConfigUpdate(_)));
}

#[test]
fn an_allocate_message_is_not_accepted_on_the_config_lane() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::allocate().encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::UnsupportedMessageType,
    );
}

#[test]
fn a_recall_message_is_not_accepted_on_the_config_lane() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::recall().encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::UnsupportedMessageType,
    );
}

// Structural failures

#[test]
fn an_empty_message_is_rejected() {
    let mut fixture = Fixture::ready();
    expect_error(
        fixture.config_update(1, Vec::new()),
        RemoteLegError::InvalidMessage,
    );
}

#[test]
fn a_message_above_the_protocol_maximum_is_rejected() {
    let mut fixture = Fixture::ready();
    let bytes = vec![0u8; layout::MAX_MESSAGE_LEN + 1];
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::MessageTooLarge,
    );
}

#[test]
fn a_wrong_magic_prefix_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut bytes = MessageBuilder::config_update().encode();
    patch(&mut bytes, layout::MAGIC_OFFSET, b"XXXX");
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidMessage,
    );
}

#[test]
fn an_unsupported_protocol_version_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut bytes = MessageBuilder::config_update().encode();
    patch(
        &mut bytes,
        layout::PROTOCOL_VERSION_OFFSET,
        &2u16.to_be_bytes(),
    );
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidProtocolVersion,
    );
}

#[test]
fn an_unsupported_schema_version_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut bytes = MessageBuilder::config_update().encode();
    patch(
        &mut bytes,
        layout::SCHEMA_VERSION_OFFSET,
        &7u16.to_be_bytes(),
    );
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidSchemaVersion,
    );
}

#[test]
fn an_unknown_message_type_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut bytes = MessageBuilder::config_update().encode();
    patch(&mut bytes, layout::MESSAGE_TYPE_OFFSET, &[0x7F]);
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::UnsupportedMessageType,
    );
}

#[test]
fn a_body_hash_that_does_not_match_the_body_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut bytes = MessageBuilder::config_update().encode();
    patch(&mut bytes, layout::BODY_HASH_OFFSET, &[0x5A; 32]);
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidMessage,
    );
}

#[test]
fn any_reserved_flag_bit_is_rejected() {
    for bits in [1u16, 0x0100, 0x8000] {
        let mut fixture = Fixture::ready();
        let mut bytes = MessageBuilder::config_update().encode();
        patch(&mut bytes, layout::FLAGS_OFFSET, &bits.to_be_bytes());
        expect_error(
            fixture.config_update(1, bytes),
            RemoteLegError::InvalidMessage,
        );
    }
}

#[test]
fn a_message_truncated_at_any_boundary_is_rejected() {
    let full = MessageBuilder::config_update().encode();
    let boundaries = [
        0,
        layout::MAGIC_LEN,
        layout::MESSAGE_TYPE_OFFSET,
        layout::SEQUENCE_OFFSET,
        layout::BODY_HASH_OFFSET,
        layout::HEADER_LEN,
        layout::HEADER_LEN + 1,
        full.len() - 1,
    ];

    for width in boundaries {
        let mut fixture = Fixture::ready();
        let bytes = full[..width].to_vec();
        expect_error(
            fixture.config_update(1, bytes),
            RemoteLegError::InvalidMessage,
        );
    }
}

#[test]
fn a_message_with_trailing_bytes_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut bytes = MessageBuilder::config_update().encode();
    bytes.push(0);
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidMessage,
    );
}

#[test]
fn a_body_that_breaks_a_field_rule_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut bytes = MessageBuilder::config_update().encode();
    patch_body(
        &mut bytes,
        layout::CONFIG_VERSION_OFFSET,
        &0u64.to_be_bytes(),
    );
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidMessage,
    );
}

#[test]
fn a_zero_sequence_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut bytes = MessageBuilder::config_update().encode();
    patch(&mut bytes, layout::SEQUENCE_OFFSET, &0u64.to_be_bytes());
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidMessage,
    );
}

// Domain binding

#[test]
fn a_wrong_source_chain_is_rejected() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::config_update()
        .source_chain(SOURCE_CHAIN_ID + 1)
        .encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidSourceDomain,
    );
}

#[test]
fn a_wrong_destination_chain_is_rejected() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::config_update()
        .destination_chain(DESTINATION_CHAIN_ID + 1)
        .encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidDestinationDomain,
    );
}

#[test]
fn a_wrong_source_application_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut application = SOURCE_APPLICATION_ID;
    application[0] ^= 0xFF;
    let bytes = MessageBuilder::config_update()
        .source_application(application)
        .encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidApplication,
    );
}

#[test]
fn a_wrong_destination_application_is_rejected() {
    let mut fixture = Fixture::ready();
    let mut application = LOCAL_APPLICATION_ID;
    application[31] ^= 0xFF;
    let bytes = MessageBuilder::config_update()
        .destination_application(application)
        .encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidApplication,
    );
}

#[test]
fn a_wrong_deployment_is_rejected() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::config_update()
        .deployment([0x66; 32])
        .encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidDeployment,
    );
}

#[test]
fn a_wrong_vault_is_rejected() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::config_update().vault([0x55; 32]).encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidVault,
    );
}

#[test]
fn a_wrong_lane_is_rejected() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::config_update()
        .lane(CONTROL_LANE_ID + 40)
        .encode();
    expect_error(fixture.config_update(1, bytes), RemoteLegError::InvalidLane);
}

#[test]
fn a_publication_before_observation_never_reaches_the_chain() {
    let message = MessageBuilder::config_update()
        .observed_at(2_000)
        .published_at(1_000)
        .build();
    assert!(message.encode().is_err());
}

#[test]
fn an_expiration_before_publication_never_reaches_the_chain() {
    let message = MessageBuilder::config_update()
        .expires_at(common::messages::PUBLISHED_AT - 1)
        .build();
    assert!(message.encode().is_err());
}

#[test]
fn a_swapped_publication_order_is_rejected_on_chain() {
    let mut fixture = Fixture::ready();
    let mut bytes = MessageBuilder::config_update().encode();
    patch(
        &mut bytes,
        layout::OBSERVED_AT_OFFSET,
        &(common::messages::PUBLISHED_AT + 1).to_be_bytes(),
    );
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidMessage,
    );
}
