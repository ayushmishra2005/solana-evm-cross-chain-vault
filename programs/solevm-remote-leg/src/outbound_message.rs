//! Building the canonical messages the leg publishes.
//!
//! Every outbound message is encoded by the shared protocol crate, so this
//! module only chooses field values and reverses the inbound direction.

use anchor_lang::prelude::*;
use protocol_types::{
    ApplicationId, AssetAmount, Body, ChainId, Commitment, ConfigVersion, DeploymentId,
    DestinationReference, EncodeError, EncodedMessage, EpochId, Flags, Header, LaneId, Message,
    PROTOCOL_VERSION, ProbeStatus, RecallSentBody, RemoteReportBody, SCHEMA_VERSION, Sequence,
    SettlementKind, Timestamp, TransferId, TransportSettlementBody, VaultId, layout,
};

use crate::errors::RemoteLegError;
use crate::message;
use crate::outbound::OutboundLane;
use crate::state::RemoteConfig;

/// Largest canonical message the transport carries.
pub const MAX_REPORT_LEN: usize = 400;

/// The leg runs no liquidity probe, so every report says so.
pub const NO_PROBE_STATUS: ProbeStatus = ProbeStatus::NotRequired;

/// One encoded message with every identifier it produces.
pub struct BuiltMessage {
    pub encoded: EncodedMessage,
    pub message_id: [u8; 32],
    pub body_hash: [u8; 32],
    pub previous_commitment: [u8; 32],
    pub next_commitment: [u8; 32],
    pub encoded_len: u16,
}

/// Everything the outbound header needs, taken from state only.
pub struct OutboundContext<'a> {
    pub config: &'a RemoteConfig,
    pub lane: &'a OutboundLane,
    pub max_report_age: u64,
    pub now: u64,
}

/// Values of one report body that come from the position and custody.
pub struct ReportFields {
    pub epoch_id: u64,
    pub remote_principal: u64,
    pub realized_loss: u64,
    pub unattributed_balance: u64,
    pub latest_completed_transfer_id: [u8; 32],
    pub config_version: u64,
    pub remote_state_commitment: [u8; 32],
}

/// Values of one recall notice, all taken from the completed record.
pub struct RecallSentFields {
    pub transfer_id: [u8; 32],
    pub principal_sent: u64,
    pub actual_amount_sent: u64,
    pub realized_loss: u64,
    pub destination_reference: [u8; 32],
    pub sent_timestamp: u64,
    pub config_version: u64,
}

/// Values of one transport settlement, taken from the record and the ledger.
pub struct TransportSettlementFields {
    pub transfer_id: [u8; 32],
    pub kind: SettlementKind,
    pub claim_amount: u128,
    pub credited_amount: u64,
    pub impaired_outstanding: u128,
    pub config_version: u64,
}

/// Maps a shared encode failure onto one typed program error.
fn encode_error(error: EncodeError) -> RemoteLegError {
    match error {
        EncodeError::BufferTooSmall { .. } => RemoteLegError::ReportTooLarge,
        _ => RemoteLegError::InvalidOutboundMessage,
    }
}

/// Builds the header that sends a message back the way it came.
fn header(context: &OutboundContext) -> Result<Header> {
    let config = context.config;
    let expires_at = context
        .now
        .checked_add(context.max_report_age)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;

    Ok(Header {
        protocol_version: PROTOCOL_VERSION,
        schema_version: SCHEMA_VERSION,
        flags: Flags::new(0),
        source_chain: ChainId::new(config.destination_chain_id),
        destination_chain: ChainId::new(config.source_chain_id),
        source_application: ApplicationId::new(config.local_application_id),
        destination_application: ApplicationId::new(config.source_application_id),
        deployment_id: DeploymentId::new(config.deployment_id),
        vault_id: VaultId::new(config.vault_id),
        lane_id: LaneId::new(config.report_lane_id),
        sequence: Sequence::new(context.lane.next_sequence),
        previous_commitment: Commitment::new(context.lane.current_commitment),
        observed_at: Timestamp::new(context.now),
        published_at: Timestamp::new(context.now),
        expires_at: Timestamp::new(expires_at),
    })
}

/// Reads the body hash the encoder already wrote into the header.
///
/// Hashing the body again would cost the same work twice.
fn read_body_hash(encoded: &[u8]) -> Result<[u8; 32]> {
    let slot = encoded
        .get(layout::BODY_HASH_OFFSET..layout::HEADER_LEN)
        .ok_or(RemoteLegError::InvalidEncodedLength)?;
    slot.try_into()
        .map_err(|_| RemoteLegError::InvalidEncodedLength.into())
}

/// Encodes one message and derives every identifier from its exact bytes.
fn build(context: &OutboundContext, body: Body, expected_len: usize) -> Result<BuiltMessage> {
    let message = Message {
        header: header(context)?,
        body,
    };

    let encoded = message.encode().map_err(encode_error)?;
    let bytes = encoded.as_bytes();

    require_eq!(
        bytes.len(),
        expected_len,
        RemoteLegError::InvalidEncodedLength
    );
    require_gte!(MAX_REPORT_LEN, bytes.len(), RemoteLegError::ReportTooLarge);

    let message_id = message::message_id(bytes)?;
    let body_hash = read_body_hash(bytes)?;
    let next_commitment = message::next_commitment(&context.lane.current_commitment, &message_id);
    let encoded_len = u16::try_from(bytes.len()).map_err(|_| RemoteLegError::ReportTooLarge)?;

    Ok(BuiltMessage {
        encoded,
        message_id,
        body_hash,
        previous_commitment: context.lane.current_commitment,
        next_commitment,
        encoded_len,
    })
}

/// Builds one canonical RemoteReport.
///
/// Schema version one never reports more than the principal it holds.
pub fn build_remote_report(
    context: &OutboundContext,
    fields: &ReportFields,
) -> Result<BuiltMessage> {
    let body = RemoteReportBody {
        epoch_id: EpochId::new(fields.epoch_id),
        remote_principal: AssetAmount::new(u128::from(fields.remote_principal)),
        reported_value: AssetAmount::new(u128::from(fields.remote_principal)),
        realized_loss: AssetAmount::new(u128::from(fields.realized_loss)),
        unattributed_balance: AssetAmount::new(u128::from(fields.unattributed_balance)),
        latest_completed_transfer_id: TransferId::new(fields.latest_completed_transfer_id),
        probe_status: NO_PROBE_STATUS,
        probe_timestamp: Timestamp::ZERO,
        config_version: ConfigVersion::new(fields.config_version),
        remote_state_commitment: Commitment::new(fields.remote_state_commitment),
    };

    build(
        context,
        Body::RemoteReport(body),
        layout::REMOTE_REPORT_MESSAGE_LEN,
    )
}

/// Builds one canonical RecallSent.
pub fn build_recall_sent(
    context: &OutboundContext,
    fields: &RecallSentFields,
) -> Result<BuiltMessage> {
    let body = RecallSentBody {
        transfer_id: TransferId::new(fields.transfer_id),
        principal_sent: AssetAmount::new(u128::from(fields.principal_sent)),
        actual_amount_sent: AssetAmount::new(u128::from(fields.actual_amount_sent)),
        realized_loss: AssetAmount::new(u128::from(fields.realized_loss)),
        destination_reference: DestinationReference::new(fields.destination_reference),
        sent_timestamp: Timestamp::new(fields.sent_timestamp),
        config_version: ConfigVersion::new(fields.config_version),
    };

    build(
        context,
        Body::RecallSent(body),
        layout::RECALL_SENT_MESSAGE_LEN,
    )
}

/// Builds one canonical TransportSettlement.
pub fn build_transport_settlement(
    context: &OutboundContext,
    fields: &TransportSettlementFields,
) -> Result<BuiltMessage> {
    let body = TransportSettlementBody {
        transfer_id: TransferId::new(fields.transfer_id),
        kind: fields.kind,
        claim_amount: AssetAmount::new(fields.claim_amount),
        credited_amount: AssetAmount::new(u128::from(fields.credited_amount)),
        impaired_outstanding: AssetAmount::new(fields.impaired_outstanding),
        config_version: ConfigVersion::new(fields.config_version),
    };

    build(
        context,
        Body::TransportSettlement(body),
        layout::TRANSPORT_SETTLEMENT_MESSAGE_LEN,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transport_settlement_stays_inside_the_transport_budget() {
        const { assert!(layout::TRANSPORT_SETTLEMENT_MESSAGE_LEN <= MAX_REPORT_LEN) };
    }

    #[test]
    fn a_remote_report_fits_the_transport_budget_exactly() {
        assert_eq!(layout::REMOTE_REPORT_MESSAGE_LEN, MAX_REPORT_LEN);
    }

    #[test]
    fn a_recall_notice_stays_inside_the_transport_budget() {
        const { assert!(layout::RECALL_SENT_MESSAGE_LEN <= MAX_REPORT_LEN) };
    }

    #[test]
    fn the_no_probe_status_is_the_canonical_zero() {
        assert_eq!(NO_PROBE_STATUS.to_u8(), 0);
        assert_eq!(NO_PROBE_STATUS, ProbeStatus::NotRequired);
    }

    #[test]
    fn a_buffer_failure_and_a_field_failure_map_to_their_own_errors() {
        assert_eq!(
            Error::from(encode_error(EncodeError::BufferTooSmall {
                needed: 400,
                available: 8,
            })),
            Error::from(RemoteLegError::ReportTooLarge)
        );
        assert_eq!(
            Error::from(encode_error(EncodeError::Invalid(
                protocol_types::ValidationError::ReservedFlagsSet
            ))),
            Error::from(RemoteLegError::InvalidOutboundMessage)
        );
    }
}
