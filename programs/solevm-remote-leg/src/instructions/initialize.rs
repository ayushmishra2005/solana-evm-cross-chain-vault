//! One time setup of the remote leg.

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::errors::RemoteLegError;
use crate::events::RemoteLegInitialized;
use crate::state::{
    MIN_CONFIG_VERSION, REMOTE_CONFIG_RESERVED, REMOTE_CONFIG_SEED, REQUIRED_MINT_DECIMALS,
    RemoteConfig, STATE_VERSION,
};

/// Settings the administrator fixes for the life of the deployment.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct InitializeParams {
    pub deployment_id: [u8; 32],
    pub vault_id: [u8; 32],
    pub source_chain_id: u32,
    pub destination_chain_id: u32,
    pub source_application_id: [u8; 32],
    pub local_application_id: [u8; 32],
    pub control_lane_id: u32,
    pub report_lane_id: u32,
    pub config_version: u64,
    pub transport_verifier: Pubkey,
    pub emergency_guardian: Pubkey,
}

impl InitializeParams {
    /// Checks every value that does not depend on an account.
    pub fn validate(&self, administrator: &Pubkey) -> Result<()> {
        self.check_authorities(administrator)?;
        self.check_domains()
    }

    fn check_authorities(&self, administrator: &Pubkey) -> Result<()> {
        require_keys_neq!(
            *administrator,
            Pubkey::default(),
            RemoteLegError::InvalidAuthority
        );
        require_keys_neq!(
            self.emergency_guardian,
            Pubkey::default(),
            RemoteLegError::InvalidAuthority
        );
        require_keys_neq!(
            self.transport_verifier,
            Pubkey::default(),
            RemoteLegError::InvalidAuthority
        );
        require_keys_neq!(
            *administrator,
            self.emergency_guardian,
            RemoteLegError::EqualAuthorities
        );
        Ok(())
    }

    fn check_domains(&self) -> Result<()> {
        require_neq!(self.source_chain_id, 0, RemoteLegError::InvalidSourceDomain);
        require_neq!(
            self.destination_chain_id,
            0,
            RemoteLegError::InvalidDestinationDomain
        );
        require_neq!(
            self.source_chain_id,
            self.destination_chain_id,
            RemoteLegError::InvalidDestinationDomain
        );

        require!(
            self.source_application_id != [0u8; 32],
            RemoteLegError::InvalidApplication
        );
        require!(
            self.local_application_id != [0u8; 32],
            RemoteLegError::InvalidApplication
        );
        require!(
            self.source_application_id != self.local_application_id,
            RemoteLegError::InvalidApplication
        );

        require!(
            self.deployment_id != [0u8; 32],
            RemoteLegError::InvalidDeployment
        );
        require!(self.vault_id != [0u8; 32], RemoteLegError::InvalidVault);

        require_neq!(self.control_lane_id, 0, RemoteLegError::InvalidLane);
        require_neq!(self.report_lane_id, 0, RemoteLegError::InvalidLane);

        require_gte!(
            self.config_version,
            MIN_CONFIG_VERSION,
            RemoteLegError::InvalidConfigVersion
        );
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(params: InitializeParams)]
pub struct InitializeRemoteLeg<'info> {
    #[account(mut)]
    pub administrator: Signer<'info>,

    #[account(
        init,
        payer = administrator,
        space = RemoteConfig::LEN,
        seeds = [REMOTE_CONFIG_SEED, &params.deployment_id, &params.vault_id],
        bump,
    )]
    pub remote_config: Account<'info, RemoteConfig>,

    pub asset_mint: Account<'info, Mint>,

    // The account type already proves classic SPL Token ownership and layout.
    pub custody_token_account: Account<'info, TokenAccount>,

    pub outbound_escrow: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,

    pub system_program: Program<'info, System>,
}

pub fn process_initialize(
    ctx: Context<InitializeRemoteLeg>,
    params: InitializeParams,
) -> Result<()> {
    let administrator = ctx.accounts.administrator.key();
    let token_program = ctx.accounts.token_program.key();
    let asset_mint = ctx.accounts.asset_mint.key();

    params.validate(&administrator)?;

    require!(
        ctx.accounts.asset_mint.decimals == REQUIRED_MINT_DECIMALS,
        RemoteLegError::InvalidMintDecimals
    );

    let config_key = ctx.accounts.remote_config.key();
    let (custody_authority, custody_authority_bump) = RemoteConfig::custody_authority(&config_key);

    check_custody(&ctx, &asset_mint, &token_program, &custody_authority)?;
    check_outbound_escrow(&ctx, &asset_mint, &token_program, &custody_authority)?;

    let initialized_at = Clock::get()?.unix_timestamp;
    let custody_token_account = ctx.accounts.custody_token_account.key();
    let outbound_escrow = ctx.accounts.outbound_escrow.key();

    ctx.accounts.remote_config.set_inner(RemoteConfig {
        state_version: STATE_VERSION,
        bump: ctx.bumps.remote_config,
        custody_authority_bump,
        frozen: false,
        administrator,
        emergency_guardian: params.emergency_guardian,
        transport_verifier: params.transport_verifier,
        asset_mint,
        token_program,
        custody_authority,
        custody_token_account,
        outbound_escrow,
        source_chain_id: params.source_chain_id,
        destination_chain_id: params.destination_chain_id,
        source_application_id: params.source_application_id,
        local_application_id: params.local_application_id,
        deployment_id: params.deployment_id,
        vault_id: params.vault_id,
        control_lane_id: params.control_lane_id,
        report_lane_id: params.report_lane_id,
        config_version: params.config_version,
        initialized_at,
        reserved: [0u8; REMOTE_CONFIG_RESERVED],
    });

    emit!(RemoteLegInitialized {
        remote_config: config_key,
        administrator,
        emergency_guardian: params.emergency_guardian,
        transport_verifier: params.transport_verifier,
        asset_mint,
        custody_authority,
        custody_token_account,
        outbound_escrow,
        deployment_id: params.deployment_id,
        vault_id: params.vault_id,
        source_chain_id: params.source_chain_id,
        destination_chain_id: params.destination_chain_id,
        config_version: params.config_version,
        initialized_at,
    });

    Ok(())
}

fn check_custody(
    ctx: &Context<InitializeRemoteLeg>,
    asset_mint: &Pubkey,
    token_program: &Pubkey,
    custody_authority: &Pubkey,
) -> Result<()> {
    let custody = &ctx.accounts.custody_token_account;
    require_keys_eq!(
        *custody.to_account_info().owner,
        *token_program,
        RemoteLegError::InvalidTokenProgram
    );
    require_keys_eq!(custody.mint, *asset_mint, RemoteLegError::InvalidMint);
    require_keys_eq!(
        custody.owner,
        *custody_authority,
        RemoteLegError::InvalidCustodyAccount
    );
    require!(
        custody.delegate.is_none(),
        RemoteLegError::InvalidCustodyAccount
    );
    require!(
        custody.close_authority.is_none(),
        RemoteLegError::InvalidCustodyAccount
    );
    Ok(())
}

/// The escrow belongs to the future bridge, so it must not be our custody.
fn check_outbound_escrow(
    ctx: &Context<InitializeRemoteLeg>,
    asset_mint: &Pubkey,
    token_program: &Pubkey,
    custody_authority: &Pubkey,
) -> Result<()> {
    let escrow = &ctx.accounts.outbound_escrow;
    require_keys_eq!(
        *escrow.to_account_info().owner,
        *token_program,
        RemoteLegError::InvalidTokenProgram
    );
    require_keys_eq!(
        escrow.mint,
        *asset_mint,
        RemoteLegError::InvalidOutboundEscrow
    );
    require_keys_neq!(
        escrow.key(),
        ctx.accounts.custody_token_account.key(),
        RemoteLegError::InvalidOutboundEscrow
    );
    require_keys_neq!(
        escrow.owner,
        *custody_authority,
        RemoteLegError::InvalidOutboundEscrow
    );
    require!(
        escrow.close_authority.is_none(),
        RemoteLegError::InvalidOutboundEscrow
    );
    require!(
        escrow.delegate.is_none(),
        RemoteLegError::InvalidOutboundEscrow
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_params() -> InitializeParams {
        InitializeParams {
            deployment_id: [1u8; 32],
            vault_id: [2u8; 32],
            source_chain_id: 8453,
            destination_chain_id: 900,
            source_application_id: [3u8; 32],
            local_application_id: [4u8; 32],
            control_lane_id: 1,
            report_lane_id: 2,
            config_version: MIN_CONFIG_VERSION,
            transport_verifier: Pubkey::new_from_array([5u8; 32]),
            emergency_guardian: Pubkey::new_from_array([6u8; 32]),
        }
    }

    fn administrator() -> Pubkey {
        Pubkey::new_from_array([7u8; 32])
    }

    #[track_caller]
    fn expect(params: &InitializeParams, signer: &Pubkey, expected: RemoteLegError) {
        let error = params
            .validate(signer)
            .expect_err("validation should reject");
        assert_eq!(error, Error::from(expected));
    }

    #[test]
    fn a_complete_parameter_set_is_accepted() {
        assert!(valid_params().validate(&administrator()).is_ok());
    }

    #[test]
    fn a_default_administrator_is_rejected() {
        expect(
            &valid_params(),
            &Pubkey::default(),
            RemoteLegError::InvalidAuthority,
        );
    }

    #[test]
    fn a_default_guardian_is_rejected() {
        let mut params = valid_params();
        params.emergency_guardian = Pubkey::default();
        expect(&params, &administrator(), RemoteLegError::InvalidAuthority);
    }

    #[test]
    fn a_default_transport_verifier_is_rejected() {
        let mut params = valid_params();
        params.transport_verifier = Pubkey::default();
        expect(&params, &administrator(), RemoteLegError::InvalidAuthority);
    }

    #[test]
    fn an_administrator_that_is_also_the_guardian_is_rejected() {
        let mut params = valid_params();
        params.emergency_guardian = administrator();
        expect(&params, &administrator(), RemoteLegError::EqualAuthorities);
    }

    #[test]
    fn a_zero_source_chain_is_rejected() {
        let mut params = valid_params();
        params.source_chain_id = 0;
        expect(
            &params,
            &administrator(),
            RemoteLegError::InvalidSourceDomain,
        );
    }

    #[test]
    fn a_zero_destination_chain_is_rejected() {
        let mut params = valid_params();
        params.destination_chain_id = 0;
        expect(
            &params,
            &administrator(),
            RemoteLegError::InvalidDestinationDomain,
        );
    }

    #[test]
    fn matching_source_and_destination_chains_are_rejected() {
        let mut params = valid_params();
        params.destination_chain_id = params.source_chain_id;
        expect(
            &params,
            &administrator(),
            RemoteLegError::InvalidDestinationDomain,
        );
    }

    #[test]
    fn a_zero_application_is_rejected() {
        for zero_source in [true, false] {
            let mut params = valid_params();
            if zero_source {
                params.source_application_id = [0u8; 32];
            } else {
                params.local_application_id = [0u8; 32];
            }
            expect(
                &params,
                &administrator(),
                RemoteLegError::InvalidApplication,
            );
        }
    }

    #[test]
    fn matching_source_and_local_applications_are_rejected() {
        let mut params = valid_params();
        params.local_application_id = params.source_application_id;
        expect(
            &params,
            &administrator(),
            RemoteLegError::InvalidApplication,
        );
    }

    #[test]
    fn a_zero_deployment_is_rejected() {
        let mut params = valid_params();
        params.deployment_id = [0u8; 32];
        expect(&params, &administrator(), RemoteLegError::InvalidDeployment);
    }

    #[test]
    fn a_zero_vault_is_rejected() {
        let mut params = valid_params();
        params.vault_id = [0u8; 32];
        expect(&params, &administrator(), RemoteLegError::InvalidVault);
    }

    #[test]
    fn a_zero_lane_is_rejected() {
        for zero_control in [true, false] {
            let mut params = valid_params();
            if zero_control {
                params.control_lane_id = 0;
            } else {
                params.report_lane_id = 0;
            }
            expect(&params, &administrator(), RemoteLegError::InvalidLane);
        }
    }

    #[test]
    fn a_config_version_below_the_first_one_is_rejected() {
        let mut params = valid_params();
        params.config_version = MIN_CONFIG_VERSION - 1;
        expect(
            &params,
            &administrator(),
            RemoteLegError::InvalidConfigVersion,
        );
    }
}
