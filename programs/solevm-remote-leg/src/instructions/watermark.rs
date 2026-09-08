//! Raises the lowest sequence a lane still accepts.

use anchor_lang::prelude::*;

use crate::control::{REPLAY_LANE_SEED, ReplayLane};
use crate::errors::RemoteLegError;
use crate::events::ReplayWatermarkAdvanced;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{REMOTE_POSITION_SEED, RemotePosition};

#[derive(Accounts)]
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
            &remote_config.control_lane_id.to_le_bytes(),
        ],
        bump = replay_lane.bump,
    )]
    pub replay_lane: Account<'info, ReplayLane>,

    /// Required when a transfer is open, so its sequence stays protected.
    #[account(
        seeds = [REMOTE_POSITION_SEED, remote_config.key().as_ref()],
        bump = remote_position.bump,
    )]
    pub remote_position: Option<Account<'info, RemotePosition>>,
}

pub fn process_advance_replay_watermark(
    ctx: Context<AdvanceReplayWatermark>,
    new_minimum_sequence: u64,
) -> Result<()> {
    let lane = &mut ctx.accounts.replay_lane;
    require_eq!(
        lane.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_eq!(
        lane.lane_id,
        ctx.accounts.remote_config.control_lane_id,
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

    check_obligation(
        new_minimum_sequence,
        ctx.accounts.remote_position.as_deref(),
    )?;

    lane.minimum_acceptable_sequence = new_minimum_sequence;
    let advanced_at = Clock::get()?.unix_timestamp;

    emit!(ReplayWatermarkAdvanced {
        remote_config: ctx.accounts.remote_config.key(),
        replay_lane: lane.key(),
        lane_id: lane.lane_id,
        previous_minimum_sequence,
        new_minimum_sequence,
        highest_consumed_sequence,
        advanced_at,
    });

    Ok(())
}

/// Keeps the replay record of an unresolved transfer out of reach.
///
/// The watermark may reach the open sequence but never pass it.
fn check_obligation(new_minimum_sequence: u64, position: Option<&RemotePosition>) -> Result<()> {
    let Some(position) = position else {
        return Ok(());
    };
    require_eq!(
        position.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );

    if position.has_active_transfer() {
        require_gte!(
            position.active_transfer_sequence,
            new_minimum_sequence,
            RemoteLegError::FinancialObligationBlocksWatermark
        );
    }
    Ok(())
}
