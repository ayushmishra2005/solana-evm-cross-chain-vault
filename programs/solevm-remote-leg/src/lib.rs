//! Solana strategy leg controlled by the canonical EVM vault.
//!
//! The leg holds no user shares, no user claims and no canonical value. It
//! only custodies the supported asset for the vault that controls it.

use anchor_lang::prelude::*;

pub mod adapter;
pub mod control;
pub mod custody;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod message;
pub mod records;
pub mod state;
pub mod strategy;

pub use control::{
    CONSUMED_MESSAGE_SEED, ConsumedMessage, MAX_BASIS_POINTS, MessageClass, REPLAY_LANE_SEED,
    RISK_CONFIG_RESERVED, RISK_CONFIG_SEED, ReplayLane, RiskConfig, SequenceRule,
};
pub use errors::RemoteLegError;
pub use events::{
    AllocateAccepted, AllocationAttributed, AllocationCompleted, AssetsDeployed, ConfigUpdated,
    ConsumedMessageClosed, ControlStateInitialized, CustodyReconciled, RecallAccepted,
    RecallAssetsSent, RecallCompleted, RecallCustodyReserved, RemoteLegFrozen,
    RemoteLegInitialized, ReplayWatermarkAdvanced, StrategyStateInitialized,
    StrategyWithdrawalCompleted,
};
pub use instructions::*;
pub use state::{
    CUSTODY_AUTHORITY_SEED, MIN_CONFIG_VERSION, REMOTE_CONFIG_RESERVED, REMOTE_CONFIG_SEED,
    REQUIRED_MINT_DECIMALS, RemoteConfig, STATE_VERSION,
};
pub use strategy::{
    REMOTE_POSITION_SEED, RemotePosition, STRATEGY_CONFIG_RESERVED, STRATEGY_CONFIG_SEED,
    StrategyConfig, TRANSFER_RECORD_SEED, TransferKind, TransferRecord, TransferStatus,
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

    pub fn initialize_strategy_state(
        ctx: Context<InitializeStrategyState>,
        max_remote_principal: u64,
    ) -> Result<()> {
        instructions::strategy_state::process_initialize_strategy_state(ctx, max_remote_principal)
    }

    pub fn reconcile_custody(ctx: Context<ReconcileCustody>) -> Result<()> {
        instructions::reconcile::process_reconcile_custody(ctx)
    }

    pub fn process_allocate(ctx: Context<ProcessAllocate>, message_bytes: Vec<u8>) -> Result<()> {
        instructions::allocate::handle_allocate(ctx, message_bytes)
    }

    pub fn attribute_allocation(ctx: Context<AttributeAllocation>) -> Result<()> {
        instructions::attribute::process_attribute_allocation(ctx)
    }

    pub fn deploy_to_strategy(ctx: Context<DeployToStrategy>, maximum_amount: u64) -> Result<()> {
        instructions::deploy::process_deploy_to_strategy(ctx, maximum_amount)
    }

    pub fn process_recall(ctx: Context<ProcessRecall>, message_bytes: Vec<u8>) -> Result<()> {
        instructions::recall::handle_recall(ctx, message_bytes)
    }

    pub fn withdraw_for_recall(
        ctx: Context<WithdrawForRecall>,
        maximum_principal: u64,
    ) -> Result<()> {
        instructions::withdraw::process_withdraw_for_recall(ctx, maximum_principal)
    }

    pub fn send_recall(ctx: Context<SendRecall>, maximum_amount: u64) -> Result<()> {
        instructions::send::process_send_recall(ctx, maximum_amount)
    }
}
