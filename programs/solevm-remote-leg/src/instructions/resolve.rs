//! Applies one canonical resolution to the quarantined open cycle.
//!
//! Admitting runs the transfer on its original terms. It never changes the
//! amounts, the identity or the custody relationships of that transfer.

use anchor_lang::prelude::*;
use protocol_types::ResolveAction;

use crate::control::{
    CONSUMED_MESSAGE_SEED, ConsumedMessage, MessageClass, REPLAY_LANE_SEED, ReplayLane,
};
use crate::errors::RemoteLegError;
use crate::events::{RecallCustodyReserved, ResolveProcessed};
use crate::instructions::allocate::{ConsumedAddress, TransferAddress, allocation_fits};
use crate::message::{self, ValidatedMessage};
use crate::records;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{
    REMOTE_POSITION_SEED, RemotePosition, ResolveOutcome, STRATEGY_CONFIG_SEED, StrategyConfig,
    TransferKind, TransferRecord, TransferStatus, reserve_recall_custody,
};

const CLASS: MessageClass = MessageClass::Resolve;

#[derive(Accounts)]
pub struct ProcessResolve<'info> {
    #[account(mut)]
    pub transport_verifier: Signer<'info>,

    #[account(
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
        has_one = transport_verifier @ RemoteLegError::Unauthorized,
    )]
    pub remote_config: Box<Account<'info, RemoteConfig>>,

    #[account(
        seeds = [STRATEGY_CONFIG_SEED, remote_config.key().as_ref()],
        bump = strategy_config.bump,
    )]
    pub strategy_config: Box<Account<'info, StrategyConfig>>,

    #[account(
        mut,
        seeds = [REMOTE_POSITION_SEED, remote_config.key().as_ref()],
        bump = remote_position.bump,
    )]
    pub remote_position: Account<'info, RemotePosition>,

    #[account(
        mut,
        seeds = [
            REPLAY_LANE_SEED,
            remote_config.key().as_ref(),
            &remote_config.control_lane_id.to_le_bytes(),
        ],
        bump = control_lane.bump,
    )]
    pub control_lane: Account<'info, ReplayLane>,

    /// CHECK: bound to the named transfer id, and read only when the position
    /// holds that transfer as its quarantined open cycle.
    #[account(mut)]
    pub transfer_record: UncheckedAccount<'info>,

    /// CHECK: created by hand after the watermark and commitment checks.
    #[account(mut)]
    pub consumed_message: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,

    /// Co-signs while the leg is frozen, so a quarantined transfer runs only
    /// with the guardian's consent.
    #[account(address = remote_config.emergency_guardian @ RemoteLegError::Unauthorized)]
    pub emergency_guardian: Option<Signer<'info>>,
}

/// What one resolution changed.
struct Admission {
    outcome: ResolveOutcome,
    reserved_amount: Option<u64>,
    remaining_principal: u64,
}

impl Admission {
    const fn unchanged(outcome: ResolveOutcome) -> Self {
        Self {
            outcome,
            reserved_amount: None,
            remaining_principal: 0,
        }
    }
}

pub fn handle_resolve(ctx: Context<ProcessResolve>, message_bytes: Vec<u8>) -> Result<()> {
    let config_key = ctx.accounts.remote_config.key();
    check_versions(&ctx)?;

    let now = message::current_time()?;
    let ValidatedMessage {
        message,
        message_id,
        fresh,
    } = message::validate_inbound(
        &message_bytes,
        CLASS,
        &ctx.accounts.remote_config,
        &ctx.accounts.control_lane,
        now,
    )?;

    let protocol_types::Body::Resolve(body) = message.body else {
        return Err(RemoteLegError::UnsupportedMessageType.into());
    };
    require_eq!(
        body.config_version.get(),
        ctx.accounts.remote_config.config_version,
        RemoteLegError::InvalidConfigVersion
    );

    let transfer_id = *body.transfer_id.as_bytes();
    let target_sequence = body.target_sequence.get();
    let target = TransferAddress::derive(&config_key, &transfer_id);
    require_keys_eq!(
        ctx.accounts.transfer_record.key(),
        target.address,
        RemoteLegError::InvalidTransferRecord
    );

    let sequence = message.header.sequence.get();
    let lane_id = ctx.accounts.remote_config.control_lane_id;
    let consumed = ConsumedAddress::derive(&config_key, lane_id, sequence);
    require_keys_eq!(
        ctx.accounts.consumed_message.key(),
        consumed.address,
        RemoteLegError::InvalidConsumedMessage
    );
    records::check_available(
        &ctx.accounts.consumed_message.to_account_info(),
        RemoteLegError::ReplayDetected,
        RemoteLegError::InvalidConsumedMessage,
    )?;

    let held = ctx
        .accounts
        .remote_position
        .holds_quarantined(&transfer_id, target_sequence);
    let transfer_kind = if held {
        ctx.accounts.remote_position.active_transfer_kind
    } else {
        TransferKind::None
    };
    let admission = if !fresh {
        Admission::unchanged(ResolveOutcome::Expired)
    } else if !held {
        Admission::unchanged(ResolveOutcome::NotQuarantined)
    } else {
        match body.action {
            ResolveAction::Admit => admit(ctx.accounts)?,
        }
    };

    records::create_and_write(
        &ctx.accounts.consumed_message.to_account_info(),
        &ctx.accounts.transport_verifier.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        &[
            CONSUMED_MESSAGE_SEED,
            config_key.as_ref(),
            &consumed.lane,
            &consumed.sequence,
            &[consumed.bump],
        ],
        ConsumedMessage::LEN,
        &ConsumedMessage {
            state_version: STATE_VERSION,
            bump: consumed.bump,
            message_class: CLASS,
            lane_id,
            sequence,
            message_id,
        },
    )?;

    let processed_at = Clock::get()?.unix_timestamp;
    let lane = &mut ctx.accounts.control_lane;
    lane.highest_consumed_sequence = sequence;
    lane.message_commitment = message::next_commitment(&lane.message_commitment, &message_id);
    lane.last_accepted_at = processed_at;
    let lane_commitment = lane.message_commitment;

    if let Some(reserved_amount) = admission.reserved_amount {
        emit!(RecallCustodyReserved {
            remote_config: config_key,
            transfer_record: target.address,
            transfer_id,
            reserved_amount,
            remaining_principal: admission.remaining_principal,
            reserved_at: processed_at,
        });
    }
    emit!(ResolveProcessed {
        remote_config: config_key,
        transfer_record: target.address,
        transfer_id,
        target_sequence,
        transfer_kind,
        outcome: admission.outcome,
        guardian_cosigned: ctx.accounts.emergency_guardian.is_some(),
        message_id,
        sequence,
        lane_commitment,
        processed_at,
    });

    Ok(())
}

/// Lets the quarantined cycle run when its original terms still hold.
///
/// A term that no longer holds leaves everything untouched, so the vault can
/// resolve again later. Only an actual admission needs the guardian.
fn admit(accounts: &mut ProcessResolve) -> Result<Admission> {
    let info = accounts.transfer_record.to_account_info();
    let mut record: TransferRecord = records::load(&info, RemoteLegError::InvalidTransferRecord)?;
    record.check_quarantined(&accounts.remote_position)?;

    match record.transfer_kind {
        TransferKind::Allocate => {
            record.check_allocation_shape()?;
            if !allocation_fits(
                &accounts.remote_position,
                &accounts.strategy_config,
                record.authorized_amount,
            )? {
                return Ok(Admission::unchanged(ResolveOutcome::AllocationLimit));
            }
        }
        TransferKind::Recall => {
            record.check_recall_shape()?;
            if accounts.remote_position.accepted_principal()? < record.requested_recall_amount {
                return Ok(Admission::unchanged(ResolveOutcome::InsufficientPrincipal));
            }
        }
        TransferKind::None => return Err(RemoteLegError::InvalidTransferKind.into()),
    }

    require!(
        !accounts.remote_config.frozen || accounts.emergency_guardian.is_some(),
        RemoteLegError::Frozen
    );

    let mut admission = Admission::unchanged(ResolveOutcome::Admitted);
    if record.transfer_kind == TransferKind::Recall {
        let reserved = reserve_recall_custody(&mut record, &mut accounts.remote_position)?;
        admission.reserved_amount = Some(reserved);
        admission.remaining_principal = record.unresolved_recall_principal()?;
    }
    record.status = TransferStatus::Pending;
    accounts.remote_position.admit_transfer();
    records::store(&info, &record)?;
    Ok(admission)
}

fn check_versions(ctx: &Context<ProcessResolve>) -> Result<()> {
    for version in [
        ctx.accounts.remote_config.state_version,
        ctx.accounts.strategy_config.state_version,
        ctx.accounts.remote_position.state_version,
        ctx.accounts.control_lane.state_version,
    ] {
        require_eq!(version, STATE_VERSION, RemoteLegError::InvalidStateVersion);
    }
    require_eq!(
        ctx.accounts.control_lane.lane_id,
        ctx.accounts.remote_config.control_lane_id,
        RemoteLegError::InvalidLane
    );
    Ok(())
}
