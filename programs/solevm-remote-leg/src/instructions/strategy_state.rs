//! One time setup of the adapter identity and the position.

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::control::{RISK_CONFIG_SEED, RiskConfig};
use crate::errors::RemoteLegError;
use crate::events::StrategyStateInitialized;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{
    REMOTE_POSITION_SEED, RemotePosition, STRATEGY_CONFIG_RESERVED, STRATEGY_CONFIG_SEED,
    StrategyConfig, TransferKind, TransferStatus,
};

#[derive(Accounts)]
pub struct InitializeStrategyState<'info> {
    #[account(mut)]
    pub administrator: Signer<'info>,

    #[account(
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
        has_one = administrator @ RemoteLegError::Unauthorized,
        has_one = asset_mint @ RemoteLegError::InvalidMint,
    )]
    pub remote_config: Box<Account<'info, RemoteConfig>>,

    #[account(
        seeds = [RISK_CONFIG_SEED, remote_config.key().as_ref()],
        bump = risk_config.bump,
    )]
    pub risk_config: Account<'info, RiskConfig>,

    #[account(
        init,
        payer = administrator,
        space = StrategyConfig::LEN,
        seeds = [STRATEGY_CONFIG_SEED, remote_config.key().as_ref()],
        bump,
    )]
    pub strategy_config: Account<'info, StrategyConfig>,

    #[account(
        init,
        payer = administrator,
        space = RemotePosition::LEN,
        seeds = [REMOTE_POSITION_SEED, remote_config.key().as_ref()],
        bump,
    )]
    pub remote_position: Account<'info, RemotePosition>,

    /// CHECK: only the stored address may ever receive a later call.
    pub adapter_program: UncheckedAccount<'info>,

    /// CHECK: ownership and address rules are checked in the handler.
    pub adapter_state: UncheckedAccount<'info>,

    /// CHECK: it must own the adapter vault, which proves the derivation.
    pub adapter_authority: UncheckedAccount<'info>,

    pub adapter_token_vault: Box<Account<'info, TokenAccount>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    pub token_program: Program<'info, Token>,

    pub system_program: Program<'info, System>,
}

pub fn process_initialize_strategy_state(
    ctx: Context<InitializeStrategyState>,
    max_remote_principal: u64,
) -> Result<()> {
    let config = &ctx.accounts.remote_config;
    require_eq!(
        config.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require_eq!(
        ctx.accounts.risk_config.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require!(!config.frozen, RemoteLegError::Frozen);
    require_neq!(
        max_remote_principal,
        0,
        RemoteLegError::InvalidStrategyConfig
    );
    require_keys_eq!(
        ctx.accounts.token_program.key(),
        config.token_program,
        RemoteLegError::InvalidTokenProgram
    );

    let adapter_program = ctx.accounts.adapter_program.to_account_info();
    require!(
        adapter_program.executable,
        RemoteLegError::InvalidAdapterProgram
    );
    require_keys_neq!(
        adapter_program.key(),
        crate::ID,
        RemoteLegError::InvalidAdapterProgram
    );

    let adapter_state = ctx.accounts.adapter_state.to_account_info();
    require_keys_eq!(
        *adapter_state.owner,
        adapter_program.key(),
        RemoteLegError::InvalidAdapterState
    );
    let expected_state = Pubkey::find_program_address(
        &[ADAPTER_STATE_SEED, config.key().as_ref()],
        &adapter_program.key(),
    )
    .0;
    require_keys_eq!(
        adapter_state.key(),
        expected_state,
        RemoteLegError::InvalidAdapterState
    );

    let expected_authority = Pubkey::find_program_address(
        &[ADAPTER_AUTHORITY_SEED, adapter_state.key().as_ref()],
        &adapter_program.key(),
    )
    .0;
    require_keys_eq!(
        ctx.accounts.adapter_authority.key(),
        expected_authority,
        RemoteLegError::InvalidAdapterAuthority
    );

    check_adapter_vault(&ctx, &expected_authority)?;

    let initialized_at = Clock::get()?.unix_timestamp;
    let adapter_token_vault = ctx.accounts.adapter_token_vault.key();

    ctx.accounts.strategy_config.set_inner(StrategyConfig {
        state_version: STATE_VERSION,
        bump: ctx.bumps.strategy_config,
        adapter_program: adapter_program.key(),
        adapter_state: adapter_state.key(),
        adapter_authority: expected_authority,
        adapter_token_vault,
        max_remote_principal,
        initialized_at,
        reserved: [0u8; STRATEGY_CONFIG_RESERVED],
    });

    ctx.accounts.remote_position.set_inner(RemotePosition {
        state_version: STATE_VERSION,
        bump: ctx.bumps.remote_position,
        attributed_principal: 0,
        deployed_principal: 0,
        recalled_custody: 0,
        unattributed_custody: 0,
        cumulative_realized_loss: 0,
        active_transfer_id: [0u8; 32],
        active_transfer_kind: TransferKind::None,
        active_transfer_sequence: 0,
        active_transfer_status: TransferStatus::None,
        latest_completed_transfer_id: [0u8; 32],
        latest_completion_at: 0,
        initialized_at,
    });

    emit!(StrategyStateInitialized {
        remote_config: config.key(),
        strategy_config: ctx.accounts.strategy_config.key(),
        remote_position: ctx.accounts.remote_position.key(),
        adapter_program: adapter_program.key(),
        adapter_state: adapter_state.key(),
        adapter_authority: expected_authority,
        adapter_token_vault,
        max_remote_principal,
        initialized_at,
    });

    Ok(())
}

/// Seed prefix the adapter uses for its state account.
const ADAPTER_STATE_SEED: &[u8] = b"adapter-state";

/// Seed prefix the adapter uses for its vault authority.
const ADAPTER_AUTHORITY_SEED: &[u8] = b"adapter-authority";

fn check_adapter_vault(
    ctx: &Context<InitializeStrategyState>,
    adapter_authority: &Pubkey,
) -> Result<()> {
    let config = &ctx.accounts.remote_config;
    let vault = &ctx.accounts.adapter_token_vault;
    require_keys_eq!(
        *vault.to_account_info().owner,
        config.token_program,
        RemoteLegError::InvalidTokenProgram
    );
    require_keys_eq!(
        vault.mint,
        config.asset_mint,
        RemoteLegError::InvalidAdapterVault
    );
    require_keys_eq!(
        vault.owner,
        *adapter_authority,
        RemoteLegError::InvalidAdapterVault
    );
    require!(
        vault.delegate.is_none(),
        RemoteLegError::InvalidAdapterVault
    );
    require!(
        vault.close_authority.is_none(),
        RemoteLegError::InvalidAdapterVault
    );
    require_keys_neq!(
        vault.key(),
        config.custody_token_account,
        RemoteLegError::InvalidAdapterVault
    );
    require_keys_neq!(
        vault.key(),
        config.outbound_escrow,
        RemoteLegError::InvalidAdapterVault
    );
    Ok(())
}
