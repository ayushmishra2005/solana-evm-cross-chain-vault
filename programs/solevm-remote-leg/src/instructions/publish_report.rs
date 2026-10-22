//! Publishing what the leg holds, entirely from its own state.

use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::adapter;
use crate::control::{RISK_CONFIG_SEED, RiskConfig};
use crate::custody;
use crate::errors::RemoteLegError;
use crate::events::RemoteReportPublished;
use crate::instructions::publication::{self, RecordEntry, RecordSlots};
use crate::message;
use crate::outbound::{OUTBOUND_LANE_SEED, OutboundLane, ReportType};
use crate::outbound_message::{self, OutboundContext, ReportFields};
use crate::records;
use crate::remote_state::{self, RemoteStateView};
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{
    REMOTE_POSITION_SEED, RemotePosition, STRATEGY_CONFIG_SEED, StrategyConfig,
    TRANSFER_RECORD_SEED, TransferKind, TransferRecord,
};

#[derive(Accounts)]
pub struct PublishRemoteReport<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
        has_one = custody_token_account @ RemoteLegError::InvalidCustodyAccount,
    )]
    pub remote_config: Box<Account<'info, RemoteConfig>>,

    #[account(
        seeds = [RISK_CONFIG_SEED, remote_config.key().as_ref()],
        bump = risk_config.bump,
    )]
    pub risk_config: Box<Account<'info, RiskConfig>>,

    #[account(
        seeds = [STRATEGY_CONFIG_SEED, remote_config.key().as_ref()],
        bump = strategy_config.bump,
        has_one = adapter_state @ RemoteLegError::InvalidAdapterState,
    )]
    pub strategy_config: Box<Account<'info, StrategyConfig>>,

    #[account(
        mut,
        seeds = [REMOTE_POSITION_SEED, remote_config.key().as_ref()],
        bump = remote_position.bump,
    )]
    pub remote_position: Box<Account<'info, RemotePosition>>,

    #[account(
        mut,
        seeds = [
            OUTBOUND_LANE_SEED,
            remote_config.key().as_ref(),
            &remote_config.report_lane_id.to_le_bytes(),
        ],
        bump = outbound_lane.bump,
    )]
    pub outbound_lane: Box<Account<'info, OutboundLane>>,

    pub custody_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: the address is pinned by the strategy configuration.
    pub adapter_state: UncheckedAccount<'info>,

    /// CHECK: the address is derived and the account is created here.
    #[account(mut)]
    pub report_record: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,

    /// CHECK: bound to the position's open allocation, and required only
    /// while one is open.
    pub active_transfer_record: Option<UncheckedAccount<'info>>,
}

pub fn process_publish_remote_report(
    ctx: Context<PublishRemoteReport>,
    epoch_id: u64,
) -> Result<()> {
    check_versions(&ctx)?;

    let adapter_state = ctx.accounts.adapter_state.to_account_info();
    adapter::check_layout(
        &adapter_state,
        &ctx.accounts.strategy_config.adapter_program,
    )?;

    let custody_amount = ctx.accounts.custody_token_account.amount;
    let adapter_principal = adapter::read_principal(&adapter_state)?;

    let position = &mut ctx.accounts.remote_position;
    custody::reconcile(position, custody_amount)?;
    custody::check_deployed(position, adapter_principal)?;

    let config = &ctx.accounts.remote_config;
    let in_flight = in_flight_attribution(
        position,
        ctx.accounts.active_transfer_record.as_ref(),
        &config.key(),
    )?;
    let remote_principal = remote_principal(position)?.saturating_sub(in_flight);
    let commitment = remote_state::commitment(&RemoteStateView {
        remote_config: &config.key(),
        config_version: config.config_version,
        frozen: config.frozen,
        position,
        custody_balance: custody_amount,
        adapter_principal,
    })?;

    let now = message::current_time()?;
    let built = outbound_message::build_remote_report(
        &OutboundContext {
            config,
            lane: &ctx.accounts.outbound_lane,
            max_report_age: ctx.accounts.risk_config.max_report_age,
            now,
        },
        &ReportFields {
            epoch_id,
            remote_principal,
            realized_loss: position.cumulative_realized_loss,
            unattributed_balance: position.unattributed_custody,
            latest_completed_transfer_id: position.latest_completed_transfer_id,
            config_version: config.config_version,
            remote_state_commitment: commitment,
        },
    )?;

    let published_at = Clock::get()?.unix_timestamp;
    let lane_id = ctx.accounts.outbound_lane.lane_id;
    let sequence = ctx.accounts.outbound_lane.next_sequence;

    publication::create_report_record(
        &RecordSlots {
            report_record: &ctx.accounts.report_record.to_account_info(),
            payer: &ctx.accounts.payer.to_account_info(),
            system_program: &ctx.accounts.system_program.to_account_info(),
        },
        &RecordEntry {
            remote_config: &config.key(),
            lane_id,
            sequence,
            report_type: ReportType::RemoteReport,
            built: &built,
            related_transfer_id: position.latest_completed_transfer_id,
            epoch_id,
            published_at,
        },
    )?;

    ctx.accounts.outbound_lane.advance(
        ReportType::RemoteReport,
        built.next_commitment,
        published_at,
    )?;

    emit!(RemoteReportPublished {
        remote_config: config.key(),
        report_record: ctx.accounts.report_record.key(),
        lane_id,
        sequence,
        message_id: built.message_id,
        body_hash: built.body_hash,
        previous_commitment: built.previous_commitment,
        next_commitment: built.next_commitment,
        encoded_len: built.encoded_len,
        message_bytes: built.encoded.as_bytes().to_vec(),
        published_at,
    });

    Ok(())
}

/// Accepted principal the leg still holds on this chain.
///
/// Unattributed custody and assets already sent are not principal.
fn remote_principal(position: &RemotePosition) -> Result<u64> {
    position
        .attributed_principal
        .checked_add(position.deployed_principal)
        .and_then(|total| total.checked_add(position.recalled_custody))
        .ok_or_else(|| RemoteLegError::ArithmeticOverflow.into())
}

/// Attribution the open allocation already received but has not settled.
///
/// The vault counts an allocation only when it settles, by completion or by
/// close, so a report that counted part of it earlier would count it twice.
fn in_flight_attribution(
    position: &RemotePosition,
    record: Option<&UncheckedAccount>,
    remote_config: &Pubkey,
) -> Result<u64> {
    if position.active_transfer_kind != TransferKind::Allocate {
        return Ok(0);
    }
    let account = record.ok_or(RemoteLegError::MissingActiveTransferRecord)?;
    let info = account.to_account_info();
    let record: TransferRecord = records::load(&info, RemoteLegError::InvalidTransferRecord)?;
    let address = Pubkey::create_program_address(
        &[
            TRANSFER_RECORD_SEED,
            remote_config.as_ref(),
            &position.active_transfer_id,
            &[record.bump],
        ],
        &crate::ID,
    )
    .map_err(|_| RemoteLegError::InvalidTransferRecord)?;
    require_keys_eq!(
        account.key(),
        address,
        RemoteLegError::InvalidTransferRecord
    );
    require!(
        record.transfer_kind == TransferKind::Allocate
            && record.transfer_id == position.active_transfer_id,
        RemoteLegError::InvalidTransferRecord
    );
    Ok(record.attributed_amount)
}

fn check_versions(ctx: &Context<PublishRemoteReport>) -> Result<()> {
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
        ctx.accounts.strategy_config.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_eq!(
        ctx.accounts.remote_position.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_eq!(
        ctx.accounts.outbound_lane.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_eq!(
        ctx.accounts.outbound_lane.lane_id,
        ctx.accounts.remote_config.report_lane_id,
        RemoteLegError::InvalidOutboundLane
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::{TransferKind, TransferStatus};

    fn position(
        attributed: u64,
        deployed: u64,
        recalled: u64,
        unattributed: u64,
    ) -> RemotePosition {
        RemotePosition {
            state_version: STATE_VERSION,
            bump: 254,
            attributed_principal: attributed,
            deployed_principal: deployed,
            recalled_custody: recalled,
            unattributed_custody: unattributed,
            cumulative_realized_loss: 0,
            active_transfer_id: [0u8; 32],
            active_transfer_kind: TransferKind::None,
            active_transfer_sequence: 0,
            active_transfer_status: TransferStatus::None,
            latest_completed_transfer_id: [0u8; 32],
            latest_completion_at: 0,
            initialized_at: 0,
        }
    }

    #[test]
    fn an_empty_position_holds_no_principal() {
        assert_eq!(
            remote_principal(&position(0, 0, 0, 0)).expect("the sum fits"),
            0
        );
    }

    #[test]
    fn every_accepted_bucket_counts_towards_principal() {
        assert_eq!(
            remote_principal(&position(10, 20, 30, 0)).expect("the sum fits"),
            60
        );
    }

    #[test]
    fn unattributed_custody_is_not_principal() {
        assert_eq!(
            remote_principal(&position(10, 0, 0, 500)).expect("the sum fits"),
            10
        );
    }

    #[test]
    fn a_realized_loss_is_never_added_back_to_principal() {
        let mut held = position(10, 0, 0, 0);
        held.cumulative_realized_loss = 400;
        assert_eq!(remote_principal(&held).expect("the sum fits"), 10);
    }

    #[test]
    fn a_sum_above_the_type_maximum_rejects() {
        let held = position(u64::MAX, 1, 0, 0);
        assert_eq!(
            remote_principal(&held),
            Err(RemoteLegError::ArithmeticOverflow.into())
        );
    }
}
