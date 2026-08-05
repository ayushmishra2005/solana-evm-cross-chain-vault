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
