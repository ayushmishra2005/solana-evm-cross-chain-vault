use accounting_model::{AccountId, AssetAmount, EpochId, Operation, ShareAmount, Timestamp};

/// Number of ordinary users in every scenario.
pub const USER_COUNT: usize = 4;

/// Actor slot of the administrator.
pub const ADMIN_SLOT: u8 = 4;
/// Actor slot of the emergency guardian.
pub const GUARDIAN_SLOT: u8 = 5;
/// Total addressable actor slots.
pub const ACTOR_COUNT: usize = 6;

/// Stable account ids. Solidity derives its addresses from the same slots.
const USER_BASE_ACCOUNT: u64 = 1;
const ADMIN_ACCOUNT: u64 = 100;
const GUARDIAN_ACCOUNT: u64 = 101;

/// Maps an actor slot onto the account id the model uses.
#[must_use]
pub fn account_for(slot: u8) -> AccountId {
    match slot {
        ADMIN_SLOT => AccountId::new(ADMIN_ACCOUNT),
        GUARDIAN_SLOT => AccountId::new(GUARDIAN_ACCOUNT),
        user => AccountId::new(USER_BASE_ACCOUNT + u64::from(user)),
    }
}

/// The operations both implementations share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ActionKind {
    RequestDeposit = 0,
    CancelDeposit = 1,
    RequestRedeem = 2,
    CancelRedeem = 3,
    CutoffEpoch = 4,
    FinalizeEpoch = 5,
    OpenNextEpoch = 6,
    ClaimDeposit = 7,
    ClaimRedeem = 8,
    RefundDeposit = 9,
    RefundRedeem = 10,
    Pause = 11,
    Unpause = 12,
    Freeze = 13,
    AbortEpoch = 14,
}

impl ActionKind {
    pub const ALL: [Self; 15] = [
        Self::RequestDeposit,
        Self::CancelDeposit,
        Self::RequestRedeem,
        Self::CancelRedeem,
        Self::CutoffEpoch,
        Self::FinalizeEpoch,
        Self::OpenNextEpoch,
        Self::ClaimDeposit,
        Self::ClaimRedeem,
        Self::RefundDeposit,
        Self::RefundRedeem,
        Self::Pause,
        Self::Unpause,
        Self::Freeze,
        Self::AbortEpoch,
    ];

    #[must_use]
    pub const fn raw(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn is_cancellation(self) -> bool {
        matches!(self, Self::CancelDeposit | Self::CancelRedeem)
    }

    #[must_use]
    pub const fn is_refund(self) -> bool {
        matches!(self, Self::RefundDeposit | Self::RefundRedeem)
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::RequestDeposit => "RequestDeposit",
            Self::CancelDeposit => "CancelDeposit",
            Self::RequestRedeem => "RequestRedeem",
            Self::CancelRedeem => "CancelRedeem",
            Self::CutoffEpoch => "CutoffEpoch",
            Self::FinalizeEpoch => "FinalizeEpoch",
            Self::OpenNextEpoch => "OpenNextEpoch",
            Self::ClaimDeposit => "ClaimDeposit",
            Self::ClaimRedeem => "ClaimRedeem",
            Self::RefundDeposit => "RefundDeposit",
            Self::RefundRedeem => "RefundRedeem",
            Self::Pause => "Pause",
            Self::Unpause => "Unpause",
            Self::Freeze => "Freeze",
            Self::AbortEpoch => "AbortEpoch",
        }
    }
}

/// One operation with everything both sides need to run it identically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action {
    pub kind: ActionKind,
    pub actor: u8,
    pub amount: u128,
    pub epoch: u64,
    pub timestamp: u64,
}

impl Action {
    #[must_use]
    pub const fn new(
        kind: ActionKind,
        actor: u8,
        amount: u128,
        epoch: u64,
        timestamp: u64,
    ) -> Self {
        Self {
            kind,
            actor,
            amount,
            epoch,
            timestamp,
        }
    }

    /// Translates into the model operation. Time reaches the model only where
    /// the model reads it.
    #[must_use]
    pub fn to_operation(self) -> Operation {
        let account = account_for(self.actor);
        let epoch = EpochId::new(self.epoch);
        let now = Timestamp::new(self.timestamp);
        match self.kind {
            ActionKind::RequestDeposit => Operation::RequestDeposit {
                account,
                assets: AssetAmount::new(self.amount),
            },
            ActionKind::CancelDeposit => Operation::CancelDeposit { account },
            ActionKind::RequestRedeem => Operation::RequestRedeem {
                account,
                shares: ShareAmount::new(self.amount),
            },
            ActionKind::CancelRedeem => Operation::CancelRedeem { account },
            ActionKind::CutoffEpoch => Operation::CutoffEpoch { now },
            ActionKind::FinalizeEpoch => Operation::FinalizeEpoch,
            ActionKind::OpenNextEpoch => Operation::OpenNextEpoch { now },
            ActionKind::ClaimDeposit => Operation::ClaimDeposit { account, epoch },
            ActionKind::ClaimRedeem => Operation::ClaimRedeem { account, epoch },
            ActionKind::RefundDeposit => Operation::ClaimAbortedDeposit { account, epoch },
            ActionKind::RefundRedeem => Operation::ClaimAbortedRedeem { account, epoch },
            ActionKind::Pause => Operation::Pause { actor: account },
            ActionKind::Unpause => Operation::Unpause { actor: account },
            ActionKind::Freeze => Operation::Freeze { actor: account },
            ActionKind::AbortEpoch => Operation::AbortEpoch { actor: account },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_slots_map_to_stable_accounts() {
        assert_eq!(account_for(0).raw(), 1);
        assert_eq!(account_for(1).raw(), 2);
        assert_eq!(account_for(2).raw(), 3);
        assert_eq!(account_for(3).raw(), 4);
        assert_eq!(account_for(ADMIN_SLOT).raw(), 100);
        assert_eq!(account_for(GUARDIAN_SLOT).raw(), 101);
    }

    #[test]
    fn every_actor_slot_has_its_own_account() {
        let mut seen = Vec::new();
        for slot in 0..ACTOR_COUNT {
            let id = account_for(slot as u8).raw();
            assert!(!seen.contains(&id));
            seen.push(id);
        }
    }

    #[test]
    fn kinds_keep_their_wire_numbers() {
        assert_eq!(ActionKind::RequestDeposit.raw(), 0);
        assert_eq!(ActionKind::AbortEpoch.raw(), 14);
        for (index, kind) in ActionKind::ALL.iter().enumerate() {
            assert_eq!(usize::from(kind.raw()), index);
        }
    }

    #[test]
    fn refunds_and_cancellations_are_flagged() {
        assert!(ActionKind::CancelDeposit.is_cancellation());
        assert!(ActionKind::CancelRedeem.is_cancellation());
        assert!(ActionKind::RefundDeposit.is_refund());
        assert!(ActionKind::RefundRedeem.is_refund());
        assert!(!ActionKind::ClaimDeposit.is_refund());
        assert!(!ActionKind::ClaimDeposit.is_cancellation());
    }
}
