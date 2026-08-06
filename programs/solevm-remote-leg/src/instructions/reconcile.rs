//! Classifies custody the leg has not explained yet.

use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::custody;
use crate::errors::RemoteLegError;
use crate::events::CustodyReconciled;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{REMOTE_POSITION_SEED, RemotePosition};

#[derive(Accounts)]
pub struct ReconcileCustody<'info> {
    #[account(
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
        has_one = custody_token_account @ RemoteLegError::InvalidCustodyAccount,
    )]
    pub remote_config: Account<'info, RemoteConfig>,

    #[account(
        mut,
        seeds = [REMOTE_POSITION_SEED, remote_config.key().as_ref()],
        bump = remote_position.bump,
    )]
    pub remote_position: Account<'info, RemotePosition>,

    pub custody_token_account: Account<'info, TokenAccount>,
}

pub fn process_reconcile_custody(ctx: Context<ReconcileCustody>) -> Result<()> {
    require_eq!(
        ctx.accounts.remote_position.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );

    let custody_amount = ctx.accounts.custody_token_account.amount;
    let position = &mut ctx.accounts.remote_position;
    let observed_surplus = custody::reconcile(position, custody_amount)?;

    emit!(CustodyReconciled {
        remote_config: ctx.accounts.remote_config.key(),
        remote_position: position.key(),
        observed_surplus,
        unattributed_custody: position.unattributed_custody,
        reconciled_at: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
