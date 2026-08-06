//! Applies one canonical configuration message to the risk limits.

use anchor_lang::prelude::*;

use crate::control::{
    CONSUMED_MESSAGE_SEED, ConsumedMessage, MessageClass, REPLAY_LANE_SEED, RISK_CONFIG_SEED,
    ReplayLane, RiskConfig,
};
use crate::errors::RemoteLegError;
use crate::events::ConfigUpdated;
use crate::instructions::allocate::ConsumedAddress;
use crate::message::{self, ValidatedMessage};
use crate::records;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};

const CLASS: MessageClass = MessageClass::ConfigUpdate;

#[derive(Accounts)]
pub struct ProcessConfigUpdate<'info> {
    #[account(mut)]
    pub transport_verifier: Signer<'info>,

    #[account(
        mut,
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
        has_one = transport_verifier @ RemoteLegError::Unauthorized,
    )]
    pub remote_config: Account<'info, RemoteConfig>,

    #[account(
        mut,
        seeds = [RISK_CONFIG_SEED, remote_config.key().as_ref()],
        bump = risk_config.bump,
    )]
    pub risk_config: Account<'info, RiskConfig>,

    #[account(
        mut,
        seeds = [
            REPLAY_LANE_SEED,
            remote_config.key().as_ref(),
            &[CLASS.to_u8()],
            &config_update_lane.lane_id.to_le_bytes(),
        ],
        bump = config_update_lane.bump,
    )]
    pub config_update_lane: Account<'info, ReplayLane>,

    /// Created by hand after replay validation, so the watermark is checked
    /// before any account lookup can reject the message.
    #[account(mut)]
    pub consumed_message: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handle_config_update(
    ctx: Context<ProcessConfigUpdate>,
    message_bytes: Vec<u8>,
) -> Result<()> {
    let config_key = ctx.accounts.remote_config.key();

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
        ctx.accounts.config_update_lane.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require!(
        ctx.accounts.config_update_lane.message_class == CLASS,
        RemoteLegError::InvalidLane
    );
    require_eq!(
        ctx.accounts.risk_config.config_version,
        ctx.accounts.remote_config.config_version,
        RemoteLegError::InvalidRiskConfig
    );
    require!(!ctx.accounts.remote_config.frozen, RemoteLegError::Frozen);

    let now = message::current_time()?;
    let ValidatedMessage {
        message,
        message_id,
    } = message::validate_inbound(
        &message_bytes,
        CLASS,
        &ctx.accounts.remote_config,
        &ctx.accounts.config_update_lane,
        now,
    )?;

    let protocol_types::Body::ConfigUpdate(body) = message.body else {
        return Err(RemoteLegError::UnsupportedMessageType.into());
    };
    let sequence = message.header.sequence.get();
    let lane_id = ctx.accounts.config_update_lane.lane_id;

    let record = ConsumedAddress::derive_for(CLASS, &config_key, lane_id, sequence);
    require_keys_eq!(
        ctx.accounts.consumed_message.key(),
        record.address,
        RemoteLegError::InvalidConsumedMessage
    );
    records::check_available(
        &ctx.accounts.consumed_message.to_account_info(),
        RemoteLegError::ReplayDetected,
        RemoteLegError::InvalidConsumedMessage,
    )?;

    let previous_config_version = ctx.accounts.remote_config.config_version;
    let new_config_version = body.config_version.get();
    require_eq!(
        body.previous_config_version.get(),
        previous_config_version,
        RemoteLegError::InvalidConfigVersion
    );
    require_gt!(
        new_config_version,
        previous_config_version,
        RemoteLegError::InvalidConfigVersion
    );
    require_gte!(
        now,
        body.effective_timestamp.get(),
        RemoteLegError::ConfigNotEffective
    );

    let max_remote_allocation_bps = body.max_remote_allocation_bps.get();
    let max_upward_deviation_bps = body.max_upward_deviation_bps.get();
    let max_downward_deviation_bps = body.max_downward_deviation_bps.get();
    let config_commitment = *body.config_commitment.as_bytes();
    RiskConfig::check_parameters(
        max_remote_allocation_bps,
        max_upward_deviation_bps,
        max_downward_deviation_bps,
        body.max_report_age,
        &config_commitment,
    )?;

    let updated_at = Clock::get()?.unix_timestamp;
    let risk_config = &mut ctx.accounts.risk_config;
    risk_config.max_remote_allocation_bps = max_remote_allocation_bps;
    risk_config.max_upward_deviation_bps = max_upward_deviation_bps;
    risk_config.max_downward_deviation_bps = max_downward_deviation_bps;
    risk_config.max_report_age = body.max_report_age;
    risk_config.config_commitment = config_commitment;
    risk_config.config_version = new_config_version;
    risk_config.last_update_at = updated_at;

    ctx.accounts.remote_config.config_version = new_config_version;

    records::create_and_write(
        &ctx.accounts.consumed_message.to_account_info(),
        &ctx.accounts.transport_verifier.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        &[
            CONSUMED_MESSAGE_SEED,
            config_key.as_ref(),
            &record.class,
            &record.lane,
            &record.sequence,
            &[record.bump],
        ],
        ConsumedMessage::LEN,
        &ConsumedMessage {
            state_version: STATE_VERSION,
            bump: record.bump,
            message_class: CLASS,
            lane_id,
            sequence,
            message_id,
        },
    )?;

    let lane = &mut ctx.accounts.config_update_lane;
    lane.highest_consumed_sequence = sequence;
    lane.message_commitment = message::next_commitment(&lane.message_commitment, &message_id);
    lane.last_accepted_at = updated_at;
    let lane_commitment = lane.message_commitment;

    emit!(ConfigUpdated {
        remote_config: config_key,
        risk_config: ctx.accounts.risk_config.key(),
        message_id,
        sequence,
        previous_config_version,
        new_config_version,
        lane_commitment,
        updated_at,
    });

    Ok(())
}
