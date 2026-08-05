//! Applies one canonical configuration message to the risk limits.

use std::io::Write;

use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Allocate, Assign, Transfer};

use crate::control::{
    CONSUMED_MESSAGE_SEED, ConsumedMessage, MessageClass, REPLAY_LANE_SEED, RISK_CONFIG_SEED,
    ReplayLane, RiskConfig,
};
use crate::errors::RemoteLegError;
use crate::events::ConfigUpdated;
use crate::message::{self, ValidatedMessage};
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

    let record = RecordAddress::derive(&config_key, lane_id, sequence);
    require_keys_eq!(
        ctx.accounts.consumed_message.key(),
        record.address,
        RemoteLegError::InvalidConsumedMessage
    );
    check_record_is_free(&ctx.accounts.consumed_message)?;

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

    create_record(
        &ctx.accounts.consumed_message,
        &ctx.accounts.transport_verifier,
        &ctx.accounts.system_program,
        &config_key,
        &record,
        ConsumedMessage {
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

/// The canonical record address for one sequence, with its seed bytes.
struct RecordAddress {
    address: Pubkey,
    bump: u8,
    class: [u8; 1],
    lane: [u8; 4],
    sequence: [u8; 8],
}

impl RecordAddress {
    fn derive(config_key: &Pubkey, lane_id: u32, sequence: u64) -> Self {
        let class = [CLASS.to_u8()];
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
}

/// Rejects anything that is not an empty system owned account.
///
/// Lamports alone are allowed, so a stranger cannot block a valid message.
fn check_record_is_free(record: &UncheckedAccount) -> Result<()> {
    let info = record.to_account_info();
    if info.owner == &crate::ID {
        return Err(RemoteLegError::ReplayDetected.into());
    }
    require_keys_eq!(
        *info.owner,
        system_program::ID,
        RemoteLegError::InvalidConsumedMessage
    );
    require!(info.data_is_empty(), RemoteLegError::InvalidConsumedMessage);
    Ok(())
}

/// Funds, allocates and assigns the record, then writes it.
fn create_record<'info>(
    record: &UncheckedAccount<'info>,
    payer: &Signer<'info>,
    system_program_account: &Program<'info, System>,
    config_key: &Pubkey,
    address: &RecordAddress,
    value: ConsumedMessage,
) -> Result<()> {
    let signer_seeds: &[&[u8]] = &[
        CONSUMED_MESSAGE_SEED,
        config_key.as_ref(),
        &address.class,
        &address.lane,
        &address.sequence,
        &[address.bump],
    ];
    let signer = &[signer_seeds];

    let space = ConsumedMessage::LEN;
    let required = Rent::get()?.minimum_balance(space);
    let current = record.lamports();
    if current < required {
        let missing = required
            .checked_sub(current)
            .ok_or(RemoteLegError::ArithmeticOverflow)?;
        system_program::transfer(
            CpiContext::new(
                system_program_account.key(),
                Transfer {
                    from: payer.to_account_info(),
                    to: record.to_account_info(),
                },
            ),
            missing,
        )?;
    }

    let width = u64::try_from(space).map_err(|_| RemoteLegError::ArithmeticOverflow)?;
    system_program::allocate(
        CpiContext::new_with_signer(
            system_program_account.key(),
            Allocate {
                account_to_allocate: record.to_account_info(),
            },
            signer,
        ),
        width,
    )?;
    system_program::assign(
        CpiContext::new_with_signer(
            system_program_account.key(),
            Assign {
                account_to_assign: record.to_account_info(),
            },
            signer,
        ),
        &crate::ID,
    )?;

    let info = record.to_account_info();
    let mut data = info.try_borrow_mut_data()?;
    let mut slot: &mut [u8] = &mut data;
    slot.write_all(ConsumedMessage::DISCRIMINATOR)?;
    value.serialize(&mut slot)?;
    Ok(())
}
