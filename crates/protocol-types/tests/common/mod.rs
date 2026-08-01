//! Fixtures shared by the integration tests.
//!
//! The values here match the golden vectors exactly, so a change to one shows
//! up as a golden vector failure.

#![allow(unreachable_pub, dead_code, clippy::unwrap_used)]

use protocol_types::{
    AllocateBody, ApplicationId, AssetAmount, AssetId, BasisPoints, Body, ChainId, Commitment,
    ConfigUpdateBody, ConfigVersion, DeploymentId, EpochId, Flags, Header, LaneId, Message,
    MessageType, PROTOCOL_VERSION, ProbeStatus, RecallBody, RecallSentBody, RemoteReportBody,
    ReportId, SCHEMA_VERSION, Sequence, Timestamp, TransferId, VaultId,
};

pub const OBSERVED_AT: u64 = 1_700_000_000;
pub const PUBLISHED_AT: u64 = 1_700_000_060;
pub const EXPIRES_AT: u64 = 1_700_003_600;

/// Previous link used by every golden commitment vector.
pub const CHAIN_PREVIOUS: Commitment = Commitment::new([0xF3; 32]);

pub const EVM_APPLICATION: [u8; 20] = [0xA1; 20];
pub const SOLANA_APPLICATION: [u8; 32] = [0xB2; 32];

#[must_use]
pub fn header() -> Header {
    Header {
        protocol_version: PROTOCOL_VERSION,
        schema_version: SCHEMA_VERSION,
        flags: Flags::NONE,
        source_chain: ChainId::new(1),
        destination_chain: ChainId::new(2),
        source_application: ApplicationId::from_evm_address(EVM_APPLICATION),
        destination_application: ApplicationId::from_solana_pubkey(SOLANA_APPLICATION),
        deployment_id: DeploymentId::new([0xD0; 32]),
        vault_id: VaultId::new([0x0E; 32]),
        lane_id: LaneId::new(7),
        sequence: Sequence::new(42),
        previous_commitment: Commitment::new([0xC1; 32]),
        observed_at: Timestamp::new(OBSERVED_AT),
        published_at: Timestamp::new(PUBLISHED_AT),
        expires_at: Timestamp::new(EXPIRES_AT),
    }
}

#[must_use]
pub fn allocate_body() -> AllocateBody {
    AllocateBody {
        transfer_id: TransferId::new([0x11; 32]),
        asset_id: AssetId::new([0x22; 32]),
        amount: AssetAmount::new(1_000_000),
        expected_source_balance: AssetAmount::new(5_000_000),
        minimum_destination_amount: AssetAmount::new(999_000),
        deadline: Timestamp::new(PUBLISHED_AT + 900),
        config_version: ConfigVersion::new(4),
    }
}

#[must_use]
pub fn recall_body() -> RecallBody {
    RecallBody {
        transfer_id: TransferId::new([0x33; 32]),
        asset_id: AssetId::new([0x44; 32]),
        requested_amount: AssetAmount::new(750_000),
        minimum_return_amount: AssetAmount::new(740_000),
        deadline: Timestamp::new(PUBLISHED_AT + 1_800),
        config_version: ConfigVersion::new(9),
    }
}

#[must_use]
pub fn remote_report_body() -> RemoteReportBody {
    RemoteReportBody {
        report_id: ReportId::new([0x55; 32]),
        epoch_id: EpochId::new(12),
        asset_id: AssetId::new([0x66; 32]),
        remote_principal: AssetAmount::new(2_000_000),
        reported_value: AssetAmount::new(2_050_000),
        realized_loss: AssetAmount::new(1_500),
        unattributed_balance: AssetAmount::new(25),
        latest_completed_transfer_id: TransferId::new([0x77; 32]),
        probe_status: ProbeStatus::Fresh,
        probe_timestamp: Timestamp::new(PUBLISHED_AT - 120),
        config_version: ConfigVersion::new(4),
        remote_state_commitment: Commitment::new([0x88; 32]),
    }
}

#[must_use]
pub fn recall_sent_body() -> RecallSentBody {
    RecallSentBody {
        transfer_id: TransferId::new([0x99; 32]),
        asset_id: AssetId::new([0xAA; 32]),
        principal_sent: AssetAmount::new(500_000),
        actual_amount_sent: AssetAmount::new(498_000),
        realized_loss: AssetAmount::new(2_000),
        destination_reference: Commitment::new([0xBB; 32]),
        sent_timestamp: Timestamp::new(PUBLISHED_AT - 30),
        config_version: ConfigVersion::new(4),
    }
}

#[must_use]
pub fn config_update_body() -> ConfigUpdateBody {
    ConfigUpdateBody {
        config_version: ConfigVersion::new(5),
        previous_config_version: ConfigVersion::new(4),
        max_remote_allocation_bps: BasisPoints::new(6_000),
        max_upward_deviation_bps: BasisPoints::new(200),
        max_downward_deviation_bps: BasisPoints::new(1_000),
        max_report_age: 3_600,
        effective_timestamp: Timestamp::new(PUBLISHED_AT + 86_400),
        config_commitment: Commitment::new([0xCC; 32]),
    }
}

#[must_use]
pub fn body_of(kind: MessageType) -> Body {
    match kind {
        MessageType::Allocate => Body::Allocate(allocate_body()),
        MessageType::Recall => Body::Recall(recall_body()),
        MessageType::RemoteReport => Body::RemoteReport(remote_report_body()),
        MessageType::RecallSent => Body::RecallSent(recall_sent_body()),
        MessageType::ConfigUpdate => Body::ConfigUpdate(config_update_body()),
    }
}

#[must_use]
pub fn message(kind: MessageType) -> Message {
    Message {
        header: header(),
        body: body_of(kind),
    }
}

#[must_use]
pub fn encoded(kind: MessageType) -> Vec<u8> {
    protocol_types::encode_message(&message(kind)).unwrap()
}

/// Rebuilds the body hash after a body byte changed.
#[must_use]
pub fn resealed(bytes: &[u8]) -> Vec<u8> {
    use protocol_types::layout::{BODY_HASH_OFFSET, HEADER_LEN, WIDE};
    let mut sealed = bytes.to_vec();
    let hash = protocol_types::keccak256(sealed.get(HEADER_LEN..).unwrap());
    sealed
        .get_mut(BODY_HASH_OFFSET..BODY_HASH_OFFSET + WIDE)
        .unwrap()
        .copy_from_slice(&hash);
    sealed
}

fn hex_digit(byte: u8) -> u8 {
    let value = match byte {
        b'0'..=b'9' => byte.wrapping_sub(b'0'),
        b'a'..=b'f' => byte.wrapping_sub(b'a').wrapping_add(10),
        b'A'..=b'F' => byte.wrapping_sub(b'A').wrapping_add(10),
        _ => 16,
    };
    assert!(value < 16, "not a hex digit");
    value
}

#[must_use]
pub fn from_hex(text: &str) -> Vec<u8> {
    let raw = text.as_bytes();
    assert!(raw.len().is_multiple_of(2), "hex needs an even length");
    raw.chunks(2)
        .map(|pair| {
            let high = hex_digit(*pair.first().unwrap());
            let low = hex_digit(*pair.get(1).unwrap());
            high.wrapping_mul(16).wrapping_add(low)
        })
        .collect()
}

#[must_use]
pub fn to_hex(bytes: &[u8]) -> String {
    use core::fmt::Write;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}
