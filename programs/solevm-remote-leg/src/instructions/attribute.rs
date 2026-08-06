//! Turns observed custody into accepted principal for the open allocation.
//!
//! The amount always comes from the token account, never from the caller.

use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::custody;
use crate::errors::RemoteLegError;
use crate::events::{AllocationAttributed, AllocationCompleted};
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{
    REMOTE_POSITION_SEED, RemotePosition, TRANSFER_RECORD_SEED, TransferKind, TransferRecord,
    TransferStatus,
};

#[derive(Accounts)]
pub struct AttributeAllocation<'info> {
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
        mut,
        seeds = [REMOTE_POSITION_SEED, remote_config.key().as_ref()],
        bump = remote_position.bump,
    )]
    pub remote_position: Account<'info, RemotePosition>,

    #[account(
        mut,
        seeds = [
            TRANSFER_RECORD_SEED,
            remote_config.key().as_ref(),
            &transfer_record.transfer_id,
        ],
        bump = transfer_record.bump,
    )]
    pub transfer_record: Box<Account<'info, TransferRecord>>,

    pub custody_token_account: Box<Account<'info, TokenAccount>>,
}

pub fn process_attribute_allocation(ctx: Context<AttributeAllocation>) -> Result<()> {
    require_eq!(
        ctx.accounts.remote_position.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require!(!ctx.accounts.remote_config.frozen, RemoteLegError::Frozen);

    ctx.accounts
        .transfer_record
        .check_active(TransferKind::Allocate, &ctx.accounts.remote_position)?;
    ctx.accounts.transfer_record.check_allocation_shape()?;

    let custody_amount = ctx.accounts.custody_token_account.amount;
    let position = &mut ctx.accounts.remote_position;
    custody::reconcile(position, custody_amount)?;

    let record = &mut ctx.accounts.transfer_record;
    let outstanding = record.outstanding_allocation()?;
    let attributed_now = outstanding.min(position.unattributed_custody);
    require_neq!(attributed_now, 0, RemoteLegError::NoAttributableAssets);

    position.unattributed_custody = position
        .unattributed_custody
        .checked_sub(attributed_now)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    position.attributed_principal = position
        .attributed_principal
        .checked_add(attributed_now)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    record.attributed_amount = record
        .attributed_amount
        .checked_add(attributed_now)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    require_gte!(
        record.authorized_amount,
        record.attributed_amount,
        RemoteLegError::AttributionExceedsAuthorization
    );

    custody::check_identity(position, custody_amount)?;

    let attributed_at = Clock::get()?.unix_timestamp;
    let config_key = ctx.accounts.remote_config.key();
    emit!(AllocationAttributed {
        remote_config: config_key,
        transfer_record: record.key(),
        transfer_id: record.transfer_id,
        attributed_now,
        attributed_total: record.attributed_amount,
        authorized_amount: record.authorized_amount,
        unattributed_custody: position.unattributed_custody,
        attributed_at,
    });

    if record.attributed_amount == record.authorized_amount {
        record.status = TransferStatus::Complete;
        record.completed_at = attributed_at;
        position.complete_transfer(record.transfer_id, attributed_at);

        emit!(AllocationCompleted {
            remote_config: config_key,
            transfer_record: record.key(),
            transfer_id: record.transfer_id,
            authorized_amount: record.authorized_amount,
            attributed_amount: record.attributed_amount,
            completed_at: attributed_at,
        });
    }

    Ok(())
}
