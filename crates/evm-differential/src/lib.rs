//! Deterministic scenario producer for the Rust to Solidity differential run.
//!
//! The accounting model is the reference. This crate walks it through scripted
//! traces, records the result and state of every step, and emits one bundle the
//! Foundry harness replays against the vault.

#![cfg_attr(test, allow(clippy::panic, clippy::unwrap_used, clippy::expect_used))]

pub mod abi;
pub mod action;
pub mod generator;
pub mod result;
pub mod rng;
pub mod scenario;
pub mod snapshot;

use crate::generator::{generate, setup_for};
use crate::rng::Rng;
use crate::scenario::{BuildError, Scenario};

/// Inputs that fully determine a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunConfig {
    pub seed: u64,
    pub cases: u32,
    pub steps: u32,
    /// Produces only this scenario index when set, for reproducing a failure.
    pub only: Option<u32>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            seed: 1,
            cases: 64,
            steps: 4,
            only: None,
        }
    }
}

/// Builds every scenario for a run. The same config always gives the same list.
pub fn build_run(config: RunConfig) -> Result<Vec<Scenario>, BuildError> {
    let mut scenarios = Vec::new();
    for index in 0..config.cases {
        if let Some(wanted) = config.only
            && wanted != index
        {
            continue;
        }
        // A per scenario stream keeps one scenario independent of the others.
        let seed = derive_seed(config.seed, index);
        let mut rng = Rng::new(seed);
        let setup = setup_for(index, seed, &mut rng);
        scenarios.push(generate(setup, &mut rng, config.steps)?);
    }
    Ok(scenarios)
}

/// Mixes the run seed with the scenario index so indexes stay independent.
#[must_use]
pub fn derive_seed(seed: u64, index: u32) -> u64 {
    let mut rng = Rng::new(seed ^ (u64::from(index).wrapping_mul(0x9E37_79B9_7F4A_7C15)));
    rng.next_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_reproducible() {
        let config = RunConfig {
            seed: 5,
            cases: 7,
            steps: 3,
            only: None,
        };
        let first = build_run(config).unwrap_or_else(|reason| panic!("{reason}"));
        let second = build_run(config).unwrap_or_else(|reason| panic!("{reason}"));
        assert_eq!(first, second);
    }

    #[test]
    fn a_different_seed_changes_the_run() {
        let base = RunConfig {
            seed: 5,
            cases: 7,
            steps: 3,
            only: None,
        };
        let other = RunConfig { seed: 6, ..base };
        assert_ne!(
            build_run(base).unwrap_or_else(|reason| panic!("{reason}")),
            build_run(other).unwrap_or_else(|reason| panic!("{reason}"))
        );
    }

    #[test]
    fn selecting_one_index_reproduces_that_scenario() {
        let config = RunConfig {
            seed: 9,
            cases: 8,
            steps: 3,
            only: None,
        };
        let all = build_run(config).unwrap_or_else(|reason| panic!("{reason}"));
        let single = build_run(RunConfig {
            only: Some(5),
            ..config
        })
        .unwrap_or_else(|reason| panic!("{reason}"));

        assert_eq!(single.len(), 1);
        assert_eq!(single.first(), all.get(5));
    }

    #[test]
    fn scenario_seeds_differ_across_indexes() {
        let mut seen = Vec::new();
        for index in 0..64 {
            let seed = derive_seed(3, index);
            assert!(!seen.contains(&seed));
            seen.push(seed);
        }
    }
}
