use anchor_lang::prelude::*;

use crate::deployment::{DeploymentAuthorization, MAX_LIFETIME_SECONDS, check_deployment_follows};
use crate::errors::VerifierError;
use crate::precompile::{load_verified_precompile, parse_precompile_data};
use crate::state::*;

#[derive(Accounts)]
#[instruction(nonce: [u8; DEPLOYMENT_NONCE_LEN])]
pub struct ConsumeDeploymentAuthorization<'info> {
    /// Anyone may relay an approval: it authorizes one exact deployment,
    /// which the following instruction must be.
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        seeds = [
            DEPLOYMENT_AUTHORITY_SEED,
            config.remote_leg_program.as_ref(),
            config.remote_config.as_ref(),
        ],
        bump = config.bump,
    )]
    pub config: Account<'info, DeploymentAuthority>,

    #[account(has_one = config @ VerifierError::Unauthorized)]
    pub risk_key: Account<'info, RiskKey>,

    /// `nonce` only locates the receipt; the handler requires it to be the
    /// signed nonce. `init` fails if the nonce was consumed.
    #[account(
        init,
        payer = payer,
        space = 8 + DeploymentReceipt::INIT_SPACE,
        seeds = [DEPLOYMENT_RECEIPT_SEED, config.key().as_ref(), nonce.as_ref()],
        bump,
    )]
    pub receipt: Account<'info, DeploymentReceipt>,

    /// CHECK: The address is pinned and the checked loaders re-verify it.
    #[account(address = solana_instructions_sysvar::ID)]
    pub instructions_sysvar: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn process_consume_deployment_authorization(
    ctx: Context<ConsumeDeploymentAuthorization>,
    nonce: [u8; DEPLOYMENT_NONCE_LEN],
) -> Result<()> {
    let precompile = load_verified_precompile(&ctx.accounts.instructions_sysvar)?;
    let verified = parse_precompile_data(&precompile.data)?;
    let message = DeploymentAuthorization::decode(verified.signing_input)?;

    let config = &ctx.accounts.config;
    let risk_key = &ctx.accounts.risk_key;
    require!(!risk_key.revoked, VerifierError::KeyRevoked);
    require!(
        verified.public_key == risk_key.public_key.as_slice(),
        VerifierError::PublicKeyMismatch
    );
    require_keys_eq!(
        message.authority,
        config.key(),
        VerifierError::AuthorityMismatch
    );
    require!(
        message.network_domain == config.network_domain,
        VerifierError::NetworkDomainMismatch
    );
    require_eq!(
        message.policy_generation,
        config.policy_generation,
        VerifierError::PolicyGenerationMismatch
    );
    require!(
        message.amount > 0,
        VerifierError::InvalidAuthorizationAmount
    );
    require!(message.nonce == nonce, VerifierError::NonceMismatch);

    let now = Clock::get()?.unix_timestamp;
    check_times(&message, now)?;
    // `issued_at` may lead the clock, so the clock itself must reach
    // `not_before`. `now < expires_at <= not_after` already bounds the end.
    require!(
        risk_key.not_before <= now
            && risk_key.not_before <= message.issued_at
            && message.expires_at <= risk_key.not_after,
        VerifierError::OutsideKeyWindow
    );

    let receipt_key = ctx.accounts.receipt.key();
    check_deployment_follows(
        &ctx.accounts.instructions_sysvar,
        config,
        &receipt_key,
        message.amount,
    )?;

    let receipt_bump = ctx.bumps.receipt;
    let config_key = config.key();
    let remote_config = config.remote_config;
    let risk_key_address = risk_key.key();
    ctx.accounts.receipt.set_inner(DeploymentReceipt {
        version: DEPLOYMENT_PROFILE_VERSION,
        bump: receipt_bump,
        config: config_key,
        remote_config,
        risk_key: risk_key_address,
        nonce,
        deployment_context: message.deployment_context,
        amount: message.amount,
        policy_generation: message.policy_generation,
        issued_at: message.issued_at,
        expires_at: message.expires_at,
        consumed_at: now,
        message_hash: solana_sha256_hasher::hash(verified.signing_input).to_bytes(),
    });
    Ok(())
}

fn check_times(message: &DeploymentAuthorization, now: i64) -> Result<()> {
    let latest_expiry = message
        .issued_at
        .checked_add(MAX_LIFETIME_SECONDS)
        .ok_or(VerifierError::InvalidAuthorizationLifetime)?;
    require!(
        message.issued_at < message.expires_at && message.expires_at <= latest_expiry,
        VerifierError::InvalidAuthorizationLifetime
    );
    let latest_issue = now
        .checked_add(MAX_ISSUED_AT_SKEW_SECONDS)
        .ok_or(VerifierError::IssuedInFuture)?;
    require!(
        message.issued_at <= latest_issue,
        VerifierError::IssuedInFuture
    );
    require!(now < message.expires_at, VerifierError::Expired);
    Ok(())
}
