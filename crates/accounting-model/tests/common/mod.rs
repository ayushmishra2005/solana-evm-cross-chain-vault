#![allow(clippy::expect_used, clippy::panic, dead_code, unreachable_pub)]

use accounting_model::{
    AccountId, AssetAmount, Authority, Config, ConfigVersion, EpochId, Genesis, Operation,
    ShareAmount, State, Timestamp, apply, check_invariants,
};

pub const ADMIN: AccountId = AccountId::new(1);
pub const GUARDIAN: AccountId = AccountId::new(2);
pub const OUTSIDER: AccountId = AccountId::new(3);
pub const ALICE: AccountId = AccountId::new(10);
pub const BOB: AccountId = AccountId::new(11);
pub const CAROL: AccountId = AccountId::new(12);

pub const EPOCH_DURATION: u64 = 3_600;
pub const MIN_DEPOSIT: u128 = 1_000_000;
pub const MIN_REDEEM: u128 = 1_000_000_000_000;
pub const FUNDING: u128 = 1_000_000_000_000;

pub fn config() -> Config {
    Config {
        version: ConfigVersion::new(1),
        asset_decimals: 6,
        share_decimals: 18,
        min_deposit_assets: AssetAmount::new(MIN_DEPOSIT),
        min_redeem_shares: ShareAmount::new(MIN_REDEEM),
        epoch_duration: EPOCH_DURATION,
    }
}

pub fn authority() -> Authority {
    Authority {
        admin: ADMIN,
        guardian: GUARDIAN,
    }
}

pub fn genesis() -> State {
    State::new(Genesis {
        config: config(),
        authority: authority(),
        accounts: [ALICE, BOB, CAROL]
            .into_iter()
            .map(|id| (id, AssetAmount::new(FUNDING)))
            .collect(),
        unattributed_balance: AssetAmount::new(500),
        opened_at: Timestamp::new(0),
    })
    .expect("valid genesis")
}

pub fn genesis_funding_only(account: AccountId, assets: u128) -> State {
    State::new(Genesis {
        config: config(),
        authority: authority(),
        accounts: vec![(account, AssetAmount::new(assets))],
        unattributed_balance: AssetAmount::ZERO,
        opened_at: Timestamp::new(0),
    })
    .expect("valid genesis")
}

pub fn run(state: &State, operations: &[Operation]) -> State {
    let mut current = state.clone();
    for operation in operations {
        current = apply(&current, *operation)
            .unwrap_or_else(|reason| panic!("operation {operation:?} rejected: {reason}"));
        check_invariants(&current).unwrap_or_else(|violation| panic!("{violation}"));
    }
    current
}

pub fn deposit(account: AccountId, assets: u128) -> Operation {
    Operation::RequestDeposit {
        account,
        assets: AssetAmount::new(assets),
    }
}

pub fn redeem(account: AccountId, shares: u128) -> Operation {
    Operation::RequestRedeem {
        account,
        shares: ShareAmount::new(shares),
    }
}

pub fn claim_deposit(account: AccountId, epoch: u64) -> Operation {
    Operation::ClaimDeposit {
        account,
        epoch: EpochId::new(epoch),
    }
}

pub fn claim_redeem(account: AccountId, epoch: u64) -> Operation {
    Operation::ClaimRedeem {
        account,
        epoch: EpochId::new(epoch),
    }
}

pub fn refund_deposit(account: AccountId, epoch: u64) -> Operation {
    Operation::ClaimAbortedDeposit {
        account,
        epoch: EpochId::new(epoch),
    }
}

pub fn refund_redeem(account: AccountId, epoch: u64) -> Operation {
    Operation::ClaimAbortedRedeem {
        account,
        epoch: EpochId::new(epoch),
    }
}

pub fn cutoff(at: u64) -> Operation {
    Operation::CutoffEpoch {
        now: Timestamp::new(at),
    }
}

pub fn open_next(at: u64) -> Operation {
    Operation::OpenNextEpoch {
        now: Timestamp::new(at),
    }
}

pub fn settle_epoch(state: &State, at: u64) -> State {
    run(state, &[cutoff(at), Operation::FinalizeEpoch])
}

/// Deposits, settles the genesis epoch and hands the shares to the account.
pub fn funded_with_shares(account: AccountId, assets: u128) -> State {
    let state = run(&genesis(), &[deposit(account, assets)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    run(
        &state,
        &[claim_deposit(account, 0), open_next(EPOCH_DURATION)],
    )
}
