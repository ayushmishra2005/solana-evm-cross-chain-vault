use anchor_lang::prelude::*;

use crate::errors::VerifierError;
use crate::jwt::{Claims, decode_signing_input};
use crate::precompile::{load_verified_precompile, parse_precompile_data};
use crate::state::*;

#[derive(Accounts)]
#[instruction(nonce_hash: [u8; 32])]
pub struct ConsumeAuthorization<'info> {
    #[account(mut)]
    pub consumer: Signer<'info>,

    #[account(
        seeds = [CONFIG_SEED, &config.environment.seed()],
        bump = config.bump,
        has_one = consumer @ VerifierError::Unauthorized,
    )]
    pub config: Account<'info, VerifierConfig>,

    #[account(has_one = config @ VerifierError::Unauthorized)]
    pub trusted_key: Account<'info, TrustedKey>,

    #[account(has_one = config @ VerifierError::Unauthorized)]
    pub mandate: Account<'info, Mandate>,

    /// `nonce_hash` only locates the receipt; the handler requires it to be
    /// the hash of the signed nonce. `init` fails if the nonce was consumed.
    #[account(
        init,
        payer = consumer,
        space = 8 + ConsumedAuthorization::INIT_SPACE,
        seeds = [RECEIPT_SEED, config.key().as_ref(), nonce_hash.as_ref()],
        bump,
    )]
    pub receipt: Account<'info, ConsumedAuthorization>,

    /// CHECK: The address is pinned and the checked loaders re-verify it.
    #[account(address = solana_instructions_sysvar::ID)]
    pub instructions_sysvar: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn process_consume_authorization(
    ctx: Context<ConsumeAuthorization>,
    nonce_hash_argument: [u8; 32],
) -> Result<()> {
    let precompile = load_verified_precompile(&ctx.accounts.instructions_sysvar)?;
    let verified = parse_precompile_data(&precompile.data)?;

    let decoded = decode_signing_input(verified.signing_input)?;
    let header = decoded.header()?;
    let claims = decoded.claims()?;

    let config = &ctx.accounts.config;
    let trusted_key = &ctx.accounts.trusted_key;
    require!(
        header.kid == trusted_key.kid.as_bytes(),
        VerifierError::KeyIdMismatch
    );
    require!(
        config.environment.accepts_kid(header.kid),
        VerifierError::InvalidKeyId
    );
    require!(!trusted_key.revoked, VerifierError::KeyRevoked);
    require!(
        verified.public_key == trusted_key.public_key.as_slice(),
        VerifierError::PublicKeyMismatch
    );

    let now = Clock::get()?.unix_timestamp;
    check_claims(&claims, now)?;
    require!(
        trusted_key.not_before <= claims.iat && claims.exp <= trusted_key.not_after,
        VerifierError::OutsideKeyWindow
    );
    check_mandate(&ctx.accounts.mandate, &claims)?;
    require!(
        nonce_hash(claims.nonce) == nonce_hash_argument,
        VerifierError::NonceHashMismatch
    );

    let receipt_bump = ctx.bumps.receipt;
    let config_key = config.key();
    let mandate_key = ctx.accounts.mandate.key();
    ctx.accounts.receipt.set_inner(ConsumedAuthorization {
        config: config_key,
        nonce_hash: nonce_hash_argument,
        signing_input_hash: solana_sha256_hasher::hash(verified.signing_input).to_bytes(),
        jti_hash: domain_hash(JTI_DOMAIN, claims.jti),
        mandate: mandate_key,
        consumed_at: now,
        expires_at: claims.exp,
        bump: receipt_bump,
    });
    Ok(())
}

fn check_claims(claims: &Claims, now: i64) -> Result<()> {
    require!(claims.iss == ISSUER, VerifierError::IssuerMismatch);
    require!(
        claims.verdict == ALLOW_VERDICT,
        VerifierError::VerdictNotAllow
    );
    require!(
        claims.iat.checked_add(VERDICT_LIFETIME_SECONDS) == Some(claims.exp),
        VerifierError::InvalidLifetime
    );
    let latest_issue = now
        .checked_add(MAX_ISSUED_AT_SKEW_SECONDS)
        .ok_or(VerifierError::IssuedInFuture)?;
    require!(claims.iat <= latest_issue, VerifierError::IssuedInFuture);
    require!(now < claims.exp, VerifierError::Expired);
    Ok(())
}

fn check_mandate(mandate: &Mandate, claims: &Claims) -> Result<()> {
    let request = &claims.request;
    require!(!mandate.revoked, VerifierError::MandateRevoked);
    require!(
        claims.mandate == mandate.mandate_id.as_bytes(),
        VerifierError::MandateMismatch
    );
    require!(
        request.amount_micros == mandate.amount_micros,
        VerifierError::AmountMismatch
    );
    require!(
        request.currency == mandate.currency.as_bytes(),
        VerifierError::CurrencyMismatch
    );
    require!(
        request.recipient == mandate.recipient.as_bytes(),
        VerifierError::RecipientMismatch
    );
    require!(
        claims.policy_version == mandate.policy_version.as_bytes(),
        VerifierError::PolicyMismatch
    );
    require!(
        request.agent == mandate.agent.as_bytes(),
        VerifierError::AgentMismatch
    );
    require!(
        request.category == mandate.category.as_bytes(),
        VerifierError::CategoryMismatch
    );
    Ok(())
}
