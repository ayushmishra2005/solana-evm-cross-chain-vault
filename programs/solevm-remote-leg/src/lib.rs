//! Solana strategy leg controlled by the canonical EVM vault.
//!
//! The leg holds no user shares, no user claims and no canonical value. It
//! only custodies the supported asset for the vault that controls it.

use anchor_lang::prelude::*;

pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

pub use errors::RemoteLegError;
pub use events::{RemoteLegFrozen, RemoteLegInitialized};
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
}
