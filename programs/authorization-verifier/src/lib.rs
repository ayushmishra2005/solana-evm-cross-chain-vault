//! Standalone verifier that consumes one ES256-signed authorization verdict.
//!
//! The native secp256r1 program verifies the signature over the exact JWS
//! signing input. This program reads that input back from the preceding
//! instruction, parses it under a strict profile, checks it against
//! administrator-approved state and records its nonce exactly once. It moves
//! no tokens.
//!
//! A second, separate profile authorizes SolEVM strategy deployments; see
//! [`deployment`]. It shares only the precompile checks with the first.

use anchor_lang::prelude::*;

pub mod deployment;
pub mod errors;
pub mod instructions;
pub mod jwt;
pub mod precompile;
pub mod state;

pub use errors::VerifierError;
pub use instructions::*;
pub use state::*;

// The devnet id is a cfg, not a feature, so `--all-features` builds keep the
// default id. The remote leg's devnet build embeds this id; see
// scripts/devnet-v2.
#[cfg(not(solevm_devnet))]
declare_id!("da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7");
#[cfg(solevm_devnet)]
declare_id!("4dAHz4oRNVmQ1ru9WkUthKBKU65Bjn3jQrQ5GDfQ9CSw");

#[program]
pub mod es256_authorization_verifier {
    use super::*;

    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        params: InitializeConfigParams,
    ) -> Result<()> {
        instructions::admin::process_initialize_config(ctx, params)
    }

    pub fn register_trusted_key(
        ctx: Context<RegisterTrustedKey>,
        params: RegisterTrustedKeyParams,
    ) -> Result<()> {
        instructions::admin::process_register_trusted_key(ctx, params)
    }

    pub fn revoke_trusted_key(ctx: Context<RevokeTrustedKey>) -> Result<()> {
        instructions::admin::process_revoke_trusted_key(ctx)
    }

    pub fn register_mandate(
        ctx: Context<RegisterMandate>,
        params: RegisterMandateParams,
    ) -> Result<()> {
        instructions::admin::process_register_mandate(ctx, params)
    }

    pub fn revoke_mandate(ctx: Context<RevokeMandate>) -> Result<()> {
        instructions::admin::process_revoke_mandate(ctx)
    }

    pub fn consume_authorization(
        ctx: Context<ConsumeAuthorization>,
        nonce_hash: [u8; 32],
    ) -> Result<()> {
        instructions::consume::process_consume_authorization(ctx, nonce_hash)
    }

    pub fn initialize_deployment_authority(
        ctx: Context<InitializeDeploymentAuthority>,
        params: InitializeDeploymentAuthorityParams,
    ) -> Result<()> {
        instructions::deployment_admin::process_initialize_deployment_authority(ctx, params)
    }

    pub fn register_risk_key(
        ctx: Context<RegisterRiskKey>,
        params: RegisterRiskKeyParams,
    ) -> Result<()> {
        instructions::deployment_admin::process_register_risk_key(ctx, params)
    }

    pub fn revoke_risk_key(ctx: Context<RevokeRiskKey>) -> Result<()> {
        instructions::deployment_admin::process_revoke_risk_key(ctx)
    }

    pub fn advance_policy_generation(ctx: Context<AdvancePolicyGeneration>) -> Result<()> {
        instructions::deployment_admin::process_advance_policy_generation(ctx)
    }

    pub fn consume_deployment_authorization(
        ctx: Context<ConsumeDeploymentAuthorization>,
        nonce: [u8; DEPLOYMENT_NONCE_LEN],
    ) -> Result<()> {
        instructions::deployment_consume::process_consume_deployment_authorization(ctx, nonce)
    }
}
