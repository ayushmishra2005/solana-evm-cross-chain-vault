use anchor_lang::prelude::*;
use anchor_lang::solana_program::bpf_loader_upgradeable;

use crate::errors::VerifierError;
use crate::state::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitializeConfigParams {
    pub environment: Environment,
    pub administrator: Pubkey,
    pub consumer: Pubkey,
}

#[derive(Accounts)]
#[instruction(params: InitializeConfigParams)]
pub struct InitializeConfig<'info> {
    /// Only the program's upgrade authority may create a config, so no one
    /// can claim an environment ahead of its operator.
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

    #[account(
        init,
        payer = upgrade_authority,
        space = 8 + VerifierConfig::INIT_SPACE,
        seeds = [CONFIG_SEED, &params.environment.seed()],
        bump,
    )]
    pub config: Account<'info, VerifierConfig>,

    pub system_program: Program<'info, System>,
}

pub fn process_initialize_config(
    ctx: Context<InitializeConfig>,
    params: InitializeConfigParams,
) -> Result<()> {
    require!(
        params.administrator != Pubkey::default()
            && params.consumer != Pubkey::default()
            && params.administrator != params.consumer,
        VerifierError::InvalidRoles
    );
    ctx.accounts.config.set_inner(VerifierConfig {
        administrator: params.administrator,
        consumer: params.consumer,
        environment: params.environment,
        bump: ctx.bumps.config,
    });
    Ok(())
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct RegisterTrustedKeyParams {
    pub kid: String,
    pub public_key: [u8; COMPRESSED_PUBLIC_KEY_LEN],
    pub not_before: i64,
    pub not_after: i64,
}

#[derive(Accounts)]
#[instruction(params: RegisterTrustedKeyParams)]
pub struct RegisterTrustedKey<'info> {
    #[account(mut)]
    pub administrator: Signer<'info>,

    #[account(has_one = administrator @ VerifierError::Unauthorized)]
    pub config: Account<'info, VerifierConfig>,

    /// One account per kid and no close path, so a revoked kid can never be
    /// registered again.
    #[account(
        init,
        payer = administrator,
        space = 8 + TrustedKey::INIT_SPACE,
        seeds = [TRUSTED_KEY_SEED, config.key().as_ref(), params.kid.as_bytes()],
        bump,
    )]
    pub trusted_key: Account<'info, TrustedKey>,

    pub system_program: Program<'info, System>,
}

pub fn process_register_trusted_key(
    ctx: Context<RegisterTrustedKey>,
    params: RegisterTrustedKeyParams,
) -> Result<()> {
    let kid = params.kid.as_bytes();
    require!(
        is_profile_text(kid, MAX_KID_LEN) && ctx.accounts.config.environment.accepts_kid(kid),
        VerifierError::InvalidKeyId
    );
    // The point itself is validated off-chain; no P-256 arithmetic runs here.
    require!(
        matches!(params.public_key.first(), Some(0x02 | 0x03)),
        VerifierError::InvalidPublicKey
    );
    require!(
        params.not_before >= 0 && params.not_before < params.not_after,
        VerifierError::InvalidKeyWindow
    );
    ctx.accounts.trusted_key.set_inner(TrustedKey {
        config: ctx.accounts.config.key(),
        kid: params.kid,
        public_key: params.public_key,
        not_before: params.not_before,
        not_after: params.not_after,
        revoked: false,
        bump: ctx.bumps.trusted_key,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct RevokeTrustedKey<'info> {
    pub administrator: Signer<'info>,

    #[account(has_one = administrator @ VerifierError::Unauthorized)]
    pub config: Account<'info, VerifierConfig>,

    #[account(mut, has_one = config @ VerifierError::Unauthorized)]
    pub trusted_key: Account<'info, TrustedKey>,
}

pub fn process_revoke_trusted_key(ctx: Context<RevokeTrustedKey>) -> Result<()> {
    let trusted_key = &mut ctx.accounts.trusted_key;
    require!(!trusted_key.revoked, VerifierError::AlreadyRevoked);
    trusted_key.revoked = true;
    Ok(())
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct RegisterMandateParams {
    /// Locates the account; must equal `mandate_hash(mandate_id)`.
    pub mandate_hash: [u8; 32],
    pub mandate_id: String,
    pub amount_micros: u64,
    pub currency: String,
    pub recipient: String,
    pub policy_version: String,
    pub agent: String,
    pub category: String,
}

impl RegisterMandateParams {
    fn is_valid(&self) -> bool {
        self.amount_micros > 0
            && is_profile_text(self.mandate_id.as_bytes(), MAX_MANDATE_LEN)
            && is_profile_text(self.currency.as_bytes(), MAX_CURRENCY_LEN)
            && is_profile_text(self.recipient.as_bytes(), MAX_RECIPIENT_LEN)
            && is_profile_text(self.policy_version.as_bytes(), MAX_POLICY_LEN)
            && is_profile_text(self.agent.as_bytes(), MAX_AGENT_LEN)
            && is_profile_text(self.category.as_bytes(), MAX_CATEGORY_LEN)
    }
}

#[derive(Accounts)]
#[instruction(params: RegisterMandateParams)]
pub struct RegisterMandate<'info> {
    #[account(mut)]
    pub administrator: Signer<'info>,

    #[account(has_one = administrator @ VerifierError::Unauthorized)]
    pub config: Account<'info, VerifierConfig>,

    #[account(
        init,
        payer = administrator,
        space = 8 + Mandate::INIT_SPACE,
        seeds = [MANDATE_SEED, config.key().as_ref(), params.mandate_hash.as_ref()],
        bump,
    )]
    pub mandate: Account<'info, Mandate>,

    pub system_program: Program<'info, System>,
}

pub fn process_register_mandate(
    ctx: Context<RegisterMandate>,
    params: RegisterMandateParams,
) -> Result<()> {
    require!(params.is_valid(), VerifierError::InvalidMandate);
    require!(
        params.mandate_hash == mandate_hash(params.mandate_id.as_bytes()),
        VerifierError::MandateHashMismatch
    );
    ctx.accounts.mandate.set_inner(Mandate {
        config: ctx.accounts.config.key(),
        mandate_hash: params.mandate_hash,
        mandate_id: params.mandate_id,
        amount_micros: params.amount_micros,
        currency: params.currency,
        recipient: params.recipient,
        policy_version: params.policy_version,
        agent: params.agent,
        category: params.category,
        revoked: false,
        bump: ctx.bumps.mandate,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct RevokeMandate<'info> {
    pub administrator: Signer<'info>,

    #[account(has_one = administrator @ VerifierError::Unauthorized)]
    pub config: Account<'info, VerifierConfig>,

    #[account(mut, has_one = config @ VerifierError::Unauthorized)]
    pub mandate: Account<'info, Mandate>,
}

pub fn process_revoke_mandate(ctx: Context<RevokeMandate>) -> Result<()> {
    let mandate = &mut ctx.accounts.mandate;
    require!(!mandate.revoked, VerifierError::AlreadyRevoked);
    mandate.revoked = true;
    Ok(())
}
