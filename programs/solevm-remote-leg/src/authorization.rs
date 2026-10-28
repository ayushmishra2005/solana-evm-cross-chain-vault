//! The risk authorization every strategy deployment needs.
//!
//! Deploying is the one instruction that increases strategy exposure, so it
//! runs only directly after the authorization verifier consumed a fresh,
//! single-use approval for exactly this deployment. The verifier checks the
//! signature, risk key, network, policy generation, lifetime and nonce, and
//! that this deployment follows it. The leg checks, in turn, that the
//! approval was spent by the preceding instruction and names this namespace,
//! this amount and the state the leg holds now.
//!
//! There is no switch: without an adopted policy no deployment can run, and
//! an adopted policy cannot be changed or closed. Recalls, withdrawals,
//! sends, reports, reconciliation, recovery and freezing never read it.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::get_stack_height;
use es256_authorization_verifier::deployment::DeploymentContext;
use es256_authorization_verifier::{
    DEPLOYMENT_PROFILE_VERSION, DEPLOYMENT_RECEIPT_SEED, DeploymentReceipt,
};
use solana_instructions_sysvar::{load_current_index_checked, load_instruction_at_checked};

use crate::errors::RemoteLegError;
use crate::state::RemoteConfig;
use crate::strategy::{RemotePosition, StrategyConfig};

/// Seed prefix of the deployment authorization policy.
pub const DEPLOY_AUTHORIZATION_SEED: &[u8] = b"deploy-authorization";

/// Bytes held back for later fields of the same layout version.
pub const DEPLOY_AUTHORIZATION_RESERVED: usize = 32;

/// Account positions in the verifier's consume instruction.
const CONSUME_CONFIG_INDEX: usize = 1;
const CONSUME_RECEIPT_INDEX: usize = 3;

const TRANSACTION_LEVEL_STACK_HEIGHT: usize = 1;

/// Which verifier deployment authority approves this leg's deployments.
/// Set once, by the administrator, and never changed.
#[account]
#[derive(InitSpace, Debug)]
pub struct DeployAuthorizationPolicy {
    pub state_version: u8,
    pub bump: u8,
    /// The verifier's deployment authority for this leg and namespace.
    pub authorization_config: Pubkey,
    pub adopted_at: i64,
    pub reserved: [u8; DEPLOY_AUTHORIZATION_RESERVED],
}

impl DeployAuthorizationPolicy {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Seeds of the policy for one remote leg.
    #[must_use]
    pub fn seeds(remote_config: &Pubkey) -> [&[u8]; 2] {
        [DEPLOY_AUTHORIZATION_SEED, remote_config.as_ref()]
    }
}

/// The deployment an approval must name, read from the leg's own accounts
/// just before the deployment would run.
#[must_use]
pub fn deployment_context(
    remote_config_key: &Pubkey,
    remote_config: &RemoteConfig,
    strategy_config_key: &Pubkey,
    strategy_config: &StrategyConfig,
    position: &RemotePosition,
) -> DeploymentContext {
    DeploymentContext {
        remote_leg_program: crate::ID,
        remote_config: *remote_config_key,
        deployment_id: remote_config.deployment_id,
        vault_id: remote_config.vault_id,
        config_version: remote_config.config_version,
        strategy_config: *strategy_config_key,
        adapter_program: strategy_config.adapter_program,
        adapter_state: strategy_config.adapter_state,
        adapter_authority: strategy_config.adapter_authority,
        adapter_token_vault: strategy_config.adapter_token_vault,
        max_remote_principal: strategy_config.max_remote_principal,
        asset_mint: remote_config.asset_mint,
        token_program: remote_config.token_program,
        custody_authority: remote_config.custody_authority,
        custody_token_account: remote_config.custody_token_account,
        deployed_principal: position.deployed_principal,
    }
}

/// The approval this deployment spends.
pub struct Authorization<'a, 'info> {
    pub policy: &'a DeployAuthorizationPolicy,
    pub receipt_key: Pubkey,
    pub receipt: &'a DeploymentReceipt,
    pub instructions_sysvar: &'a AccountInfo<'info>,
}

impl Authorization<'_, '_> {
    /// Requires the preceding instruction to have consumed this receipt for
    /// this leg, and the receipt to name exactly this deployment.
    pub fn check(
        &self,
        remote_config: &Pubkey,
        context: &DeploymentContext,
        amount: u64,
        now: i64,
    ) -> Result<()> {
        self.check_consumed_just_before()?;

        let receipt = self.receipt;
        require!(
            receipt.version == DEPLOYMENT_PROFILE_VERSION
                && receipt.config == self.policy.authorization_config
                && receipt.remote_config == *remote_config
                && self.receipt_address_is_canonical(),
            RemoteLegError::InvalidAuthorizationReceipt
        );
        // Spent in this transaction, not carried over from an earlier one.
        require_eq!(
            receipt.consumed_at,
            now,
            RemoteLegError::InvalidAuthorizationReceipt
        );
        require!(
            now < receipt.expires_at,
            RemoteLegError::AuthorizationExpired
        );
        require_eq!(
            receipt.amount,
            amount,
            RemoteLegError::AuthorizationAmountMismatch
        );
        require!(
            receipt.deployment_context == context.commitment(),
            RemoteLegError::AuthorizationContextMismatch
        );
        Ok(())
    }

    fn check_consumed_just_before(&self) -> Result<()> {
        require!(
            get_stack_height() == TRANSACTION_LEVEL_STACK_HEIGHT,
            RemoteLegError::DeploymentNotTopLevel
        );
        let sysvar = self.instructions_sysvar;
        let current = usize::from(load_current_index_checked(sysvar)?);
        let this = load_instruction_at_checked(current, sysvar)?;
        require_keys_eq!(
            this.program_id,
            crate::ID,
            RemoteLegError::DeploymentNotTopLevel
        );

        let consume = current
            .checked_sub(1)
            .and_then(|index| load_instruction_at_checked(index, sysvar).ok())
            .ok_or(RemoteLegError::DeploymentAuthorizationRequired)?;
        let mut expected_data = es256_authorization_verifier::instruction::ConsumeDeploymentAuthorization::DISCRIMINATOR.to_vec();
        expected_data.extend_from_slice(&self.receipt.nonce);
        let named = |index: usize| consume.accounts.get(index).map(|meta| meta.pubkey);
        require!(
            consume.program_id == es256_authorization_verifier::ID
                && consume.data == expected_data
                && named(CONSUME_CONFIG_INDEX) == Some(self.policy.authorization_config)
                && named(CONSUME_RECEIPT_INDEX) == Some(self.receipt_key),
            RemoteLegError::DeploymentAuthorizationRequired
        );
        Ok(())
    }

    fn receipt_address_is_canonical(&self) -> bool {
        Pubkey::create_program_address(
            &[
                DEPLOYMENT_RECEIPT_SEED,
                self.receipt.config.as_ref(),
                &self.receipt.nonce,
                &[self.receipt.bump],
            ],
            &es256_authorization_verifier::ID,
        )
        .is_ok_and(|address| address == self.receipt_key)
    }
}
