use accounting_model::{
    AccountId, AssetAmount, EpochId, Operation, ShareAmount, Timestamp, TransferId,
};

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
    BeginAllocate = 15,
    CompleteAllocation = 16,
    BeginRecall = 17,
    AcceptRecallSent = 18,
    ObserveInbound = 19,
    ApplyRemoteValuation = 20,
    LockRemoteValuation = 21,
    RequestClose = 22,
    SettleClose = 23,
    BeginRecover = 24,
    SettleRecover = 25,
}

impl ActionKind {
    pub const ALL: [Self; 26] = [
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
        Self::BeginAllocate,
        Self::CompleteAllocation,
        Self::BeginRecall,
        Self::AcceptRecallSent,
        Self::ObserveInbound,
        Self::ApplyRemoteValuation,
        Self::LockRemoteValuation,
        Self::RequestClose,
        Self::SettleClose,
        Self::BeginRecover,
        Self::SettleRecover,
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
            Self::BeginAllocate => "BeginAllocate",
            Self::CompleteAllocation => "CompleteAllocation",
            Self::BeginRecall => "BeginRecall",
            Self::AcceptRecallSent => "AcceptRecallSent",
            Self::ObserveInbound => "ObserveInbound",
            Self::ApplyRemoteValuation => "ApplyRemoteValuation",
            Self::LockRemoteValuation => "LockRemoteValuation",
            Self::RequestClose => "RequestClose",
            Self::SettleClose => "SettleClose",
            Self::BeginRecover => "BeginRecover",
            Self::SettleRecover => "SettleRecover",
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
    pub aux0: u128,
    pub aux1: u128,
}

fn transfer_id_from(aux0: u128) -> TransferId {
    let mut raw = [0u8; 32];
    raw[16..32].copy_from_slice(&aux0.to_be_bytes());
    TransferId::new(raw)
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
            aux0: 0,
            aux1: 0,
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
            ActionKind::BeginAllocate => Operation::BeginAllocate {
                actor: account,
                transfer_id: transfer_id_from(self.aux0),
                amount: AssetAmount::new(self.amount),
            },
            ActionKind::CompleteAllocation => Operation::CompleteAllocation {
                transfer_id: transfer_id_from(self.aux0),
                reported_value: AssetAmount::new(self.amount),
            },
            ActionKind::BeginRecall => Operation::BeginRecall {
                actor: account,
                transfer_id: transfer_id_from(self.aux0),
                amount: AssetAmount::new(self.amount),
            },
            ActionKind::AcceptRecallSent => Operation::AcceptRecallSent {
                transfer_id: transfer_id_from(self.aux0),
                principal: AssetAmount::new(self.amount),
                actual: AssetAmount::new(self.aux1),
                loss: AssetAmount::new(self.amount.saturating_sub(self.aux1)),
            },
            ActionKind::ObserveInbound => Operation::ObserveInbound {
                amount: AssetAmount::new(self.amount),
            },
            ActionKind::ApplyRemoteValuation => Operation::ApplyRemoteValuation {
                reported_value: AssetAmount::new(self.amount),
                newly_realized_loss: AssetAmount::new(self.aux1),
            },
            ActionKind::LockRemoteValuation => Operation::LockRemoteValuation,
            ActionKind::RequestClose => Operation::RequestClose {
                actor: account,
                transfer_id: transfer_id_from(self.aux0),
            },
            ActionKind::SettleClose => Operation::SettleClose {
                transfer_id: transfer_id_from(self.aux0),
                credited: AssetAmount::new(self.amount),
            },
            ActionKind::BeginRecover => Operation::BeginRecover {
                actor: account,
                transfer_id: transfer_id_from(self.aux0),
            },
            ActionKind::SettleRecover => Operation::SettleRecover {
                transfer_id: transfer_id_from(self.aux0),
                credited: AssetAmount::new(self.amount),
            },
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
        assert_eq!(ActionKind::LockRemoteValuation.raw(), 21);
        assert_eq!(ActionKind::RequestClose.raw(), 22);
        assert_eq!(ActionKind::SettleRecover.raw(), 25);
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
