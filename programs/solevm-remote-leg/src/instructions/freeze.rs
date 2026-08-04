//! One way emergency stop.

use anchor_lang::prelude::*;

use crate::errors::RemoteLegError;
use crate::events::RemoteLegFrozen;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig};

#[derive(Accounts)]
pub struct FreezeRemoteLeg<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
    )]
    pub remote_config: Account<'info, RemoteConfig>,
}

pub fn process_freeze(ctx: Context<FreezeRemoteLeg>) -> Result<()> {
    let authority = ctx.accounts.authority.key();
    let config = &mut ctx.accounts.remote_config;

    require!(
        config.is_emergency_authority(&authority),
        RemoteLegError::Unauthorized
    );
    require!(!config.frozen, RemoteLegError::AlreadyFrozen);

    config.frozen = true;
    let frozen_at = Clock::get()?.unix_timestamp;

    emit!(RemoteLegFrozen {
        remote_config: config.key(),
        authority,
        deployment_id: config.deployment_id,
        vault_id: config.vault_id,
        config_version: config.config_version,
        frozen_at,
    });

    Ok(())
}
