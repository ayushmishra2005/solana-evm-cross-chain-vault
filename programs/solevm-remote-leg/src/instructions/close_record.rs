//! Returns the rent of a record the watermark has already passed.

use anchor_lang::prelude::*;

use crate::control::{CONSUMED_MESSAGE_SEED, ConsumedMessage, REPLAY_LANE_SEED, ReplayLane};
use crate::errors::RemoteLegError;
use crate::events::ConsumedMessageClosed;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};

#[derive(Accounts)]
pub struct CloseConsumedMessage<'info> {
    #[account(
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
        has_one = administrator @ RemoteLegError::InvalidRentDestination,
    )]
    pub remote_config: Account<'info, RemoteConfig>,

    /// CHECK: the configuration fixes this key, so the caller cannot pick it.
    #[account(mut)]
    pub administrator: UncheckedAccount<'info>,

    #[account(
        seeds = [
            REPLAY_LANE_SEED,
            remote_config.key().as_ref(),
            &[replay_lane.message_class.to_u8()],
            &replay_lane.lane_id.to_le_bytes(),
        ],
        bump = replay_lane.bump,
    )]
    pub replay_lane: Account<'info, ReplayLane>,

    #[account(
        mut,
        close = administrator,
        seeds = [
            CONSUMED_MESSAGE_SEED,
            remote_config.key().as_ref(),
            &[consumed_message.message_class.to_u8()],
            &consumed_message.lane_id.to_le_bytes(),
            &consumed_message.sequence.to_le_bytes(),
        ],
        bump = consumed_message.bump,
    )]
    pub consumed_message: Account<'info, ConsumedMessage>,
}

pub fn process_close_consumed_message(ctx: Context<CloseConsumedMessage>) -> Result<()> {
    let record = &ctx.accounts.consumed_message;
    let lane = &ctx.accounts.replay_lane;

    require_eq!(
        record.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require!(
        record.message_class == lane.message_class,
        RemoteLegError::InvalidConsumedMessage
    );
    require_eq!(
        record.lane_id,
        lane.lane_id,
        RemoteLegError::InvalidConsumedMessage
    );
    require_gt!(
        lane.minimum_acceptable_sequence,
        record.sequence,
        RemoteLegError::RecordNotClosable
    );

    emit!(ConsumedMessageClosed {
        remote_config: ctx.accounts.remote_config.key(),
        consumed_message: record.key(),
        message_class: record.message_class,
        lane_id: record.lane_id,
        sequence: record.sequence,
        message_id: record.message_id,
        rent_destination: ctx.accounts.administrator.key(),
        closed_at: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
