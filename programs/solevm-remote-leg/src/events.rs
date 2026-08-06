//! Events emitted by the remote leg.

use anchor_lang::prelude::*;

use crate::control::MessageClass;

/// Emitted once, when the configuration account is created.
#[event]
#[derive(Debug)]
pub struct RemoteLegInitialized {
    pub remote_config: Pubkey,
    pub administrator: Pubkey,
    pub emergency_guardian: Pubkey,
    pub transport_verifier: Pubkey,
    pub asset_mint: Pubkey,
    pub custody_authority: Pubkey,
    pub custody_token_account: Pubkey,
    pub outbound_escrow: Pubkey,
    pub deployment_id: [u8; 32],
    pub vault_id: [u8; 32],
    pub source_chain_id: u32,
    pub destination_chain_id: u32,
    pub config_version: u64,
    pub initialized_at: i64,
}

/// Emitted once, when the leg moves to its terminal frozen state.
#[event]
#[derive(Debug)]
pub struct RemoteLegFrozen {
    pub remote_config: Pubkey,
    pub authority: Pubkey,
    pub deployment_id: [u8; 32],
    pub vault_id: [u8; 32],
    pub config_version: u64,
    pub frozen_at: i64,
}

/// Emitted once, when the risk config and the three replay lanes are created.
#[event]
#[derive(Debug)]
pub struct ControlStateInitialized {
    pub remote_config: Pubkey,
    pub risk_config: Pubkey,
    pub administrator: Pubkey,
    pub config_version: u64,
    pub control_lane_id: u32,
    pub mandatory_watermark_lag: u64,
    pub initialized_at: i64,
}

/// Emitted for every accepted canonical config update.
#[event]
#[derive(Debug)]
pub struct ConfigUpdated {
    pub remote_config: Pubkey,
    pub risk_config: Pubkey,
    pub message_id: [u8; 32],
    pub sequence: u64,
    pub previous_config_version: u64,
    pub new_config_version: u64,
    pub lane_commitment: [u8; 32],
    pub updated_at: i64,
}

/// Emitted when the administrator raises a lane watermark.
#[event]
#[derive(Debug)]
pub struct ReplayWatermarkAdvanced {
    pub remote_config: Pubkey,
    pub replay_lane: Pubkey,
    pub message_class: MessageClass,
    pub lane_id: u32,
    pub previous_minimum_sequence: u64,
    pub new_minimum_sequence: u64,
    pub highest_consumed_sequence: u64,
    pub advanced_at: i64,
}

/// Emitted once, when the adapter identity and the position are created.
#[event]
#[derive(Debug)]
pub struct StrategyStateInitialized {
    pub remote_config: Pubkey,
    pub strategy_config: Pubkey,
    pub remote_position: Pubkey,
    pub adapter_program: Pubkey,
    pub adapter_state: Pubkey,
    pub adapter_authority: Pubkey,
    pub adapter_token_vault: Pubkey,
    pub max_remote_principal: u64,
    pub initialized_at: i64,
}

/// Emitted when custody holds more than the leg had explained.
#[event]
#[derive(Debug)]
pub struct CustodyReconciled {
    pub remote_config: Pubkey,
    pub remote_position: Pubkey,
    pub observed_surplus: u64,
    pub unattributed_custody: u64,
    pub reconciled_at: i64,
}

/// Emitted for every accepted canonical allocation message.
#[event]
#[derive(Debug)]
pub struct AllocateAccepted {
    pub remote_config: Pubkey,
    pub transfer_record: Pubkey,
    pub transfer_id: [u8; 32],
    pub message_id: [u8; 32],
    pub sequence: u64,
    pub authorized_amount: u64,
    pub minimum_amount: u64,
    pub lane_commitment: [u8; 32],
    pub accepted_at: i64,
}

/// Emitted whenever observed custody becomes accepted principal.
#[event]
#[derive(Debug)]
pub struct AllocationAttributed {
    pub remote_config: Pubkey,
    pub transfer_record: Pubkey,
    pub transfer_id: [u8; 32],
    pub attributed_now: u64,
    pub attributed_total: u64,
    pub authorized_amount: u64,
    pub unattributed_custody: u64,
    pub attributed_at: i64,
}

/// Emitted once the full authorized amount has arrived.
#[event]
#[derive(Debug)]
pub struct AllocationCompleted {
    pub remote_config: Pubkey,
    pub transfer_record: Pubkey,
    pub transfer_id: [u8; 32],
    pub authorized_amount: u64,
    pub attributed_amount: u64,
    pub completed_at: i64,
}

/// Emitted for every deposit the leg makes into the adapter.
#[event]
#[derive(Debug)]
pub struct AssetsDeployed {
    pub remote_config: Pubkey,
    pub adapter_state: Pubkey,
    pub deployed_now: u64,
    pub attributed_principal: u64,
    pub deployed_principal: u64,
    pub deployed_at: i64,
}

/// Emitted for every accepted canonical recall message.
#[event]
#[derive(Debug)]
pub struct RecallAccepted {
    pub remote_config: Pubkey,
    pub transfer_record: Pubkey,
    pub transfer_id: [u8; 32],
    pub message_id: [u8; 32],
    pub sequence: u64,
    pub requested_amount: u64,
    pub minimum_amount: u64,
    pub lane_commitment: [u8; 32],
    pub accepted_at: i64,
}

/// Emitted when local custody is set aside for a recall, with no transfer.
#[event]
#[derive(Debug)]
pub struct RecallCustodyReserved {
    pub remote_config: Pubkey,
    pub transfer_record: Pubkey,
    pub transfer_id: [u8; 32],
    pub reserved_amount: u64,
    pub remaining_principal: u64,
    pub reserved_at: i64,
}

/// Emitted for every adapter withdrawal that a recall triggered.
#[event]
#[derive(Debug)]
pub struct StrategyWithdrawalCompleted {
    pub remote_config: Pubkey,
    pub transfer_record: Pubkey,
    pub transfer_id: [u8; 32],
    pub principal_reduction: u64,
    pub assets_returned: u64,
    pub realized_loss: u64,
    pub deployed_principal: u64,
    pub withdrawn_at: i64,
}

/// Emitted for every send of recalled custody to the fixed escrow.
#[event]
#[derive(Debug)]
pub struct RecallAssetsSent {
    pub remote_config: Pubkey,
    pub transfer_record: Pubkey,
    pub transfer_id: [u8; 32],
    pub amount_sent: u64,
    pub total_sent: u64,
    pub outbound_escrow: Pubkey,
    pub sent_at: i64,
}

/// Emitted once sent assets plus loss match the requested principal.
#[event]
#[derive(Debug)]
pub struct RecallCompleted {
    pub remote_config: Pubkey,
    pub transfer_record: Pubkey,
    pub transfer_id: [u8; 32],
    pub requested_amount: u64,
    pub assets_sent: u64,
    pub realized_loss: u64,
    pub completed_at: i64,
}

/// Emitted when a settled record is closed and its rent returns.
#[event]
#[derive(Debug)]
pub struct ConsumedMessageClosed {
    pub remote_config: Pubkey,
    pub consumed_message: Pubkey,
    pub message_class: MessageClass,
    pub lane_id: u32,
    pub sequence: u64,
    pub message_id: [u8; 32],
    pub rent_destination: Pubkey,
    pub closed_at: i64,
}
