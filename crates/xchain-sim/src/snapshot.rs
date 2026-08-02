//! In memory copies of a run.
//!
//! A snapshot lets two fault plans start from one shared history, so their
//! final digests can be compared.

extern crate alloc;

use crate::error::SimError;
use crate::operation::Operation;
use crate::simulator::Simulator;
use crate::state_hash::StateHash;

/// A frozen copy of the whole simulator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    state: Simulator,
    hash: StateHash,
}

impl Snapshot {
    #[must_use]
    pub fn new(state: &Simulator) -> Self {
        Self {
            state: state.clone(),
            hash: state.state_hash(),
        }
    }

    /// The digest taken when the snapshot was made.
    #[must_use]
    pub const fn state_hash(&self) -> StateHash {
        self.hash
    }

    /// A fresh simulator that starts where the snapshot was taken.
    #[must_use]
    pub fn restore(&self) -> Simulator {
        self.state.clone()
    }

    /// Restores, then runs a list of steps on the copy.
    pub fn branch(&self, operations: &[Operation]) -> Result<Simulator, SimError> {
        let mut copy = self.restore();
        copy.apply_all(operations)?;
        Ok(copy)
    }
}

impl Simulator {
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::EndpointId;
    use crate::lane::Lane;

    fn simulator() -> Simulator {
        Simulator::new(&[EndpointId::new(1), EndpointId::new(2)]).unwrap_or_else(|_| {
            unreachable!("two distinct endpoints always build");
        })
    }

    #[test]
    fn a_snapshot_keeps_the_digest_of_the_moment_it_was_taken() {
        let mut sim = simulator();
        let snapshot = sim.snapshot();
        sim.pause_lane(Lane::Control);
        assert_eq!(snapshot.state_hash(), snapshot.restore().state_hash());
        assert_ne!(snapshot.state_hash(), sim.state_hash());
    }

    #[test]
    fn two_branches_from_one_snapshot_do_not_share_state() {
        let sim = simulator();
        let snapshot = sim.snapshot();
        let left = snapshot
            .branch(&[Operation::PauseLane(Lane::Control)])
            .unwrap_or_else(|_| unreachable!("pausing always works"));
        let right = snapshot
            .branch(&[Operation::PauseLane(Lane::Asset)])
            .unwrap_or_else(|_| unreachable!("pausing always works"));
        assert_ne!(left.state_hash(), right.state_hash());
        assert_eq!(snapshot.restore().state_hash(), snapshot.state_hash());
    }

    #[test]
    fn a_branch_reports_the_first_refusal() {
        let sim = simulator();
        let snapshot = sim.snapshot();
        let outcome = snapshot.branch(&[Operation::HaltEndpoint(EndpointId::new(9))]);
        assert!(outcome.is_err());
    }
}
