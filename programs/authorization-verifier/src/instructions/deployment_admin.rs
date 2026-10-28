use anchor_lang::prelude::*;
use anchor_lang::solana_program::bpf_loader_upgradeable;

use crate::errors::VerifierError;
use crate::state::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitializeDeploymentAuthorityParams {
    pub administrator: Pubkey,
    pub network_domain: [u8; 32],
}

#[derive(Accounts)]
pub struct InitializeDeploymentAuthority<'info> {
    /// Only the upgrade authority may bind a namespace to a risk authority,
    /// so no one can claim a remote leg's namespace ahead of its operator.
    #[account(mut)]
    pub upgrade_authority: Signer<'info>,

    #[account(
        seeds = [crate::ID.as_ref()],
        bump,
        seeds::program = bpf_loader_upgradeable::ID,
        constraint = program_data.upgrade_authority_address == Some(upgrade_authority.key())
            @ VerifierError::Unauthorized,
    )]
    pub program_data: Account<'info, ProgramData>,

    /// CHECK: Only its address and executable flag are used.
    #[account(
        constraint = remote_leg_program.executable && remote_leg_program.key() != crate::ID
            @ VerifierError::InvalidDeploymentAuthority,
    )]
    pub remote_leg_program: UncheckedAccount<'info>,

    /// CHECK: Only its address and owner are used; the remote leg checks
    /// the binding against its typed config when it adopts this authority.
    #[account(
        owner = remote_leg_program.key() @ VerifierError::InvalidDeploymentAuthority,
        constraint = !remote_config.data_is_empty() @ VerifierError::InvalidDeploymentAuthority,
    )]
    pub remote_config: UncheckedAccount<'info>,

    #[account(
        init,
        payer = upgrade_authority,
        space = 8 + DeploymentAuthority::INIT_SPACE,
        seeds = [
            DEPLOYMENT_AUTHORITY_SEED,
            remote_leg_program.key().as_ref(),
            remote_config.key().as_ref(),
        ],
        bump,
    )]
    pub config: Account<'info, DeploymentAuthority>,

    pub system_program: Program<'info, System>,
}

pub fn process_initialize_deployment_authority(
    ctx: Context<InitializeDeploymentAuthority>,
    params: InitializeDeploymentAuthorityParams,
) -> Result<()> {
    require!(
        params.administrator != Pubkey::default() && params.network_domain != [0u8; 32],
        VerifierError::InvalidDeploymentAuthority
    );
    ctx.accounts.config.set_inner(DeploymentAuthority {
        version: DEPLOYMENT_PROFILE_VERSION,
        bump: ctx.bumps.config,
        administrator: params.administrator,
        remote_leg_program: ctx.accounts.remote_leg_program.key(),
        remote_config: ctx.accounts.remote_config.key(),
        network_domain: params.network_domain,
        policy_generation: 0,
    });
    Ok(())
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterRiskKeyParams {
    pub public_key: [u8; COMPRESSED_PUBLIC_KEY_LEN],
    pub not_before: i64,
    pub not_after: i64,
}

#[derive(Accounts)]
#[instruction(params: RegisterRiskKeyParams)]
pub struct RegisterRiskKey<'info> {
    #[account(mut)]
    pub administrator: Signer<'info>,

    #[account(has_one = administrator @ VerifierError::Unauthorized)]
    pub config: Account<'info, DeploymentAuthority>,

    /// One account per key and no close path, so a revoked key can never be
    /// registered again.
    #[account(
        init,
        payer = administrator,
        space = 8 + RiskKey::INIT_SPACE,
        seeds = [
            RISK_KEY_SEED,
            config.key().as_ref(),
            public_key_parity_seed(&params.public_key),
            public_key_x_seed(&params.public_key),
        ],
        bump,
    )]
    pub risk_key: Account<'info, RiskKey>,

    pub system_program: Program<'info, System>,
}

pub fn process_register_risk_key(
    ctx: Context<RegisterRiskKey>,
    params: RegisterRiskKeyParams,
) -> Result<()> {
    // The point itself is validated off-chain; no P-256 arithmetic runs here.
    require!(
        matches!(params.public_key.first(), Some(0x02 | 0x03)),
        VerifierError::InvalidPublicKey
    );
    require!(
        params.not_before >= 0 && params.not_before < params.not_after,
        VerifierError::InvalidKeyWindow
    );
    ctx.accounts.risk_key.set_inner(RiskKey {
        config: ctx.accounts.config.key(),
        public_key: params.public_key,
        not_before: params.not_before,
        not_after: params.not_after,
        revoked: false,
        bump: ctx.bumps.risk_key,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct RevokeRiskKey<'info> {
    pub administrator: Signer<'info>,

    #[account(has_one = administrator @ VerifierError::Unauthorized)]
    pub config: Account<'info, DeploymentAuthority>,

    #[account(mut, has_one = config @ VerifierError::Unauthorized)]
    pub risk_key: Account<'info, RiskKey>,
}

pub fn process_revoke_risk_key(ctx: Context<RevokeRiskKey>) -> Result<()> {
    let risk_key = &mut ctx.accounts.risk_key;
    require!(!risk_key.revoked, VerifierError::AlreadyRevoked);
    risk_key.revoked = true;
    Ok(())
}

#[derive(Accounts)]
pub struct AdvancePolicyGeneration<'info> {
    pub administrator: Signer<'info>,

    #[account(mut, has_one = administrator @ VerifierError::Unauthorized)]
    pub config: Account<'info, DeploymentAuthority>,
}

pub fn process_advance_policy_generation(ctx: Context<AdvancePolicyGeneration>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.policy_generation = config
        .policy_generation
        .checked_add(1)
        .ok_or(VerifierError::PolicyGenerationExhausted)?;
    Ok(())
}
