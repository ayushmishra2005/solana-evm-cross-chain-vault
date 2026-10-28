//! One time adoption of the risk authority that approves deployments.

use anchor_lang::prelude::*;
use es256_authorization_verifier::{
    DEPLOYMENT_AUTHORITY_SEED, DEPLOYMENT_PROFILE_VERSION, DeploymentAuthority,
};

use crate::authorization::{
    DEPLOY_AUTHORIZATION_RESERVED, DEPLOY_AUTHORIZATION_SEED, DeployAuthorizationPolicy,
};
use crate::errors::RemoteLegError;
use crate::events::DeployAuthorizationAdopted;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};

#[derive(Accounts)]
pub struct InitializeDeployAuthorization<'info> {
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
    )]
    pub remote_config: Box<Account<'info, RemoteConfig>>,

    /// The only address the verifier can bind to this leg and namespace, so
    /// no other authority or profile can be adopted.
    #[account(
        seeds = [
            DEPLOYMENT_AUTHORITY_SEED,
            crate::ID.as_ref(),
            remote_config.key().as_ref(),
        ],
        bump = authorization_config.bump,
        seeds::program = es256_authorization_verifier::ID,
    )]
    pub authorization_config: Account<'info, DeploymentAuthority>,

    #[account(
        init,
        payer = administrator,
        space = DeployAuthorizationPolicy::LEN,
        seeds = [DEPLOY_AUTHORIZATION_SEED, remote_config.key().as_ref()],
        bump,
    )]
    pub authorization_policy: Account<'info, DeployAuthorizationPolicy>,

    pub system_program: Program<'info, System>,
}

pub fn process_initialize_deploy_authorization(
    ctx: Context<InitializeDeployAuthorization>,
) -> Result<()> {
    let remote_config = &ctx.accounts.remote_config;
    require_eq!(
        remote_config.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    let authority = &ctx.accounts.authorization_config;
    require!(
        authority.version == DEPLOYMENT_PROFILE_VERSION
            && authority.remote_leg_program == crate::ID
            && authority.remote_config == remote_config.key(),
        RemoteLegError::InvalidAuthorizationPolicy
    );

    let adopted_at = Clock::get()?.unix_timestamp;
    ctx.accounts
        .authorization_policy
        .set_inner(DeployAuthorizationPolicy {
            state_version: STATE_VERSION,
            bump: ctx.bumps.authorization_policy,
            authorization_config: authority.key(),
            adopted_at,
            reserved: [0u8; DEPLOY_AUTHORIZATION_RESERVED],
        });

    emit!(DeployAuthorizationAdopted {
        remote_config: remote_config.key(),
        authorization_policy: ctx.accounts.authorization_policy.key(),
        authorization_config: authority.key(),
        network_domain: authority.network_domain,
        adopted_at,
    });

    Ok(())
}
