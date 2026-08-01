//! Chain independent reference model for the SolEVM Vault accounting rules.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::integer_division
)]

extern crate alloc;

mod amount;
mod error;
mod invariant;
mod math;
mod operation;
mod request;
mod state;
mod transition;

pub use amount::{AccountId, AssetAmount, ConfigVersion, EpochId, ShareAmount, Timestamp};
pub use error::Rejection;
pub use invariant::{Violation, check_invariants};
pub use math::{PricingBasis, assets_to_shares, mul_div_floor, shares_to_assets};
pub use operation::Operation;
pub use request::{DepositRequest, RedeemRequest, RequestKey, RequestState};
pub use state::{
    AbortedTerms, Account, Authority, Buckets, Config, Entitlements, Epoch, EpochOutcome,
    EpochPhase, EpochTerms, Genesis, RequestTotals, State, VaultState,
};
pub use transition::apply;
