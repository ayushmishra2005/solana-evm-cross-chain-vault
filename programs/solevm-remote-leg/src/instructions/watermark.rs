//! Raises the lowest sequence a lane still accepts.

use anchor_lang::prelude::*;

use crate::control::{MessageClass, REPLAY_LANE_SEED, ReplayLane};
use crate::errors::RemoteLegError;
use crate::events::ReplayWatermarkAdvanced;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};

#[derive(Accounts)]
#[instruction(message_class: MessageClass)]
pub struct AdvanceReplayWatermark<'info> {
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
    pub remote_config: Account<'info, RemoteConfig>,

    #[account(
        mut,
        seeds = [
            REPLAY_LANE_SEED,
            remote_config.key().as_ref(),
            &[replay_lane.message_class.to_u8()],
            &replay_lane.lane_id.to_le_bytes(),
        ],
        bump = replay_lane.bump,
    )]
    pub replay_lane: Account<'info, ReplayLane>,
}

pub fn process_advance_replay_watermark(
    ctx: Context<AdvanceReplayWatermark>,
    message_class: MessageClass,
    new_minimum_sequence: u64,
) -> Result<()> {
    let lane = &mut ctx.accounts.replay_lane;
    require_eq!(
        lane.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require!(
        lane.message_class == message_class,
        RemoteLegError::InvalidLane
    );

    let previous_minimum_sequence = lane.minimum_acceptable_sequence;
    let highest_consumed_sequence = lane.highest_consumed_sequence;

    require_gt!(
        new_minimum_sequence,
        previous_minimum_sequence,
        RemoteLegError::InvalidWatermark
    );
    require_gte!(
        highest_consumed_sequence,
        new_minimum_sequence,
        RemoteLegError::InvalidWatermark
    );

    let lowest_allowed_highest = new_minimum_sequence
        .checked_add(lane.mandatory_watermark_lag)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    require_gte!(
        highest_consumed_sequence,
        lowest_allowed_highest,
        RemoteLegError::WatermarkLagViolation
    );

    lane.minimum_acceptable_sequence = new_minimum_sequence;
    let advanced_at = Clock::get()?.unix_timestamp;

    emit!(ReplayWatermarkAdvanced {
        remote_config: ctx.accounts.remote_config.key(),
        replay_lane: lane.key(),
        message_class,
        lane_id: lane.lane_id,
        previous_minimum_sequence,
        new_minimum_sequence,
        highest_consumed_sequence,
        advanced_at,
    });

    Ok(())
}
