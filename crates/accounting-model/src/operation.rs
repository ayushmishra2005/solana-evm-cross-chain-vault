use crate::amount::{AccountId, AssetAmount, EpochId, ShareAmount, Timestamp, TransferId};

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
    BeginAllocate {
        actor: AccountId,
        transfer_id: TransferId,
        amount: AssetAmount,
    },
    CompleteAllocation {
        transfer_id: TransferId,
        reported_value: AssetAmount,
    },
    BeginRecall {
        actor: AccountId,
        transfer_id: TransferId,
        amount: AssetAmount,
    },
    AcceptRecallSent {
        transfer_id: TransferId,
        principal: AssetAmount,
        actual: AssetAmount,
        loss: AssetAmount,
    },
    ObserveInbound {
        amount: AssetAmount,
    },
    ApplyRemoteValuation {
        reported_value: AssetAmount,
    },
    LockRemoteValuation,
}
