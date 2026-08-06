//! Moves recalled custody to the one fixed outbound escrow.
//!
//! Reaching the escrow is not a receipt on the source chain.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::custody;
use crate::errors::RemoteLegError;
use crate::events::{RecallAssetsSent, RecallCompleted};
use crate::state::{CUSTODY_AUTHORITY_SEED, REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{
    self, REMOTE_POSITION_SEED, RemotePosition, TRANSFER_RECORD_SEED, TransferKind, TransferRecord,
};

#[derive(Accounts)]
pub struct SendRecall<'info> {
    #[account(
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
        has_one = custody_token_account @ RemoteLegError::InvalidCustodyAccount,
        has_one = outbound_escrow @ RemoteLegError::InvalidOutboundEscrow,
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

    /// CHECK: it signs the transfer, and its seeds prove the derivation.
    #[account(
        seeds = [CUSTODY_AUTHORITY_SEED, remote_config.key().as_ref()],
        bump = remote_config.custody_authority_bump,
    )]
    pub custody_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub custody_token_account: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub outbound_escrow: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}

pub fn process_send_recall(ctx: Context<SendRecall>, maximum_amount: u64) -> Result<()> {
    require_eq!(
        ctx.accounts.remote_position.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_keys_eq!(
        ctx.accounts.token_program.key(),
        ctx.accounts.remote_config.token_program,
        RemoteLegError::InvalidTokenProgram
    );
    require_keys_eq!(
        ctx.accounts.outbound_escrow.mint,
        ctx.accounts.remote_config.asset_mint,
        RemoteLegError::InvalidOutboundEscrow
    );

    ctx.accounts
        .transfer_record
        .check_active(TransferKind::Recall, &ctx.accounts.remote_position)?;
    ctx.accounts.transfer_record.check_recall_shape()?;

    let custody_before = ctx.accounts.custody_token_account.amount;
    let position = &mut ctx.accounts.remote_position;
    custody::reconcile(position, custody_before)?;

    let amount = maximum_amount.min(position.recalled_custody);
    require_neq!(amount, 0, RemoteLegError::NoRecalledCustody);

    let escrow_before = ctx.accounts.outbound_escrow.amount;
    let config_key = ctx.accounts.remote_config.key();
    let seeds: &[&[u8]] = &[
        CUSTODY_AUTHORITY_SEED,
        config_key.as_ref(),
        &[ctx.accounts.remote_config.custody_authority_bump],
    ];
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.custody_token_account.to_account_info(),
                to: ctx.accounts.outbound_escrow.to_account_info(),
                authority: ctx.accounts.custody_authority.to_account_info(),
            },
            &[seeds],
        ),
        amount,
    )?;

    ctx.accounts.custody_token_account.reload()?;
    ctx.accounts.outbound_escrow.reload()?;
    let custody_after = ctx.accounts.custody_token_account.amount;
    let escrow_after = ctx.accounts.outbound_escrow.amount;

    let custody_decrease = custody_before
        .checked_sub(custody_after)
        .ok_or(RemoteLegError::InvalidBalanceDelta)?;
    let escrow_increase = escrow_after
        .checked_sub(escrow_before)
        .ok_or(RemoteLegError::InvalidBalanceDelta)?;
    require_eq!(
        custody_decrease,
        amount,
        RemoteLegError::InvalidBalanceDelta
    );
    require_eq!(
        escrow_increase,
        custody_decrease,
        RemoteLegError::InvalidBalanceDelta
    );

    let position = &mut ctx.accounts.remote_position;
    position.recalled_custody = position
        .recalled_custody
        .checked_sub(amount)
        .ok_or(RemoteLegError::NoRecalledCustody)?;

    let record = &mut ctx.accounts.transfer_record;
    record.assets_sent = record
        .assets_sent
        .checked_add(amount)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;

    custody::check_identity(position, custody_after)?;

    let sent_at = Clock::get()?.unix_timestamp;
    emit!(RecallAssetsSent {
        remote_config: config_key,
        transfer_record: record.key(),
        transfer_id: record.transfer_id,
        amount_sent: amount,
        total_sent: record.assets_sent,
        outbound_escrow: ctx.accounts.outbound_escrow.key(),
        sent_at,
    });

    if strategy::settle_recall(record, position, sent_at)? {
        emit!(RecallCompleted {
            remote_config: config_key,
            transfer_record: record.key(),
            transfer_id: record.transfer_id,
            requested_amount: record.requested_recall_amount,
            assets_sent: record.assets_sent,
            realized_loss: record.realized_loss,
            completed_at: sent_at,
        });
    }

    Ok(())
}
