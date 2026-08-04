//! Events emitted by the remote leg.

use anchor_lang::prelude::*;

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
