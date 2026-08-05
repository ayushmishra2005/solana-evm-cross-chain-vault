//! One time setup of the risk configuration and the three replay lanes.

use anchor_lang::prelude::*;

use crate::control::{
    MessageClass, REPLAY_LANE_SEED, RISK_CONFIG_RESERVED, RISK_CONFIG_SEED, ReplayLane, RiskConfig,
};
use crate::errors::RemoteLegError;
use crate::events::ControlStateInitialized;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};

/// Risk limits the administrator sets before any message is accepted.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct ControlStateParams {
    pub max_remote_allocation_bps: u16,
    pub max_upward_deviation_bps: u16,
    pub max_downward_deviation_bps: u16,
    pub max_report_age: u64,
    pub config_commitment: [u8; 32],
    pub mandatory_watermark_lag: u64,
}

impl ControlStateParams {
    /// Checks every value that does not depend on an account.
    pub fn validate(&self) -> Result<()> {
        RiskConfig::check_parameters(
            self.max_remote_allocation_bps,
            self.max_upward_deviation_bps,
            self.max_downward_deviation_bps,
            self.max_report_age,
            &self.config_commitment,
        )?;
        require_neq!(
            self.mandatory_watermark_lag,
            0,
            RemoteLegError::InvalidWatermark
        );
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeControlState<'info> {
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
    pub remote_config: Account<'info, RemoteConfig>,

    #[account(
        init,
        payer = administrator,
        space = RiskConfig::LEN,
        seeds = [RISK_CONFIG_SEED, remote_config.key().as_ref()],
        bump,
    )]
    pub risk_config: Account<'info, RiskConfig>,

    #[account(
        init,
        payer = administrator,
        space = ReplayLane::LEN,
        seeds = [
            REPLAY_LANE_SEED,
            remote_config.key().as_ref(),
            &[MessageClass::Allocate.to_u8()],
            &remote_config.control_lane_id.to_le_bytes(),
        ],
        bump,
    )]
    pub allocate_lane: Account<'info, ReplayLane>,

    #[account(
        init,
        payer = administrator,
        space = ReplayLane::LEN,
        seeds = [
            REPLAY_LANE_SEED,
            remote_config.key().as_ref(),
            &[MessageClass::Recall.to_u8()],
            &remote_config.control_lane_id.to_le_bytes(),
        ],
        bump,
    )]
    pub recall_lane: Account<'info, ReplayLane>,

    #[account(
        init,
        payer = administrator,
        space = ReplayLane::LEN,
        seeds = [
            REPLAY_LANE_SEED,
            remote_config.key().as_ref(),
            &[MessageClass::ConfigUpdate.to_u8()],
            &remote_config.control_lane_id.to_le_bytes(),
        ],
        bump,
    )]
    pub config_update_lane: Account<'info, ReplayLane>,

    pub system_program: Program<'info, System>,
}

pub fn process_initialize_control_state(
    ctx: Context<InitializeControlState>,
    params: ControlStateParams,
) -> Result<()> {
    params.validate()?;

    let config = &ctx.accounts.remote_config;
    require_eq!(
        config.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );

    let lane_id = config.control_lane_id;
    let initialized_at = Clock::get()?.unix_timestamp;

    ctx.accounts.risk_config.set_inner(RiskConfig {
        state_version: STATE_VERSION,
        bump: ctx.bumps.risk_config,
        max_remote_allocation_bps: params.max_remote_allocation_bps,
        max_upward_deviation_bps: params.max_upward_deviation_bps,
        max_downward_deviation_bps: params.max_downward_deviation_bps,
        max_report_age: params.max_report_age,
        config_version: config.config_version,
        config_commitment: params.config_commitment,
        initialized_at,
        last_update_at: initialized_at,
        reserved: [0u8; RISK_CONFIG_RESERVED],
    });

    let lanes = [
        (
            &mut ctx.accounts.allocate_lane,
            MessageClass::Allocate,
            ctx.bumps.allocate_lane,
        ),
        (
            &mut ctx.accounts.recall_lane,
            MessageClass::Recall,
            ctx.bumps.recall_lane,
        ),
        (
            &mut ctx.accounts.config_update_lane,
            MessageClass::ConfigUpdate,
            ctx.bumps.config_update_lane,
        ),
    ];

    for (lane, message_class, bump) in lanes {
        lane.set_inner(ReplayLane {
            state_version: STATE_VERSION,
            bump,
            message_class,
            lane_id,
            minimum_acceptable_sequence: ReplayLane::FIRST_SEQUENCE,
            highest_consumed_sequence: 0,
            message_commitment: [0u8; 32],
            mandatory_watermark_lag: params.mandatory_watermark_lag,
            last_accepted_at: 0,
        });
    }

    emit!(ControlStateInitialized {
        remote_config: config.key(),
        risk_config: ctx.accounts.risk_config.key(),
        administrator: ctx.accounts.administrator.key(),
        config_version: config.config_version,
        control_lane_id: lane_id,
        mandatory_watermark_lag: params.mandatory_watermark_lag,
        initialized_at,
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_params() -> ControlStateParams {
        ControlStateParams {
            max_remote_allocation_bps: 6_000,
            max_upward_deviation_bps: 200,
            max_downward_deviation_bps: 1_000,
            max_report_age: 3_600,
            config_commitment: [0xAA; 32],
            mandatory_watermark_lag: 8,
        }
    }

    #[track_caller]
    fn expect(params: &ControlStateParams, expected: RemoteLegError) {
        let error = params.validate().expect_err("validation should reject");
        assert_eq!(error, Error::from(expected));
    }

    #[test]
    fn a_complete_parameter_set_is_accepted() {
        assert!(valid_params().validate().is_ok());
    }

    #[test]
    fn basis_points_above_ten_thousand_are_rejected() {
        let setters: [fn(&mut ControlStateParams); 3] = [
            |params| params.max_remote_allocation_bps = 10_001,
            |params| params.max_upward_deviation_bps = 10_001,
            |params| params.max_downward_deviation_bps = 10_001,
        ];
        for set in setters {
            let mut params = valid_params();
            set(&mut params);
            expect(&params, RemoteLegError::InvalidBasisPoints);
        }
    }

    #[test]
    fn basis_points_at_exactly_ten_thousand_are_accepted() {
        let mut params = valid_params();
        params.max_remote_allocation_bps = 10_000;
        params.max_upward_deviation_bps = 10_000;
        params.max_downward_deviation_bps = 10_000;
        assert!(params.validate().is_ok());
    }

    #[test]
    fn a_zero_report_age_is_rejected() {
        let mut params = valid_params();
        params.max_report_age = 0;
        expect(&params, RemoteLegError::InvalidReportAge);
    }

    #[test]
    fn a_zero_config_commitment_is_rejected() {
        let mut params = valid_params();
        params.config_commitment = [0u8; 32];
        expect(&params, RemoteLegError::InvalidConfigCommitment);
    }

    #[test]
    fn a_zero_watermark_lag_is_rejected() {
        let mut params = valid_params();
        params.mandatory_watermark_lag = 0;
        expect(&params, RemoteLegError::InvalidWatermark);
    }
}
