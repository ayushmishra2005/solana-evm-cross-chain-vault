//! Accepts one canonical allocation and opens its transfer cycle.
//!
//! Accepting the message only authorises assets. It never counts them. An
//! allocation that may not run is consumed and held as quarantined instead.

use anchor_lang::prelude::*;

use crate::control::{
    CONSUMED_MESSAGE_SEED, ConsumedMessage, MessageClass, REPLAY_LANE_SEED, RISK_CONFIG_SEED,
    ReplayLane, RiskConfig,
};
use crate::custody::narrow_amount;
use crate::errors::RemoteLegError;
use crate::events::{AllocateAccepted, MessageQuarantined};
use crate::message::{self, ValidatedMessage};
use crate::records;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{
    QuarantineReason, REMOTE_POSITION_SEED, RemotePosition, STRATEGY_CONFIG_SEED, StrategyConfig,
    TRANSFER_RECORD_SEED, TransferKind, TransferRecord,
};

const CLASS: MessageClass = MessageClass::Allocate;

#[derive(Accounts)]
pub struct ProcessAllocate<'info> {
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
        seeds = [RISK_CONFIG_SEED, remote_config.key().as_ref()],
        bump = risk_config.bump,
    )]
    pub risk_config: Account<'info, RiskConfig>,

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

    /// CHECK: created by hand once the transfer id has been validated.
    #[account(mut)]
    pub transfer_record: UncheckedAccount<'info>,

    /// CHECK: created by hand after the watermark and commitment checks.
    #[account(mut)]
    pub consumed_message: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handle_allocate(ctx: Context<ProcessAllocate>, message_bytes: Vec<u8>) -> Result<()> {
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

    let protocol_types::Body::Allocate(body) = message.body else {
        return Err(RemoteLegError::UnsupportedMessageType.into());
    };
    require_eq!(
        body.config_version.get(),
        ctx.accounts.remote_config.config_version,
        RemoteLegError::InvalidConfigVersion
    );

    let transfer_id = *body.transfer_id.as_bytes();
    require!(
        transfer_id != [0u8; 32],
        RemoteLegError::InvalidTransferRecord
    );

    let record = TransferAddress::derive(&config_key, &transfer_id);
    require_keys_eq!(
        ctx.accounts.transfer_record.key(),
        record.address,
        RemoteLegError::InvalidTransferRecord
    );
    records::check_available(
        &ctx.accounts.transfer_record.to_account_info(),
        RemoteLegError::TransferAlreadyExists,
        RemoteLegError::InvalidTransferRecord,
    )?;

    ctx.accounts.remote_position.check_no_active_transfer()?;

    let authorized_amount = narrow_amount(body.amount.get())?;
    let minimum_amount = narrow_amount(body.minimum_destination_amount.get())?;
    require_neq!(authorized_amount, 0, RemoteLegError::InvalidTransferRecord);
    require_neq!(
        minimum_amount,
        0,
        RemoteLegError::InvalidMinimumDestinationAmount
    );
    require_gte!(
        authorized_amount,
        minimum_amount,
        RemoteLegError::InvalidMinimumDestinationAmount
    );
    let quarantine = if !fresh {
        Some(QuarantineReason::Expired)
    } else if ctx.accounts.remote_config.frozen {
        Some(QuarantineReason::Frozen)
    } else if !allocation_fits(
        &ctx.accounts.remote_position,
        &ctx.accounts.strategy_config,
        authorized_amount,
    )? {
        Some(QuarantineReason::AllocationLimit)
    } else {
        None
    };

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

    let accepted_at = Clock::get()?.unix_timestamp;
    let transfer = TransferRecord::new_allocation(
        record.bump,
        transfer_id,
        sequence,
        authorized_amount,
        minimum_amount,
        body.expected_source_balance.get(),
        accepted_at,
    );
    records::create_and_write(
        &ctx.accounts.transfer_record.to_account_info(),
        &ctx.accounts.transport_verifier.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        &[
            TRANSFER_RECORD_SEED,
            config_key.as_ref(),
            &transfer_id,
            &[record.bump],
        ],
        TransferRecord::LEN,
        &if quarantine.is_some() {
            transfer.quarantined()
        } else {
            transfer
        },
    )?;

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

    let lane = &mut ctx.accounts.control_lane;
    lane.highest_consumed_sequence = sequence;
    lane.message_commitment = message::next_commitment(&lane.message_commitment, &message_id);
    lane.last_accepted_at = accepted_at;
    let lane_commitment = lane.message_commitment;

    if let Some(reason) = quarantine {
        ctx.accounts.remote_position.quarantine_transfer(
            TransferKind::Allocate,
            transfer_id,
            sequence,
        );
        emit!(MessageQuarantined {
            remote_config: config_key,
            message_class: CLASS,
            reason,
            transfer_id,
            message_id,
            sequence,
            lane_commitment,
            quarantined_at: accepted_at,
        });
        return Ok(());
    }

    ctx.accounts
        .remote_position
        .open_transfer(TransferKind::Allocate, transfer_id, sequence);

    emit!(AllocateAccepted {
        remote_config: config_key,
        transfer_record: record.address,
        transfer_id,
        message_id,
        sequence,
        authorized_amount,
        minimum_amount,
        lane_commitment,
        accepted_at,
    });

    Ok(())
}

fn check_versions(ctx: &Context<ProcessAllocate>) -> Result<()> {
    for version in [
        ctx.accounts.remote_config.state_version,
        ctx.accounts.risk_config.state_version,
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

/// The basis points carry no vault total, so the ceiling is an absolute one.
pub fn allocation_fits(
    position: &RemotePosition,
    strategy_config: &StrategyConfig,
    authorized_amount: u64,
) -> Result<bool> {
    let projected = position
        .accepted_principal()?
        .checked_add(authorized_amount)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    Ok(projected <= strategy_config.max_remote_principal)
}

/// The canonical transfer record address, with its bump.
pub struct TransferAddress {
    pub address: Pubkey,
    pub bump: u8,
}

impl TransferAddress {
    pub fn derive(config_key: &Pubkey, transfer_id: &[u8; 32]) -> Self {
        let (address, bump) = Pubkey::find_program_address(
            &TransferRecord::seeds(config_key, transfer_id),
            &crate::ID,
        );
        Self { address, bump }
    }
}

/// The canonical consumed record address for one sequence of the control lane.
pub struct ConsumedAddress {
    pub address: Pubkey,
    pub bump: u8,
    pub lane: [u8; 4],
    pub sequence: [u8; 8],
}

impl ConsumedAddress {
    pub fn derive(config_key: &Pubkey, lane_id: u32, sequence: u64) -> Self {
        let lane = lane_id.to_le_bytes();
        let sequence = sequence.to_le_bytes();
        let (address, bump) = Pubkey::find_program_address(
            &ConsumedMessage::seeds(config_key, &lane, &sequence),
            &crate::ID,
        );
        Self {
            address,
            bump,
            lane,
            sequence,
        }
    }
}
