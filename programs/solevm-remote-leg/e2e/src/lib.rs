//! Deterministic driver for live Solana remote-leg execution over xchain-sim.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::too_many_arguments
)]

#[path = "../../tests/common/mod.rs"]
mod common;

pub mod hexutil;
pub mod ops;
pub mod trace;
pub mod world;

pub use ops::{Script, WireOp};
pub use world::{Observation, World};

pub fn run_script(input: &str) -> Result<String, String> {
    let observation = World::run_json(input)?;
    serde_json::to_string(&observation).map_err(|error| error.to_string())
}
