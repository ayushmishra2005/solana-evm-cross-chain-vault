//! One time setup of the outbound report lane.

use anchor_lang::prelude::*;

use crate::errors::RemoteLegError;
use crate::events::OutboundStateInitialized;
use crate::outbound::{FIRST_OUTBOUND_SEQUENCE, OUTBOUND_LANE_SEED, OutboundLane};
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};

#[derive(Accounts)]
pub struct InitializeOutboundState<'info> {
    #[account(mut)]
    pub administrator: Signer<'info>,

    #[account(
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
        has_one = administrator @ RemoteLegError::Unauthorized,
    )]
    pub remote_config: Box<Account<'info, RemoteConfig>>,

    #[account(
        init,
        payer = administrator,
        space = OutboundLane::LEN,
        seeds = [
            OUTBOUND_LANE_SEED,
            remote_config.key().as_ref(),
            &remote_config.report_lane_id.to_le_bytes(),
        ],
        bump,
    )]
    pub outbound_lane: Account<'info, OutboundLane>,

    pub system_program: Program<'info, System>,
}

pub fn process_initialize_outbound_state(ctx: Context<InitializeOutboundState>) -> Result<()> {
    let config = &ctx.accounts.remote_config;
    require_eq!(
        config.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require!(!config.frozen, RemoteLegError::Frozen);

    let initialized_at = Clock::get()?.unix_timestamp;
    let lane_id = config.report_lane_id;

    ctx.accounts.outbound_lane.set_inner(OutboundLane {
        state_version: STATE_VERSION,
        bump: ctx.bumps.outbound_lane,
        lane_id,
        next_sequence: FIRST_OUTBOUND_SEQUENCE,
        current_commitment: [0u8; 32],
        last_publication_at: 0,
        total_reports: 0,
        total_recall_notifications: 0,
        initialized_at,
    });

    emit!(OutboundStateInitialized {
        remote_config: config.key(),
        outbound_lane: ctx.accounts.outbound_lane.key(),
        lane_id,
        next_sequence: FIRST_OUTBOUND_SEQUENCE,
        initialized_at,
    });

    Ok(())
}
