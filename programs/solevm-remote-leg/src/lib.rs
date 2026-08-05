//! Solana strategy leg controlled by the canonical EVM vault.
//!
//! The leg holds no user shares, no user claims and no canonical value. It
//! only custodies the supported asset for the vault that controls it.

use anchor_lang::prelude::*;

pub mod control;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod message;
pub mod state;

pub use control::{
    CONSUMED_MESSAGE_SEED, ConsumedMessage, MAX_BASIS_POINTS, MessageClass, REPLAY_LANE_SEED,
    RISK_CONFIG_RESERVED, RISK_CONFIG_SEED, ReplayLane, RiskConfig,
};
pub use errors::RemoteLegError;
pub use events::{
    ConfigUpdated, ConsumedMessageClosed, ControlStateInitialized, RemoteLegFrozen,
    RemoteLegInitialized, ReplayWatermarkAdvanced,
};
pub use instructions::*;
pub use state::{
    CUSTODY_AUTHORITY_SEED, MIN_CONFIG_VERSION, REMOTE_CONFIG_RESERVED, REMOTE_CONFIG_SEED,
    REQUIRED_MINT_DECIMALS, RemoteConfig, STATE_VERSION,
};

declare_id!("4sLaaRdiY74cvqVCXLjW2wPncy2ArQkTRauNTdphKByo");

#[program]
pub mod solevm_remote_leg {
    use super::*;

    pub fn initialize_remote_leg(
        ctx: Context<InitializeRemoteLeg>,
        params: InitializeParams,
    ) -> Result<()> {
        instructions::initialize::process_initialize(ctx, params)
    }

    pub fn freeze_remote_leg(ctx: Context<FreezeRemoteLeg>) -> Result<()> {
        instructions::freeze::process_freeze(ctx)
    }

    pub fn initialize_control_state(
        ctx: Context<InitializeControlState>,
        params: ControlStateParams,
    ) -> Result<()> {
        instructions::control_state::process_initialize_control_state(ctx, params)
    }

    pub fn process_config_update(
        ctx: Context<ProcessConfigUpdate>,
        message_bytes: Vec<u8>,
    ) -> Result<()> {
        instructions::config_update::handle_config_update(ctx, message_bytes)
    }

    pub fn advance_replay_watermark(
        ctx: Context<AdvanceReplayWatermark>,
        message_class: MessageClass,
        new_minimum_sequence: u64,
    ) -> Result<()> {
        instructions::watermark::process_advance_replay_watermark(
            ctx,
            message_class,
            new_minimum_sequence,
        )
    }

    pub fn close_consumed_message(ctx: Context<CloseConsumedMessage>) -> Result<()> {
        instructions::close_record::process_close_consumed_message(ctx)
    }
}
