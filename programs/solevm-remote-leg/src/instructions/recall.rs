//! Accepts one canonical recall and reserves the custody it can already cover.
//!
//! Reserving only reclassifies a bucket. No token moves here.

use anchor_lang::prelude::*;

use crate::control::{
    CONSUMED_MESSAGE_SEED, ConsumedMessage, MessageClass, REPLAY_LANE_SEED, RISK_CONFIG_SEED,
    ReplayLane, RiskConfig,
};
use crate::custody::narrow_amount;
use crate::errors::RemoteLegError;
use crate::events::{RecallAccepted, RecallCustodyReserved};
use crate::instructions::allocate::ConsumedAddress;
use crate::message::{self, ValidatedMessage};
use crate::records;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{
    REMOTE_POSITION_SEED, RemotePosition, TRANSFER_RECORD_SEED, TransferKind, TransferRecord,
};

const CLASS: MessageClass = MessageClass::Recall;

#[derive(Accounts)]
pub struct ProcessRecall<'info> {
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
            &recall_lane.lane_id.to_le_bytes(),
        ],
        bump = recall_lane.bump,
    )]
    pub recall_lane: Account<'info, ReplayLane>,

    /// CHECK: created by hand once the transfer id has been validated.
    #[account(mut)]
    pub transfer_record: UncheckedAccount<'info>,

    /// CHECK: created by hand after the watermark and commitment checks.
    #[account(mut)]
    pub consumed_message: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handle_recall(ctx: Context<ProcessRecall>, message_bytes: Vec<u8>) -> Result<()> {
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
        &ctx.accounts.recall_lane,
        now,
    )?;

    let protocol_types::Body::Recall(body) = message.body else {
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

    let requested_amount = narrow_amount(body.requested_amount.get())?;
    let minimum_amount = narrow_amount(body.minimum_return_amount.get())?;
    require_neq!(requested_amount, 0, RemoteLegError::InvalidRecallAmount);
    require_neq!(minimum_amount, 0, RemoteLegError::InvalidMinimumReturn);
    require_gte!(
        requested_amount,
        minimum_amount,
        RemoteLegError::InvalidMinimumReturn
    );
    require_gte!(
        ctx.accounts.remote_position.accepted_principal()?,
        requested_amount,
        RemoteLegError::InsufficientRemotePrincipal
    );

    let sequence = message.header.sequence.get();
    let lane_id = ctx.accounts.recall_lane.lane_id;
    let consumed = ConsumedAddress::derive_for(CLASS, &config_key, lane_id, sequence);
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

    let reserved = requested_amount.min(ctx.accounts.remote_position.attributed_principal);
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
        &TransferRecord::new_recall(
            record.bump,
            transfer_id,
            sequence,
            requested_amount,
            minimum_amount,
            reserved,
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

    let lane = &mut ctx.accounts.recall_lane;
    lane.highest_consumed_sequence = sequence;
    lane.message_commitment = message::next_commitment(&lane.message_commitment, &message_id);
    lane.last_accepted_at = accepted_at;
    let lane_commitment = lane.message_commitment;

    let position = &mut ctx.accounts.remote_position;
    position.attributed_principal = position
        .attributed_principal
        .checked_sub(reserved)
        .ok_or(RemoteLegError::InsufficientAttributedCustody)?;
    position.recalled_custody = position
        .recalled_custody
        .checked_add(reserved)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    position.open_transfer(TransferKind::Recall, transfer_id, sequence);

    let remaining_principal = requested_amount
        .checked_sub(reserved)
        .ok_or(RemoteLegError::InvalidRecallAmount)?;

    emit!(RecallAccepted {
        remote_config: config_key,
        transfer_record: record.address,
        transfer_id,
        message_id,
        sequence,
        requested_amount,
        minimum_amount,
        lane_commitment,
        accepted_at,
    });
    emit!(RecallCustodyReserved {
        remote_config: config_key,
        transfer_record: record.address,
        transfer_id,
        reserved_amount: reserved,
        remaining_principal,
        reserved_at: accepted_at,
    });

    Ok(())
}

fn check_versions(ctx: &Context<ProcessRecall>) -> Result<()> {
    for version in [
        ctx.accounts.remote_config.state_version,
        ctx.accounts.risk_config.state_version,
        ctx.accounts.remote_position.state_version,
        ctx.accounts.recall_lane.state_version,
    ] {
        require_eq!(version, STATE_VERSION, RemoteLegError::InvalidStateVersion);
    }
    require!(
        ctx.accounts.recall_lane.message_class == CLASS,
        RemoteLegError::InvalidLane
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
