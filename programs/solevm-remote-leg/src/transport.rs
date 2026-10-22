//! Pooled accounting for allocations that ended without fully arriving.
//!
//! Custody is one fungible token account, so an asset that arrives late
//! cannot be traced to the allocation it was meant for. Impairment is
//! therefore one pool, and a recovery draws on the pool, never on a transfer.

use anchor_lang::prelude::*;

use crate::errors::RemoteLegError;
use crate::state::STATE_VERSION;

/// Seed prefix of the transport ledger.
pub const TRANSPORT_LEDGER_SEED: &[u8] = b"transport-ledger";

/// Every closed claim, split into what was credited, what is still impaired
/// and what was later recovered.
///
/// Amounts are wide because they accumulate over the life of the leg.
#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct TransportLedger {
    pub state_version: u8,
    pub bump: u8,
    pub impaired_outstanding: u128,
    pub cumulative_closed_claim: u128,
    pub cumulative_closed_credit: u128,
    pub cumulative_recovered: u128,
    /// Message sequence of the last allocation closed with a shortfall.
    pub last_impaired_sequence: u64,
    pub total_closed: u64,
    pub total_recoveries: u64,
    pub initialized_at: i64,
}

impl TransportLedger {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Address and bump of the ledger for one configuration.
    #[must_use]
    pub fn address(remote_config: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[TRANSPORT_LEDGER_SEED, remote_config.as_ref()], &crate::ID)
    }

    /// An empty ledger, created by the first close.
    #[must_use]
    pub const fn empty(bump: u8, initialized_at: i64) -> Self {
        Self {
            state_version: STATE_VERSION,
            bump,
            impaired_outstanding: 0,
            cumulative_closed_claim: 0,
            cumulative_closed_credit: 0,
            cumulative_recovered: 0,
            last_impaired_sequence: 0,
            total_closed: 0,
            total_recoveries: 0,
            initialized_at,
        }
    }

    /// Adds one closed allocation and returns its shortfall.
    pub fn record_close(&mut self, claim: u64, credited: u64, sequence: u64) -> Result<u64> {
        let shortfall = claim
            .checked_sub(credited)
            .ok_or(RemoteLegError::AttributionExceedsAuthorization)?;
        self.cumulative_closed_claim = add(self.cumulative_closed_claim, claim)?;
        self.cumulative_closed_credit = add(self.cumulative_closed_credit, credited)?;
        self.impaired_outstanding = add(self.impaired_outstanding, shortfall)?;
        if shortfall > 0 {
            self.last_impaired_sequence = sequence;
        }
        self.total_closed = self
            .total_closed
            .checked_add(1)
            .ok_or(RemoteLegError::ArithmeticOverflow)?;
        self.check_balanced()?;
        Ok(shortfall)
    }

    /// Moves recovered assets out of the impaired pool.
    pub fn record_recovery(&mut self, credited: u64) -> Result<()> {
        self.impaired_outstanding = self
            .impaired_outstanding
            .checked_sub(u128::from(credited))
            .ok_or(RemoteLegError::RecoveryStateMismatch)?;
        self.cumulative_recovered = add(self.cumulative_recovered, credited)?;
        self.total_recoveries = self
            .total_recoveries
            .checked_add(1)
            .ok_or(RemoteLegError::ArithmeticOverflow)?;
        self.check_balanced()
    }

    /// Every closed claim is credited, still impaired or recovered, exactly once.
    pub fn check_balanced(&self) -> Result<()> {
        let explained = self
            .cumulative_closed_credit
            .checked_add(self.impaired_outstanding)
            .and_then(|total| total.checked_add(self.cumulative_recovered))
            .ok_or(RemoteLegError::ArithmeticOverflow)?;
        require!(
            explained == self.cumulative_closed_claim,
            RemoteLegError::TransportLedgerImbalance
        );
        Ok(())
    }
}

fn add(total: u128, amount: u64) -> Result<u128> {
    total
        .checked_add(u128::from(amount))
        .ok_or_else(|| RemoteLegError::ArithmeticOverflow.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_close_adds_no_impairment() {
        let mut ledger = TransportLedger::empty(254, 1);
        assert_eq!(ledger.record_close(1_000, 1_000, 3), Ok(0));
        assert_eq!(ledger.impaired_outstanding, 0);
        assert_eq!(ledger.last_impaired_sequence, 0);
        assert_eq!(ledger.total_closed, 1);
    }

    #[test]
    fn a_short_close_pools_its_shortfall_and_remembers_the_allocation() {
        let mut ledger = TransportLedger::empty(254, 1);
        assert_eq!(ledger.record_close(1_000, 600, 3), Ok(400));
        assert_eq!(ledger.record_close(500, 0, 9), Ok(500));
        assert_eq!(ledger.impaired_outstanding, 900);
        assert_eq!(ledger.last_impaired_sequence, 9);
        assert_eq!(ledger.cumulative_closed_claim, 1_500);
        assert_eq!(ledger.cumulative_closed_credit, 600);
    }

    #[test]
    fn a_full_close_keeps_the_last_impaired_allocation() {
        let mut ledger = TransportLedger::empty(254, 1);
        assert_eq!(ledger.record_close(1_000, 600, 3), Ok(400));
        assert_eq!(ledger.record_close(1_000, 1_000, 7), Ok(0));
        assert_eq!(ledger.last_impaired_sequence, 3);
    }

    #[test]
    fn a_credit_above_the_claim_is_refused() {
        let mut ledger = TransportLedger::empty(254, 1);
        assert_eq!(
            ledger.record_close(1_000, 1_001, 3),
            Err(RemoteLegError::AttributionExceedsAuthorization.into())
        );
        assert_eq!(ledger, TransportLedger::empty(254, 1));
    }

    #[test]
    fn a_recovery_draws_on_the_pool_and_keeps_the_ledger_balanced() {
        let mut ledger = TransportLedger::empty(254, 1);
        assert_eq!(ledger.record_close(1_000, 600, 3), Ok(400));
        assert_eq!(ledger.record_recovery(150), Ok(()));
        assert_eq!(ledger.impaired_outstanding, 250);
        assert_eq!(ledger.cumulative_recovered, 150);
        assert_eq!(ledger.total_recoveries, 1);
        assert_eq!(ledger.check_balanced(), Ok(()));
    }

    #[test]
    fn a_recovery_above_the_pool_is_refused() {
        let mut ledger = TransportLedger::empty(254, 1);
        assert_eq!(ledger.record_close(1_000, 600, 3), Ok(400));
        assert_eq!(
            ledger.record_recovery(401),
            Err(RemoteLegError::RecoveryStateMismatch.into())
        );
    }

    #[test]
    fn a_zero_recovery_still_counts_as_one_settlement() {
        let mut ledger = TransportLedger::empty(254, 1);
        assert_eq!(ledger.record_close(1_000, 600, 3), Ok(400));
        assert_eq!(ledger.record_recovery(0), Ok(()));
        assert_eq!(ledger.impaired_outstanding, 400);
        assert_eq!(ledger.total_recoveries, 1);
    }

    #[test]
    fn a_tampered_ledger_does_not_balance() {
        let mut ledger = TransportLedger::empty(254, 1);
        assert_eq!(ledger.record_close(1_000, 600, 3), Ok(400));
        ledger.impaired_outstanding = 401;
        assert_eq!(
            ledger.check_balanced(),
            Err(RemoteLegError::TransportLedgerImbalance.into())
        );
    }
}
