//! Publishing the one terminal notice of a finished recall.
//!
//! The notice says only that the leg sent the recorded amount to its own
//! configured escrow. It never claims the hub received anything.

use anchor_lang::prelude::*;

use crate::errors::RemoteLegError;
use crate::events::RecallSentPublished;
use crate::instructions::publication::{self, MarkerEntry, MarkerSlots, RecordEntry, RecordSlots};
use crate::message;
use crate::outbound::{OUTBOUND_LANE_SEED, OutboundLane, RecallSentMarker, ReportType};
use crate::outbound_message::{self, OutboundContext, RecallSentFields};
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{TRANSFER_RECORD_SEED, TransferKind, TransferRecord, TransferStatus};

use crate::control::{RISK_CONFIG_SEED, RiskConfig};

#[derive(Accounts)]
pub struct PublishRecallSent<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
    )]
    pub remote_config: Box<Account<'info, RemoteConfig>>,

    #[account(
        seeds = [RISK_CONFIG_SEED, remote_config.key().as_ref()],
        bump = risk_config.bump,
    )]
    pub risk_config: Box<Account<'info, RiskConfig>>,

    #[account(
        seeds = [
            TRANSFER_RECORD_SEED,
            remote_config.key().as_ref(),
            &transfer_record.transfer_id,
        ],
        bump = transfer_record.bump,
    )]
    pub transfer_record: Box<Account<'info, TransferRecord>>,

    #[account(
        mut,
        seeds = [
            OUTBOUND_LANE_SEED,
            remote_config.key().as_ref(),
            &remote_config.report_lane_id.to_le_bytes(),
        ],
        bump = outbound_lane.bump,
    )]
    pub outbound_lane: Box<Account<'info, OutboundLane>>,

    /// CHECK: the address is derived and the account is created here.
    #[account(mut)]
    pub report_record: UncheckedAccount<'info>,

    /// CHECK: the address is derived and the account is created here.
    #[account(mut)]
    pub recall_sent_marker: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn process_publish_recall_sent(ctx: Context<PublishRecallSent>) -> Result<()> {
    check_versions(&ctx)?;

    let record = &ctx.accounts.transfer_record;
    let config = &ctx.accounts.remote_config;
    let now = message::current_time()?;
    let sent_timestamp = check_eligible(record, now)?;
    check_unpublished(
        &ctx.accounts.recall_sent_marker,
        &config.key(),
        &record.transfer_id,
    )?;

    let built = outbound_message::build_recall_sent(
        &OutboundContext {
            config,
            lane: &ctx.accounts.outbound_lane,
            max_report_age: ctx.accounts.risk_config.max_report_age,
            now,
        },
        &RecallSentFields {
            transfer_id: record.transfer_id,
            principal_sent: record.requested_recall_amount,
            actual_amount_sent: record.assets_sent,
            realized_loss: record.realized_loss,
            destination_reference: config.outbound_escrow.to_bytes(),
            sent_timestamp,
            config_version: config.config_version,
        },
    )?;

    let published_at = Clock::get()?.unix_timestamp;
    let lane_id = ctx.accounts.outbound_lane.lane_id;
    let sequence = ctx.accounts.outbound_lane.next_sequence;
    let payer = ctx.accounts.payer.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();

    publication::create_report_record(
        &RecordSlots {
            report_record: &ctx.accounts.report_record.to_account_info(),
            payer: &payer,
            system_program: &system_program,
        },
        &RecordEntry {
            remote_config: &config.key(),
            lane_id,
            sequence,
            report_type: ReportType::RecallSent,
            built: &built,
            related_transfer_id: record.transfer_id,
            epoch_id: 0,
            published_at,
        },
    )?;

    publication::create_recall_marker(
        &MarkerSlots {
            marker: &ctx.accounts.recall_sent_marker.to_account_info(),
            payer: &payer,
            system_program: &system_program,
        },
        &MarkerEntry {
            remote_config: &config.key(),
            transfer_id: record.transfer_id,
            sequence,
            message_id: built.message_id,
            published_at,
        },
    )?;

    ctx.accounts.outbound_lane.advance(
        ReportType::RecallSent,
        built.next_commitment,
        published_at,
    )?;

    emit!(RecallSentPublished {
        remote_config: config.key(),
        report_record: ctx.accounts.report_record.key(),
        transfer_id: record.transfer_id,
        lane_id,
        sequence,
        message_id: built.message_id,
        body_hash: built.body_hash,
        previous_commitment: built.previous_commitment,
        next_commitment: built.next_commitment,
        encoded_len: built.encoded_len,
        message_bytes: built.encoded.as_bytes().to_vec(),
        published_at,
    });

    Ok(())
}

/// Rejects a repeat before the message is built, so a retry stays cheap.
fn check_unpublished(
    marker: &UncheckedAccount,
    remote_config: &Pubkey,
    transfer_id: &[u8; 32],
) -> Result<()> {
    let expected = RecallSentMarker::address(remote_config, transfer_id).0;
    require_keys_eq!(
        marker.key(),
        expected,
        RemoteLegError::InvalidRecallIdentity
    );
    require!(
        marker.to_account_info().owner != &crate::ID,
        RemoteLegError::RecallSentAlreadyPublished
    );
    Ok(())
}

/// Accepts only a recall that has fully unwound, and returns its send time.
fn check_eligible(record: &TransferRecord, now: u64) -> Result<u64> {
    require!(
        record.transfer_kind == TransferKind::Recall,
        RemoteLegError::InvalidRecallIdentity
    );
    require!(
        record.status == TransferStatus::Complete,
        RemoteLegError::RecallNotComplete
    );

    let settled = record
        .assets_sent
        .checked_add(record.realized_loss)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    require_eq!(
        settled,
        record.requested_recall_amount,
        RemoteLegError::RecallNotComplete
    );

    require_neq!(
        record.completed_at,
        0,
        RemoteLegError::InvalidPublicationTimestamp
    );
    let sent_timestamp = u64::try_from(record.completed_at)
        .map_err(|_| RemoteLegError::InvalidPublicationTimestamp)?;
    require_gte!(
        now,
        sent_timestamp,
        RemoteLegError::InvalidPublicationTimestamp
    );

    Ok(sent_timestamp)
}

fn check_versions(ctx: &Context<PublishRecallSent>) -> Result<()> {
    require_eq!(
        ctx.accounts.remote_config.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_eq!(
        ctx.accounts.risk_config.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_eq!(
        ctx.accounts.transfer_record.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_eq!(
        ctx.accounts.outbound_lane.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_eq!(
        ctx.accounts.outbound_lane.lane_id,
        ctx.accounts.remote_config.report_lane_id,
        RemoteLegError::InvalidOutboundLane
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::MessageClass;

    fn recall(sent: u64, loss: u64, requested: u64) -> TransferRecord {
        TransferRecord {
            state_version: STATE_VERSION,
            bump: 254,
            transfer_kind: TransferKind::Recall,
            message_class: MessageClass::Recall,
            status: TransferStatus::Complete,
            transfer_id: [0x41; 32],
            message_sequence: 3,
            authorized_amount: 0,
            attributed_amount: 0,
            requested_recall_amount: requested,
            minimum_amount: 0,
            custody_principal_reserved: 0,
            strategy_principal_resolved: requested,
            assets_withdrawn: sent,
            assets_sent: sent,
            realized_loss: loss,
            expected_source_balance: 0,
            created_at: 100,
            completed_at: 500,
        }
    }

    #[test]
    fn a_fully_unwound_recall_is_eligible() {
        assert_eq!(check_eligible(&recall(900, 100, 1_000), 900), Ok(500));
    }

    #[test]
    fn a_total_loss_recall_is_eligible_even_though_nothing_was_sent() {
        assert_eq!(check_eligible(&recall(0, 1_000, 1_000), 900), Ok(500));
    }

    #[test]
    fn an_allocation_is_never_a_recall_notice() {
        let mut record = recall(900, 100, 1_000);
        record.transfer_kind = TransferKind::Allocate;
        assert_eq!(
            check_eligible(&record, 900),
            Err(RemoteLegError::InvalidRecallIdentity.into())
        );
    }

    #[test]
    fn a_pending_recall_is_not_eligible() {
        let mut record = recall(900, 100, 1_000);
        record.status = TransferStatus::Pending;
        assert_eq!(
            check_eligible(&record, 900),
            Err(RemoteLegError::RecallNotComplete.into())
        );
    }

    #[test]
    fn a_partial_send_is_not_eligible() {
        assert_eq!(
            check_eligible(&recall(500, 100, 1_000), 900),
            Err(RemoteLegError::RecallNotComplete.into())
        );
    }

    #[test]
    fn a_record_without_a_completion_time_is_not_eligible() {
        let mut record = recall(900, 100, 1_000);
        record.completed_at = 0;
        assert_eq!(
            check_eligible(&record, 900),
            Err(RemoteLegError::InvalidPublicationTimestamp.into())
        );
    }

    #[test]
    fn a_completion_in_the_future_is_not_eligible() {
        assert_eq!(
            check_eligible(&recall(900, 100, 1_000), 499),
            Err(RemoteLegError::InvalidPublicationTimestamp.into())
        );
    }

    #[test]
    fn a_completion_exactly_now_is_eligible() {
        assert_eq!(check_eligible(&recall(900, 100, 1_000), 500), Ok(500));
    }
}
