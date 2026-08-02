//! Fixtures shared by the simulator integration tests.

#![allow(unreachable_pub, dead_code, clippy::unwrap_used)]

use protocol_types::{
    AllocateBody, ApplicationId, AssetAmount, Body, ChainId, Commitment, ConfigVersion,
    DeploymentId, EpochId, Flags, Header, LaneId, Message, MessageType, PROTOCOL_VERSION,
    ProbeStatus, RecallBody, RemoteReportBody, SCHEMA_VERSION, Sequence, Timestamp, TransferId,
    VaultId, encode_message,
};
use xchain_sim::{
    AssetRequest, ControlRequest, EndpointId, Fault, FaultAction, FaultId, FaultTarget, Simulator,
    SimulatorConfig, Tick,
};

pub const HUB: EndpointId = EndpointId::new(1);
pub const LEG: EndpointId = EndpointId::new(2);
pub const WATCHER: EndpointId = EndpointId::new(3);

pub const OBSERVED_AT: u64 = 1_700_000_000;
pub const PUBLISHED_AT: u64 = 1_700_000_060;
pub const EXPIRES_AT: u64 = 1_700_003_600;

#[must_use]
pub fn simulator() -> Simulator {
    Simulator::new(&[HUB, LEG, WATCHER]).unwrap()
}

#[must_use]
pub fn simulator_with(config: SimulatorConfig) -> Simulator {
    Simulator::with_config(config, &[HUB, LEG, WATCHER]).unwrap()
}

#[must_use]
pub fn header(sequence: u64) -> Header {
    Header {
        protocol_version: PROTOCOL_VERSION,
        schema_version: SCHEMA_VERSION,
        flags: Flags::NONE,
        source_chain: ChainId::new(1),
        destination_chain: ChainId::new(2),
        source_application: ApplicationId::from_evm_address([0xA1; 20]),
        destination_application: ApplicationId::from_solana_pubkey([0xB2; 32]),
        deployment_id: DeploymentId::new([0xD0; 32]),
        vault_id: VaultId::new([0x0E; 32]),
        lane_id: LaneId::new(7),
        sequence: Sequence::new(sequence),
        previous_commitment: Commitment::new([0xC1; 32]),
        observed_at: Timestamp::new(OBSERVED_AT),
        published_at: Timestamp::new(PUBLISHED_AT),
        expires_at: Timestamp::new(EXPIRES_AT),
    }
}

#[must_use]
pub fn allocate_body() -> Body {
    Body::Allocate(AllocateBody {
        transfer_id: TransferId::new([0x11; 32]),
        amount: AssetAmount::new(1_000_000),
        expected_source_balance: AssetAmount::new(5_000_000),
        minimum_destination_amount: AssetAmount::new(999_000),
        deadline: Timestamp::new(PUBLISHED_AT + 900),
        config_version: ConfigVersion::new(4),
    })
}

#[must_use]
pub fn recall_body() -> Body {
    Body::Recall(RecallBody {
        transfer_id: TransferId::new([0x33; 32]),
        requested_amount: AssetAmount::new(750_000),
        minimum_return_amount: AssetAmount::new(740_000),
        deadline: Timestamp::new(PUBLISHED_AT + 1_800),
        config_version: ConfigVersion::new(9),
    })
}

#[must_use]
pub fn report_body() -> Body {
    Body::RemoteReport(RemoteReportBody {
        epoch_id: EpochId::new(12),
        remote_principal: AssetAmount::new(2_000_000),
        reported_value: AssetAmount::new(2_050_000),
        realized_loss: AssetAmount::new(1_500),
        unattributed_balance: AssetAmount::new(25),
        latest_completed_transfer_id: TransferId::new([0x77; 32]),
        probe_status: ProbeStatus::Fresh,
        probe_timestamp: Timestamp::new(OBSERVED_AT - 120),
        config_version: ConfigVersion::new(4),
        remote_state_commitment: Commitment::new([0x88; 32]),
    })
}

#[must_use]
pub fn body_of(kind: MessageType) -> Body {
    match kind {
        MessageType::Recall => recall_body(),
        MessageType::RemoteReport => report_body(),
        _ => allocate_body(),
    }
}

#[must_use]
pub fn message(sequence: u64) -> Message {
    Message {
        header: header(sequence),
        body: allocate_body(),
    }
}

/// Canonical bytes of an allocate message with the given sequence number.
#[must_use]
pub fn canonical(sequence: u64) -> Vec<u8> {
    encode_message(&message(sequence)).unwrap()
}

#[must_use]
pub fn canonical_of(kind: MessageType, sequence: u64) -> Vec<u8> {
    encode_message(&Message {
        header: header(sequence),
        body: body_of(kind),
    })
    .unwrap()
}

#[must_use]
pub fn transfer(tag: u8) -> TransferId {
    TransferId::new([tag; 32])
}

#[must_use]
pub fn amount(value: u128) -> AssetAmount {
    AssetAmount::new(value)
}

#[must_use]
pub fn control_at(tick: u64, sequence: u64) -> ControlRequest {
    ControlRequest::new(HUB, LEG, canonical(sequence), Tick::new(tick))
}

#[must_use]
pub fn asset_at(tick: u64, tag: u8, value: u128) -> AssetRequest {
    AssetRequest::new(transfer(tag), HUB, LEG, amount(value), Tick::new(tick))
}

#[must_use]
pub fn fault(id: u32, target: FaultTarget, action: FaultAction) -> Fault {
    Fault::new(FaultId::new(id), target, action)
}

/// Offset of the first body byte, which is safe to corrupt in a test.
#[must_use]
pub fn first_body_offset() -> usize {
    protocol_types::layout::HEADER_LEN
}

/// The stored body hash of a canonical message.
#[must_use]
pub fn body_hash_bytes(bytes: &[u8]) -> Vec<u8> {
    use protocol_types::layout::{BODY_HASH_OFFSET, WIDE};
    bytes
        .get(BODY_HASH_OFFSET..BODY_HASH_OFFSET + WIDE)
        .unwrap()
        .to_vec()
}
