//! Plumbing shared by both outbound publication instructions.

use anchor_lang::prelude::*;

use crate::errors::RemoteLegError;
use crate::outbound::{
    RECALL_SENT_MARKER_SEED, REPORT_RECORD_SEED, RecallSentMarker, ReportRecord, ReportType,
};
use crate::outbound_message::BuiltMessage;
use crate::records;
use crate::state::STATE_VERSION;

/// Accounts a record creation needs.
pub struct RecordSlots<'a, 'info> {
    pub report_record: &'a AccountInfo<'info>,
    pub payer: &'a AccountInfo<'info>,
    pub system_program: &'a AccountInfo<'info>,
}

/// Accounts a marker creation needs.
pub struct MarkerSlots<'a, 'info> {
    pub marker: &'a AccountInfo<'info>,
    pub payer: &'a AccountInfo<'info>,
    pub system_program: &'a AccountInfo<'info>,
}

/// What one published message leaves behind in its record.
pub struct RecordEntry<'a> {
    pub remote_config: &'a Pubkey,
    pub lane_id: u32,
    pub sequence: u64,
    pub report_type: ReportType,
    pub built: &'a BuiltMessage,
    pub related_transfer_id: [u8; 32],
    pub epoch_id: u64,
    pub published_at: i64,
}

/// What one published notice leaves behind in its marker.
pub struct MarkerEntry<'a> {
    pub remote_config: &'a Pubkey,
    pub transfer_id: [u8; 32],
    pub sequence: u64,
    pub message_id: [u8; 32],
    pub published_at: i64,
}

/// Writes the record that proves one message left at one sequence.
///
/// The address is derived here, so a caller cannot point at another slot.
pub fn create_report_record(slots: &RecordSlots, entry: &RecordEntry) -> Result<()> {
    let (expected, bump) =
        ReportRecord::address(entry.remote_config, entry.lane_id, entry.sequence);
    require_keys_eq!(
        slots.report_record.key(),
        expected,
        RemoteLegError::InvalidReportRecord
    );
    records::check_available(
        slots.report_record,
        RemoteLegError::ReportAlreadyPublished,
        RemoteLegError::InvalidReportRecord,
    )?;

    let lane_bytes = entry.lane_id.to_le_bytes();
    let sequence_bytes = entry.sequence.to_le_bytes();
    let bump_seed = [bump];
    let seeds: [&[u8]; 5] = [
        REPORT_RECORD_SEED,
        entry.remote_config.as_ref(),
        &lane_bytes,
        &sequence_bytes,
        &bump_seed,
    ];

    records::create_and_write(
        slots.report_record,
        slots.payer,
        slots.system_program,
        &seeds,
        ReportRecord::LEN,
        &ReportRecord {
            state_version: STATE_VERSION,
            bump,
            report_type: entry.report_type,
            lane_id: entry.lane_id,
            sequence: entry.sequence,
            message_id: entry.built.message_id,
            body_hash: entry.built.body_hash,
            previous_commitment: entry.built.previous_commitment,
            next_commitment: entry.built.next_commitment,
            encoded_len: entry.built.encoded_len,
            published_at: entry.published_at,
            related_transfer_id: entry.related_transfer_id,
            epoch_id: entry.epoch_id,
        },
    )
}

/// Writes the marker that makes one recall notice publish only once.
pub fn create_recall_marker(slots: &MarkerSlots, entry: &MarkerEntry) -> Result<()> {
    let (expected, bump) = RecallSentMarker::address(entry.remote_config, &entry.transfer_id);
    require_keys_eq!(
        slots.marker.key(),
        expected,
        RemoteLegError::InvalidRecallIdentity
    );
    records::check_available(
        slots.marker,
        RemoteLegError::RecallSentAlreadyPublished,
        RemoteLegError::InvalidRecallIdentity,
    )?;

    let bump_seed = [bump];
    let seeds: [&[u8]; 4] = [
        RECALL_SENT_MARKER_SEED,
        entry.remote_config.as_ref(),
        &entry.transfer_id,
        &bump_seed,
    ];

    records::create_and_write(
        slots.marker,
        slots.payer,
        slots.system_program,
        &seeds,
        RecallSentMarker::LEN,
        &RecallSentMarker {
            state_version: STATE_VERSION,
            bump,
            transfer_id: entry.transfer_id,
            report_sequence: entry.sequence,
            message_id: entry.message_id,
            published_at: entry.published_at,
        },
    )
}
