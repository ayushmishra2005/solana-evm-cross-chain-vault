//! Pulls recall principal back out of the strategy adapter.
//!
//! An accepted recall may keep unwinding while the leg is frozen.

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::adapter::{self, AdapterCall};
use crate::custody;
use crate::errors::RemoteLegError;
use crate::events::{RecallCompleted, StrategyWithdrawalCompleted};
use crate::state::{CUSTODY_AUTHORITY_SEED, REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{
    self, REMOTE_POSITION_SEED, RemotePosition, STRATEGY_CONFIG_SEED, StrategyConfig,
    TRANSFER_RECORD_SEED, TransferKind, TransferRecord,
};

#[derive(Accounts)]
pub struct WithdrawForRecall<'info> {
    #[account(
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
        has_one = custody_token_account @ RemoteLegError::InvalidCustodyAccount,
        has_one = asset_mint @ RemoteLegError::InvalidMint,
    )]
    pub remote_config: Box<Account<'info, RemoteConfig>>,

    #[account(
        seeds = [STRATEGY_CONFIG_SEED, remote_config.key().as_ref()],
        bump = strategy_config.bump,
        has_one = adapter_program @ RemoteLegError::InvalidAdapterProgram,
        has_one = adapter_state @ RemoteLegError::InvalidAdapterState,
        has_one = adapter_authority @ RemoteLegError::InvalidAdapterAuthority,
        has_one = adapter_token_vault @ RemoteLegError::InvalidAdapterVault,
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
            TRANSFER_RECORD_SEED,
            remote_config.key().as_ref(),
            &transfer_record.transfer_id,
        ],
        bump = transfer_record.bump,
    )]
    pub transfer_record: Box<Account<'info, TransferRecord>>,

    /// CHECK: it signs the call, and its seeds prove the derivation.
    #[account(
        seeds = [CUSTODY_AUTHORITY_SEED, remote_config.key().as_ref()],
        bump = remote_config.custody_authority_bump,
    )]
    pub custody_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub custody_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: the strategy configuration fixes this address.
    pub adapter_program: UncheckedAccount<'info>,

    /// CHECK: the strategy configuration fixes this address.
    #[account(mut)]
    pub adapter_state: UncheckedAccount<'info>,

    /// CHECK: the strategy configuration fixes this address.
    pub adapter_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub adapter_token_vault: Box<Account<'info, TokenAccount>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    pub token_program: Program<'info, Token>,
}

pub fn process_withdraw_for_recall(
    ctx: Context<WithdrawForRecall>,
    maximum_principal: u64,
) -> Result<()> {
    require_eq!(
        ctx.accounts.remote_position.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_eq!(
        ctx.accounts.strategy_config.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_keys_eq!(
        ctx.accounts.token_program.key(),
        ctx.accounts.remote_config.token_program,
        RemoteLegError::InvalidTokenProgram
    );
    require_keys_eq!(
        ctx.accounts.adapter_token_vault.mint,
        ctx.accounts.remote_config.asset_mint,
        RemoteLegError::InvalidAdapterVault
    );

    ctx.accounts
        .transfer_record
        .check_active(TransferKind::Recall, &ctx.accounts.remote_position)?;
    ctx.accounts.transfer_record.check_recall_shape()?;

    let custody_before = ctx.accounts.custody_token_account.amount;
    let position = &mut ctx.accounts.remote_position;
    custody::reconcile(position, custody_before)?;

    let unresolved = ctx.accounts.transfer_record.unresolved_recall_principal()?;
    let request = maximum_principal
        .min(unresolved)
        .min(ctx.accounts.remote_position.deployed_principal);
    require_neq!(request, 0, RemoteLegError::InsufficientStrategyLiquidity);

    let adapter_state_info = ctx.accounts.adapter_state.to_account_info();
    let principal_before = adapter::read_principal(&adapter_state_info)?;
    custody::check_deployed(&ctx.accounts.remote_position, principal_before)?;
    let vault_before = ctx.accounts.adapter_token_vault.amount;

    let config_key = ctx.accounts.remote_config.key();
    let call = AdapterCall {
        adapter_program: ctx.accounts.adapter_program.to_account_info(),
        adapter_state: adapter_state_info.clone(),
        adapter_authority: ctx.accounts.adapter_authority.to_account_info(),
        adapter_token_vault: ctx.accounts.adapter_token_vault.to_account_info(),
        custody_authority: ctx.accounts.custody_authority.to_account_info(),
        custody_token_account: ctx.accounts.custody_token_account.to_account_info(),
        asset_mint: ctx.accounts.asset_mint.to_account_info(),
        token_program: ctx.accounts.token_program.to_account_info(),
    };
    call.withdraw(
        request,
        &[
            CUSTODY_AUTHORITY_SEED,
            config_key.as_ref(),
            &[ctx.accounts.remote_config.custody_authority_bump],
        ],
    )?;

    ctx.accounts.custody_token_account.reload()?;
    ctx.accounts.adapter_token_vault.reload()?;
    let custody_after = ctx.accounts.custody_token_account.amount;
    let vault_after = ctx.accounts.adapter_token_vault.amount;
    let principal_after = adapter::read_principal(&adapter_state_info)?;

    let principal_reduction = principal_before
        .checked_sub(principal_after)
        .ok_or(RemoteLegError::InvalidPrincipalDelta)?;
    let assets_returned = custody_after
        .checked_sub(custody_before)
        .ok_or(RemoteLegError::InvalidBalanceDelta)?;
    let vault_decrease = vault_before
        .checked_sub(vault_after)
        .ok_or(RemoteLegError::InvalidBalanceDelta)?;

    require_eq!(
        vault_decrease,
        assets_returned,
        RemoteLegError::InvalidBalanceDelta
    );
    require_gte!(
        request,
        principal_reduction,
        RemoteLegError::InvalidPrincipalDelta
    );
    require_neq!(
        principal_reduction,
        0,
        RemoteLegError::InsufficientStrategyLiquidity
    );
    let realized_loss = principal_reduction
        .checked_sub(assets_returned)
        .ok_or(RemoteLegError::InvalidRealizedLoss)?;

    let position = &mut ctx.accounts.remote_position;
    position.deployed_principal = position
        .deployed_principal
        .checked_sub(principal_reduction)
        .ok_or(RemoteLegError::InvalidPrincipalDelta)?;
    position.recalled_custody = position
        .recalled_custody
        .checked_add(assets_returned)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    position.cumulative_realized_loss = position
        .cumulative_realized_loss
        .checked_add(realized_loss)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;

    let record = &mut ctx.accounts.transfer_record;
    record.strategy_principal_resolved = record
        .strategy_principal_resolved
        .checked_add(principal_reduction)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    record.assets_withdrawn = record
        .assets_withdrawn
        .checked_add(assets_returned)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    record.realized_loss = record
        .realized_loss
        .checked_add(realized_loss)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    require_gte!(
        record.requested_recall_amount,
        record
            .custody_principal_reserved
            .checked_add(record.strategy_principal_resolved)
            .ok_or(RemoteLegError::ArithmeticOverflow)?,
        RemoteLegError::InvalidRecallAmount
    );

    custody::check_identity(position, custody_after)?;
    custody::check_deployed(position, principal_after)?;

    let withdrawn_at = Clock::get()?.unix_timestamp;
    emit!(StrategyWithdrawalCompleted {
        remote_config: config_key,
        transfer_record: record.key(),
        transfer_id: record.transfer_id,
        principal_reduction,
        assets_returned,
        realized_loss,
        deployed_principal: position.deployed_principal,
        withdrawn_at,
    });

    // A loss can settle the request on its own, leaving nothing to send.
    if strategy::settle_recall(record, position, withdrawn_at)? {
        emit!(RecallCompleted {
            remote_config: config_key,
            transfer_record: record.key(),
            transfer_id: record.transfer_id,
            requested_amount: record.requested_recall_amount,
            assets_sent: record.assets_sent,
            realized_loss: record.realized_loss,
            completed_at: withdrawn_at,
        });
    }

    Ok(())
}
