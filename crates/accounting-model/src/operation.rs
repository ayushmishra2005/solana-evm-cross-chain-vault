use crate::amount::{AccountId, AssetAmount, EpochId, ShareAmount, Timestamp};

/// Every state change the model accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    RequestDeposit {
        account: AccountId,
        assets: AssetAmount,
    },
    CancelDeposit {
        account: AccountId,
    },
    RequestRedeem {
        account: AccountId,
        shares: ShareAmount,
    },
    CancelRedeem {
        account: AccountId,
    },
    CutoffEpoch {
        now: Timestamp,
    },
    FinalizeEpoch,
    AbortEpoch {
        actor: AccountId,
    },
    ClaimDeposit {
        account: AccountId,
        epoch: EpochId,
    },
    ClaimRedeem {
        account: AccountId,
        epoch: EpochId,
    },
    ClaimAbortedDeposit {
        account: AccountId,
        epoch: EpochId,
    },
    ClaimAbortedRedeem {
        account: AccountId,
        epoch: EpochId,
    },
    OpenNextEpoch {
        now: Timestamp,
    },
    Pause {
        actor: AccountId,
    },
    Unpause {
        actor: AccountId,
    },
    Freeze {
        actor: AccountId,
    },
}
