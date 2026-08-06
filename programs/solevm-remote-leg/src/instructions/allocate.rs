//! Accepts one canonical allocation and opens its transfer cycle.
//!
//! Accepting the message only authorises assets. It never counts them.

use anchor_lang::prelude::*;

use crate::control::{
    CONSUMED_MESSAGE_SEED, ConsumedMessage, MessageClass, REPLAY_LANE_SEED, RISK_CONFIG_SEED,
    ReplayLane, RiskConfig,
};
use crate::custody::narrow_amount;
use crate::errors::RemoteLegError;
use crate::events::AllocateAccepted;
use crate::message::{self, ValidatedMessage};
use crate::records;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{
    REMOTE_POSITION_SEED, RemotePosition, STRATEGY_CONFIG_SEED, StrategyConfig,
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
            &[CLASS.to_u8()],
            &allocate_lane.lane_id.to_le_bytes(),
        ],
        bump = allocate_lane.bump,
    )]
    pub allocate_lane: Account<'info, ReplayLane>,

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
    require!(!ctx.accounts.remote_config.frozen, RemoteLegError::Frozen);

    let now = message::current_time()?;
    let ValidatedMessage {
        message,
        message_id,
    } = message::validate_inbound(
        &message_bytes,
        CLASS,
        &ctx.accounts.remote_config,
        &ctx.accounts.allocate_lane,
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
    check_allocation_limit(&ctx, authorized_amount)?;

    let sequence = message.header.sequence.get();
    let lane_id = ctx.accounts.allocate_lane.lane_id;
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
        &TransferRecord::new_allocation(
            record.bump,
            transfer_id,
            sequence,
            authorized_amount,
            minimum_amount,
            body.expected_source_balance.get(),
            accepted_at,
        ),
    )?;

    records::create_and_write(
        &ctx.accounts.consumed_message.to_account_info(),
        &ctx.accounts.transport_verifier.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        &[
            CONSUMED_MESSAGE_SEED,
            config_key.as_ref(),
            &consumed.class,
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

    let lane = &mut ctx.accounts.allocate_lane;
    lane.highest_consumed_sequence = sequence;
    lane.message_commitment = message::next_commitment(&lane.message_commitment, &message_id);
    lane.last_accepted_at = accepted_at;
    let lane_commitment = lane.message_commitment;

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
        ctx.accounts.allocate_lane.state_version,
    ] {
        require_eq!(version, STATE_VERSION, RemoteLegError::InvalidStateVersion);
    }
    require!(
        ctx.accounts.allocate_lane.message_class == CLASS,
        RemoteLegError::InvalidLane
    );
    Ok(())
}

/// The basis points carry no vault total, so the ceiling is an absolute one.
fn check_allocation_limit(ctx: &Context<ProcessAllocate>, authorized_amount: u64) -> Result<()> {
    let accepted = ctx.accounts.remote_position.accepted_principal()?;
    let projected = accepted
        .checked_add(authorized_amount)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    require_gte!(
        ctx.accounts.strategy_config.max_remote_principal,
        projected,
        RemoteLegError::RemoteAllocationLimitExceeded
    );
    Ok(())
}

/// The canonical transfer record address, with its bump.
struct TransferAddress {
    address: Pubkey,
    bump: u8,
}

impl TransferAddress {
    fn derive(config_key: &Pubkey, transfer_id: &[u8; 32]) -> Self {
        let (address, bump) = Pubkey::find_program_address(
            &TransferRecord::seeds(config_key, transfer_id),
            &crate::ID,
        );
        Self { address, bump }
    }
}

/// The canonical consumed record address for one sequence of this class.
pub struct ConsumedAddress {
    pub address: Pubkey,
    pub bump: u8,
    pub class: [u8; 1],
    pub lane: [u8; 4],
    pub sequence: [u8; 8],
}

impl ConsumedAddress {
    pub fn derive_for(
        class: MessageClass,
        config_key: &Pubkey,
        lane_id: u32,
        sequence: u64,
    ) -> Self {
        let class = [class.to_u8()];
        let lane = lane_id.to_le_bytes();
        let sequence = sequence.to_le_bytes();
        let (address, bump) = Pubkey::find_program_address(
            &ConsumedMessage::seeds(config_key, &class, &lane, &sequence),
            &crate::ID,
        );
        Self {
            address,
            bump,
            class,
            lane,
            sequence,
        }
    }

    fn derive(config_key: &Pubkey, lane_id: u32, sequence: u64) -> Self {
        Self::derive_for(CLASS, config_key, lane_id, sequence)
    }
}
