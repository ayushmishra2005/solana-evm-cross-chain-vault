use crate::action::{ADMIN_SLOT, ActionKind, GUARDIAN_SLOT, USER_COUNT};
use crate::rng::Rng;
use crate::scenario::{BuildError, Builder, Scenario, Setup};

/// Number of scenario families the generator cycles through.
pub const FAMILY_COUNT: u8 = 7;

pub const FAMILY_DEPOSIT: u8 = 0;
pub const FAMILY_REDEMPTION: u8 = 1;
pub const FAMILY_MIXED: u8 = 2;
pub const FAMILY_MULTI_EPOCH: u8 = 3;
pub const FAMILY_PAUSE: u8 = 4;
pub const FAMILY_FREEZE: u8 = 5;
pub const FAMILY_REJECTION: u8 = 6;

#[must_use]
pub const fn family_name(family: u8) -> &'static str {
    match family {
        FAMILY_DEPOSIT => "DepositLifecycle",
        FAMILY_REDEMPTION => "RedemptionLifecycle",
        FAMILY_MIXED => "MixedSettlement",
        FAMILY_MULTI_EPOCH => "MultiEpoch",
        FAMILY_PAUSE => "PauseBehavior",
        FAMILY_FREEZE => "FreezeAndAbort",
        FAMILY_REJECTION => "RejectionTrace",
        _ => "Unknown",
    }
}

const BASE_TIMESTAMP: u64 = 1_700_000_000;

/// Builds the settings for one scenario from its index and seed.
#[must_use]
pub fn setup_for(index: u32, seed: u64, rng: &mut Rng) -> Setup {
    let family = u8::try_from(u64::from(index) % u64::from(FAMILY_COUNT)).unwrap_or(0);
    let min_deposit = if rng.chance(1, 4) { 500_000 } else { 1_000_000 };
    let min_redeem = if rng.chance(1, 4) {
        500_000_000_000
    } else {
        1_000_000_000_000
    };
    let epoch_duration = rng.between(600, 7_200);
    let start_timestamp = BASE_TIMESTAMP + rng.between(0, 100_000);

    let mut initial_assets = [0u128; USER_COUNT];
    for slot in &mut initial_assets {
        *slot = rng.between_u128(60_000_000, 900_000_000);
    }

    Setup {
        index,
        seed,
        family,
        start_timestamp,
        epoch_duration,
        min_deposit,
        min_redeem,
        config_version: 1,
        initial_assets,
    }
}

/// Produces one complete scenario.
pub fn generate(setup: Setup, rng: &mut Rng, rounds: u32) -> Result<Scenario, BuildError> {
    let mut builder = Builder::new(setup)?;
    match setup.family {
        FAMILY_DEPOSIT => deposit_lifecycle(&mut builder, rng, setup),
        FAMILY_REDEMPTION => redemption_lifecycle(&mut builder, rng, setup),
        FAMILY_MIXED => mixed_settlement(&mut builder, rng, setup),
        FAMILY_MULTI_EPOCH => multi_epoch(&mut builder, rng, setup, rounds),
        FAMILY_PAUSE => pause_behavior(&mut builder, rng, setup),
        FAMILY_FREEZE => freeze_and_abort(&mut builder, rng, setup),
        _ => rejection_trace(&mut builder, rng, setup),
    }
    builder.finish()
}

// Shared steps

fn any_user(rng: &mut Rng) -> u8 {
    u8::try_from(rng.below(USER_COUNT as u64)).unwrap_or(0)
}

fn cutoff(builder: &mut Builder, rng: &mut Rng) {
    let target = builder.current_cutoff_at();
    builder.advance_to(target.saturating_add(rng.below(45)));
    let caller = any_user(rng);
    builder.exec_simple(ActionKind::CutoffEpoch, caller);
}

fn finalize(builder: &mut Builder, rng: &mut Rng) {
    let caller = any_user(rng);
    builder.exec_simple(ActionKind::FinalizeEpoch, caller);
}

fn settle(builder: &mut Builder, rng: &mut Rng) {
    cutoff(builder, rng);
    finalize(builder, rng);
}

fn open_next(builder: &mut Builder, rng: &mut Rng) {
    builder.advance_by(rng.between(1, 300));
    let caller = any_user(rng);
    builder.exec_simple(ActionKind::OpenNextEpoch, caller);
}

/// Picks a deposit the user can actually afford, favouring uneven amounts.
fn deposit_amount(builder: &Builder, rng: &mut Rng, user: usize, setup: Setup) -> u128 {
    let balance = builder.user_assets(user);
    if balance < setup.min_deposit {
        return setup.min_deposit;
    }
    let ceiling = balance
        .min(setup.min_deposit.saturating_mul(30))
        .max(setup.min_deposit);
    rng.between_u128(setup.min_deposit, ceiling)
}

/// Picks a redemption the user can cover, favouring uneven amounts.
fn redeem_amount(builder: &Builder, rng: &mut Rng, user: usize, setup: Setup) -> u128 {
    let held = builder.user_shares(user);
    if held < setup.min_redeem {
        return setup.min_redeem;
    }
    let ceiling = held.saturating_sub(held / 4).max(setup.min_redeem);
    rng.between_u128(setup.min_redeem, ceiling)
}

fn request_deposit(builder: &mut Builder, rng: &mut Rng, user: u8, setup: Setup) {
    let amount = deposit_amount(builder, rng, usize::from(user), setup);
    builder.exec(ActionKind::RequestDeposit, user, amount, 0);
}

fn request_redeem(builder: &mut Builder, rng: &mut Rng, user: u8, setup: Setup) {
    let amount = redeem_amount(builder, rng, usize::from(user), setup);
    builder.exec(ActionKind::RequestRedeem, user, amount, 0);
}

/// Runs one full deposit epoch so every user ends up holding shares.
fn bootstrap_shares(builder: &mut Builder, rng: &mut Rng, setup: Setup) {
    let epoch = builder.current_epoch_id();
    for user in 0..USER_COUNT {
        let slot = u8::try_from(user).unwrap_or(0);
        request_deposit(builder, rng, slot, setup);
    }
    settle(builder, rng);
    for user in 0..USER_COUNT {
        let slot = u8::try_from(user).unwrap_or(0);
        builder.exec(ActionKind::ClaimDeposit, slot, 0, epoch);
    }
}

// Families

fn deposit_lifecycle(builder: &mut Builder, rng: &mut Rng, setup: Setup) {
    let epoch = builder.current_epoch_id();

    request_deposit(builder, rng, 0, setup);
    request_deposit(builder, rng, 0, setup);
    request_deposit(builder, rng, 1, setup);
    builder.advance_by(rng.between(1, 60));

    builder.exec_simple(ActionKind::CancelDeposit, 0);
    builder.exec_simple(ActionKind::CancelDeposit, 0);
    request_deposit(builder, rng, 0, setup);
    request_deposit(builder, rng, 2, setup);

    builder.exec(ActionKind::RequestDeposit, 3, 0, 0);
    builder.exec(
        ActionKind::RequestDeposit,
        3,
        setup.min_deposit.saturating_sub(1),
        0,
    );

    cutoff(builder, rng);
    builder.exec_simple(ActionKind::CancelDeposit, 1);
    builder.exec(ActionKind::RequestDeposit, 2, setup.min_deposit, 0);
    builder.exec_simple(ActionKind::CutoffEpoch, 0);
    builder.exec(ActionKind::ClaimDeposit, 0, 0, epoch);
    finalize(builder, rng);

    builder.exec(ActionKind::ClaimDeposit, 0, 0, epoch);
    builder.exec(ActionKind::ClaimDeposit, 0, 0, epoch);
    builder.exec(ActionKind::ClaimDeposit, 1, 0, epoch);
    builder.exec(ActionKind::ClaimDeposit, 3, 0, epoch);
    builder.exec(ActionKind::RefundDeposit, 1, 0, epoch);

    open_next(builder, rng);
    builder.exec(ActionKind::ClaimDeposit, 2, 0, epoch);
}

fn redemption_lifecycle(builder: &mut Builder, rng: &mut Rng, setup: Setup) {
    bootstrap_shares(builder, rng, setup);
    open_next(builder, rng);
    let epoch = builder.current_epoch_id();

    request_redeem(builder, rng, 0, setup);
    request_redeem(builder, rng, 0, setup);
    request_redeem(builder, rng, 1, setup);
    builder.advance_by(rng.between(1, 60));

    builder.exec_simple(ActionKind::CancelRedeem, 0);
    builder.exec_simple(ActionKind::CancelRedeem, 0);
    request_redeem(builder, rng, 0, setup);

    builder.exec(ActionKind::RequestRedeem, 2, 0, 0);
    builder.exec(
        ActionKind::RequestRedeem,
        2,
        setup.min_redeem.saturating_sub(1),
        0,
    );

    cutoff(builder, rng);
    builder.exec_simple(ActionKind::CancelRedeem, 1);
    builder.exec(ActionKind::RequestRedeem, 2, setup.min_redeem, 0);
    finalize(builder, rng);

    builder.exec(ActionKind::ClaimRedeem, 0, 0, epoch);
    builder.exec(ActionKind::ClaimRedeem, 0, 0, epoch);
    builder.exec(ActionKind::ClaimRedeem, 1, 0, epoch);
    builder.exec(ActionKind::ClaimRedeem, 3, 0, epoch);
    builder.exec(ActionKind::RefundRedeem, 1, 0, epoch);
}

fn mixed_settlement(builder: &mut Builder, rng: &mut Rng, setup: Setup) {
    bootstrap_shares(builder, rng, setup);
    open_next(builder, rng);
    let epoch = builder.current_epoch_id();

    request_deposit(builder, rng, 0, setup);
    request_deposit(builder, rng, 1, setup);
    request_redeem(builder, rng, 2, setup);
    request_redeem(builder, rng, 3, setup);
    request_deposit(builder, rng, 2, setup);
    request_redeem(builder, rng, 0, setup);

    settle(builder, rng);

    for user in 0..USER_COUNT {
        let slot = u8::try_from(user).unwrap_or(0);
        builder.exec(ActionKind::ClaimDeposit, slot, 0, epoch);
        builder.exec(ActionKind::ClaimRedeem, slot, 0, epoch);
    }
    open_next(builder, rng);
}

fn multi_epoch(builder: &mut Builder, rng: &mut Rng, setup: Setup, rounds: u32) {
    bootstrap_shares(builder, rng, setup);
    let mut pending: Vec<u64> = Vec::new();
    let cycles = rounds.clamp(3, 5);

    for _ in 0..cycles {
        open_next(builder, rng);
        let epoch = builder.current_epoch_id();

        let depositors = rng.between(1, 2);
        for offset in 0..depositors {
            let user = u8::try_from(offset % (USER_COUNT as u64)).unwrap_or(0);
            request_deposit(builder, rng, user, setup);
        }
        let redeemer = any_user(rng);
        request_redeem(builder, rng, redeemer, setup);

        settle(builder, rng);
        pending.push(epoch);

        // Claim an older epoch so entitlements survive across settlements.
        if pending.len() > 1
            && let Some(older) = pending.first().copied()
        {
            for user in 0..USER_COUNT {
                let slot = u8::try_from(user).unwrap_or(0);
                builder.exec(ActionKind::ClaimDeposit, slot, 0, older);
                builder.exec(ActionKind::ClaimRedeem, slot, 0, older);
            }
            pending.remove(0);
        }
    }

    for epoch in pending {
        for user in 0..USER_COUNT {
            let slot = u8::try_from(user).unwrap_or(0);
            builder.exec(ActionKind::ClaimDeposit, slot, 0, epoch);
            builder.exec(ActionKind::ClaimRedeem, slot, 0, epoch);
        }
    }
}

fn pause_behavior(builder: &mut Builder, rng: &mut Rng, setup: Setup) {
    let epoch = builder.current_epoch_id();
    request_deposit(builder, rng, 0, setup);
    request_deposit(builder, rng, 1, setup);

    builder.exec_simple(ActionKind::Pause, 0);
    builder.exec_simple(ActionKind::Pause, ADMIN_SLOT);
    builder.exec_simple(ActionKind::Pause, ADMIN_SLOT);

    builder.exec(ActionKind::RequestDeposit, 2, setup.min_deposit, 0);
    builder.exec(ActionKind::RequestRedeem, 2, setup.min_redeem, 0);
    builder.exec_simple(ActionKind::CancelDeposit, 1);
    builder.exec_simple(ActionKind::CutoffEpoch, 0);
    builder.exec_simple(ActionKind::FinalizeEpoch, 0);

    builder.exec_simple(ActionKind::Unpause, GUARDIAN_SLOT);
    builder.exec_simple(ActionKind::Unpause, 0);
    builder.exec_simple(ActionKind::Unpause, ADMIN_SLOT);
    builder.exec_simple(ActionKind::Unpause, ADMIN_SLOT);

    request_deposit(builder, rng, 1, setup);
    request_deposit(builder, rng, 2, setup);
    settle(builder, rng);

    builder.exec_simple(ActionKind::Pause, GUARDIAN_SLOT);
    builder.exec(ActionKind::ClaimDeposit, 0, 0, epoch);
    builder.exec(ActionKind::ClaimDeposit, 1, 0, epoch);
    builder.exec_simple(ActionKind::OpenNextEpoch, 0);
    builder.exec_simple(ActionKind::Unpause, ADMIN_SLOT);
    open_next(builder, rng);
    builder.exec(ActionKind::ClaimDeposit, 2, 0, epoch);
}

fn freeze_and_abort(builder: &mut Builder, rng: &mut Rng, setup: Setup) {
    bootstrap_shares(builder, rng, setup);
    let settled = builder.current_epoch_id();
    open_next(builder, rng);
    let epoch = builder.current_epoch_id();

    request_deposit(builder, rng, 0, setup);
    request_deposit(builder, rng, 1, setup);
    request_redeem(builder, rng, 2, setup);
    request_redeem(builder, rng, 3, setup);

    // Half of these runs freeze after cutoff instead of during the open phase.
    if rng.chance(1, 2) {
        cutoff(builder, rng);
    }

    builder.exec(ActionKind::RefundDeposit, 0, 0, epoch);
    builder.exec_simple(ActionKind::Freeze, 0);
    builder.exec_simple(ActionKind::Freeze, GUARDIAN_SLOT);
    builder.exec_simple(ActionKind::Freeze, ADMIN_SLOT);

    builder.exec(ActionKind::RequestDeposit, 0, setup.min_deposit, 0);
    builder.exec_simple(ActionKind::CancelDeposit, 0);
    builder.exec_simple(ActionKind::CancelRedeem, 2);
    builder.exec_simple(ActionKind::CutoffEpoch, 0);
    builder.exec_simple(ActionKind::FinalizeEpoch, 0);
    builder.exec_simple(ActionKind::Unpause, ADMIN_SLOT);

    builder.exec(ActionKind::RefundDeposit, 0, 0, epoch);
    builder.exec_simple(ActionKind::AbortEpoch, 0);
    builder.exec_simple(ActionKind::AbortEpoch, GUARDIAN_SLOT);
    builder.exec_simple(ActionKind::AbortEpoch, GUARDIAN_SLOT);

    builder.exec(ActionKind::RefundDeposit, 0, 0, epoch);
    builder.exec(ActionKind::RefundDeposit, 0, 0, epoch);
    builder.exec(ActionKind::RefundDeposit, 1, 0, epoch);
    builder.exec(ActionKind::RefundRedeem, 2, 0, epoch);
    builder.exec(ActionKind::RefundRedeem, 2, 0, epoch);
    builder.exec(ActionKind::RefundRedeem, 3, 0, epoch);
    builder.exec(ActionKind::RefundDeposit, 2, 0, epoch);
    builder.exec(ActionKind::ClaimDeposit, 0, 0, epoch);

    // The older finalized epoch keeps working after the freeze.
    builder.exec(ActionKind::ClaimDeposit, 0, 0, settled);
    builder.exec(ActionKind::ClaimRedeem, 2, 0, settled);
    builder.exec_simple(ActionKind::OpenNextEpoch, 0);
}

fn rejection_trace(builder: &mut Builder, rng: &mut Rng, setup: Setup) {
    let epoch = builder.current_epoch_id();

    builder.exec(ActionKind::RequestDeposit, 0, 0, 0);
    builder.exec(
        ActionKind::RequestDeposit,
        0,
        setup.min_deposit.saturating_sub(1),
        0,
    );
    builder.exec(ActionKind::RequestDeposit, 0, u128::from(u64::MAX), 0);
    builder.exec(ActionKind::RequestRedeem, 0, setup.min_redeem, 0);
    builder.exec_simple(ActionKind::CancelDeposit, 0);
    builder.exec_simple(ActionKind::CancelRedeem, 0);
    builder.exec_simple(ActionKind::CutoffEpoch, 0);
    builder.exec_simple(ActionKind::FinalizeEpoch, 0);
    builder.exec(ActionKind::ClaimDeposit, 0, 0, epoch);
    builder.exec(ActionKind::ClaimRedeem, 0, 0, epoch);
    builder.exec(ActionKind::RefundDeposit, 0, 0, epoch);
    builder.exec(ActionKind::RefundRedeem, 0, 0, epoch);
    builder.exec_simple(ActionKind::Unpause, ADMIN_SLOT);
    builder.exec_simple(ActionKind::AbortEpoch, GUARDIAN_SLOT);
    builder.exec_simple(ActionKind::OpenNextEpoch, 0);

    request_deposit(builder, rng, 0, setup);
    request_deposit(builder, rng, 1, setup);
    builder.exec_simple(ActionKind::CancelDeposit, 2);

    settle(builder, rng);

    builder.exec_simple(ActionKind::FinalizeEpoch, 0);
    builder.exec_simple(ActionKind::CutoffEpoch, 0);
    builder.exec(ActionKind::ClaimDeposit, 0, 0, epoch);
    builder.exec(ActionKind::ClaimDeposit, 0, 0, epoch);
    builder.exec(ActionKind::RefundDeposit, 0, 0, epoch);
    builder.exec(ActionKind::ClaimDeposit, 2, 0, epoch);
    builder.exec(ActionKind::ClaimRedeem, 1, 0, epoch);
    builder.exec(ActionKind::ClaimDeposit, 0, 0, epoch.saturating_add(5));

    open_next(builder, rng);
    builder.exec_simple(ActionKind::OpenNextEpoch, 0);
    builder.exec_simple(ActionKind::CutoffEpoch, 0);
}
