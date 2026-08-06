//! Sends attributed custody to the one configured strategy adapter.
//!
//! Every result is read back from the accounts. The adapter reports nothing.

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::adapter::{self, AdapterCall};
use crate::custody;
use crate::errors::RemoteLegError;
use crate::events::AssetsDeployed;
use crate::state::{CUSTODY_AUTHORITY_SEED, REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{REMOTE_POSITION_SEED, RemotePosition, STRATEGY_CONFIG_SEED, StrategyConfig};

#[derive(Accounts)]
pub struct DeployToStrategy<'info> {
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

    /// CHECK: it signs the transfer, and its seeds prove the derivation.
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

pub fn process_deploy_to_strategy(
    ctx: Context<DeployToStrategy>,
    maximum_amount: u64,
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
    require!(!ctx.accounts.remote_config.frozen, RemoteLegError::Frozen);
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

    let custody_before = ctx.accounts.custody_token_account.amount;
    let position = &mut ctx.accounts.remote_position;
    custody::reconcile(position, custody_before)?;

    let amount = maximum_amount.min(position.attributed_principal);
    require_neq!(amount, 0, RemoteLegError::InsufficientAttributedCustody);

    let adapter_state_info = ctx.accounts.adapter_state.to_account_info();
    let principal_before = adapter::read_principal(&adapter_state_info)?;
    custody::check_deployed(position, principal_before)?;
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
    call.deposit_exact(
        amount,
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

    let custody_decrease = custody_before
        .checked_sub(custody_after)
        .ok_or(RemoteLegError::InvalidBalanceDelta)?;
    let vault_increase = vault_after
        .checked_sub(vault_before)
        .ok_or(RemoteLegError::InvalidBalanceDelta)?;
    let principal_increase = principal_after
        .checked_sub(principal_before)
        .ok_or(RemoteLegError::InvalidPrincipalDelta)?;

    require_eq!(
        custody_decrease,
        amount,
        RemoteLegError::InvalidBalanceDelta
    );
    require_eq!(
        vault_increase,
        custody_decrease,
        RemoteLegError::InvalidBalanceDelta
    );
    require_eq!(
        principal_increase,
        custody_decrease,
        RemoteLegError::InvalidPrincipalDelta
    );

    let position = &mut ctx.accounts.remote_position;
    position.attributed_principal = position
        .attributed_principal
        .checked_sub(amount)
        .ok_or(RemoteLegError::InsufficientAttributedCustody)?;
    position.deployed_principal = position
        .deployed_principal
        .checked_add(amount)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;

    custody::check_identity(position, custody_after)?;
    custody::check_deployed(position, principal_after)?;

    emit!(AssetsDeployed {
        remote_config: config_key,
        adapter_state: adapter_state_info.key(),
        deployed_now: amount,
        attributed_principal: position.attributed_principal,
        deployed_principal: position.deployed_principal,
        deployed_at: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
