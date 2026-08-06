//! Rules that must hold after every accepted or rejected operation.
//!
//! Each case drives a randomized but deterministic trace, then checks the
//! accounting identities the leg promises.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::{ALLOCATE_TRANSFER_ID, Fixture, RECALL_TRANSFER_ID, expect_error, expect_rejection};
use solevm_remote_leg::{RemoteLegError, TransferKind, TransferStatus};

const AMOUNT: u64 = 1_000_000;

/// A small reproducible generator, so a failing case is easy to rerun.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }

    /// A value in `1..=limit`, never zero.
    fn upto(&mut self, limit: u64) -> u64 {
        (self.next() % limit) + 1
    }
}

/// Every identity the leg keeps between instructions.
#[track_caller]
fn check_identities(fixture: &Fixture) {
    let position = fixture.position();
    let custody = fixture.token_amount(fixture.custody);
    assert_eq!(
        custody,
        position.attributed_principal + position.recalled_custody + position.unattributed_custody,
        "custody must equal the sum of its buckets"
    );
    assert_eq!(
        fixture.adapter().principal,
        position.deployed_principal,
        "adapter principal must equal deployed principal"
    );
}

#[track_caller]
fn check_one_open_cycle(fixture: &Fixture) {
    let position = fixture.position();
    if position.active_transfer_kind == TransferKind::None {
        assert_eq!(position.active_transfer_status, TransferStatus::None);
        return;
    }
    let record = fixture.transfer(&position.active_transfer_id);
    assert_eq!(record.status, TransferStatus::Pending);
    assert_eq!(record.transfer_kind, position.active_transfer_kind);
}

#[test]
fn partial_arrivals_never_break_custody_reconciliation() {
    for seed in 0..24u64 {
        let mut random = Seeded(seed);
        let mut fixture = Fixture::deployed();
        fixture.accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);

        let mut delivered = 0;
        while delivered < AMOUNT {
            let slice = random.upto(AMOUNT / 3).min(AMOUNT - delivered);
            fixture.credit(fixture.custody, slice);
            delivered += slice;
            let _ = fixture.attribute(ALLOCATE_TRANSFER_ID);
            check_identities(&fixture);
            check_one_open_cycle(&fixture);

            let record = fixture.transfer(&ALLOCATE_TRANSFER_ID);
            assert!(record.attributed_amount <= record.authorized_amount);
            assert_eq!(record.attributed_amount, delivered);
        }

        assert_eq!(
            fixture.transfer(&ALLOCATE_TRANSFER_ID).status,
            TransferStatus::Complete
        );
        assert_eq!(fixture.position().attributed_principal, AMOUNT);
    }
}

#[test]
fn excess_custody_stays_unattributed_through_any_arrival_order() {
    for seed in 0..16u64 {
        let mut random = Seeded(seed);
        let mut fixture = Fixture::deployed();
        let excess = random.upto(AMOUNT);

        fixture.credit(fixture.custody, excess);
        fixture.reconcile().expect("reconciliation lands");
        assert_eq!(fixture.position().unattributed_custody, excess);

        fixture.accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);
        fixture.credit(fixture.custody, AMOUNT);
        fixture
            .attribute(ALLOCATE_TRANSFER_ID)
            .expect("attribution lands");

        let position = fixture.position();
        assert_eq!(position.attributed_principal, AMOUNT);
        assert_eq!(position.unattributed_custody, excess);
        check_identities(&fixture);
    }
}

#[test]
fn repeated_reconciliation_is_idempotent() {
    let mut fixture = Fixture::deployed();
    fixture.credit(fixture.custody, AMOUNT);
    fixture.reconcile().expect("first reconciliation lands");
    let after_first = fixture.position();

    for _ in 0..4 {
        fixture.reconcile().expect("reconciliation lands");
        let position = fixture.position();
        assert_eq!(
            position.unattributed_custody,
            after_first.unattributed_custody
        );
        assert_eq!(
            position.attributed_principal,
            after_first.attributed_principal
        );
        assert_eq!(position.deployed_principal, after_first.deployed_principal);
        assert_eq!(position.recalled_custody, after_first.recalled_custody);
        check_identities(&fixture);
    }
}

#[test]
fn deposits_of_any_size_keep_the_adapter_and_the_position_equal() {
    for seed in 0..16u64 {
        let mut random = Seeded(seed);
        let mut fixture = Fixture::deployed();
        fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);

        let mut deployed = 0;
        while deployed < AMOUNT {
            let slice = random.upto(AMOUNT / 3);
            fixture.deploy(slice).expect("deposit lands");
            deployed = fixture.position().deployed_principal;
            check_identities(&fixture);
        }
        assert_eq!(deployed, AMOUNT);
        assert_eq!(fixture.position().attributed_principal, 0);
    }
}

#[test]
fn partial_withdrawals_never_lose_or_invent_principal() {
    for seed in 0..16u64 {
        let mut random = Seeded(seed);
        let mut fixture = Fixture::deployed();
        fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
        fixture.deploy(AMOUNT).expect("deposit lands");

        let liquidity = random.upto(AMOUNT / 2);
        let loss_bps = (random.next() % 2_000) as u16;
        fixture
            .configure_adapter(liquidity, loss_bps, false)
            .expect("the adapter takes the test conditions");
        fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);

        while fixture.position().deployed_principal > 0 {
            fixture
                .withdraw(RECALL_TRANSFER_ID, AMOUNT)
                .expect("withdrawal lands");
            check_identities(&fixture);

            let record = fixture.transfer(&RECALL_TRANSFER_ID);
            assert!(record.realized_loss <= record.strategy_principal_resolved);
            assert_eq!(
                record.strategy_principal_resolved,
                record.assets_withdrawn + record.realized_loss
            );
        }

        let record = fixture.transfer(&RECALL_TRANSFER_ID);
        assert_eq!(record.strategy_principal_resolved, AMOUNT);
        assert_eq!(
            fixture.position().cumulative_realized_loss,
            record.realized_loss
        );
    }
}

#[test]
fn sent_assets_plus_loss_never_exceed_the_request() {
    for seed in 0..16u64 {
        let mut random = Seeded(seed);
        let mut fixture = Fixture::deployed();
        fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
        fixture.deploy(AMOUNT).expect("deposit lands");

        let loss_bps = (random.next() % 3_000) as u16;
        fixture
            .configure_adapter(AMOUNT, loss_bps, false)
            .expect("the adapter takes the test conditions");
        fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
        fixture
            .withdraw(RECALL_TRANSFER_ID, AMOUNT)
            .expect("withdrawal lands");

        while fixture.position().recalled_custody > 0 {
            let slice = random.upto(AMOUNT / 3);
            fixture
                .send_recall(RECALL_TRANSFER_ID, slice)
                .expect("send lands");

            let record = fixture.transfer(&RECALL_TRANSFER_ID);
            assert!(record.assets_sent + record.realized_loss <= record.requested_recall_amount);
            check_identities(&fixture);
        }

        let record = fixture.transfer(&RECALL_TRANSFER_ID);
        assert_eq!(
            record.assets_sent + record.realized_loss,
            record.requested_recall_amount
        );
        assert_eq!(record.status, TransferStatus::Complete);
        assert_eq!(fixture.position().active_transfer_kind, TransferKind::None);
    }
}

#[test]
fn a_rejected_operation_leaves_every_number_where_it_was() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    let before = fixture.position();
    let custody_before = fixture.token_amount(fixture.custody);

    // No recall is open, so the record does not exist and none of these
    // may touch state.
    expect_rejection(fixture.withdraw(RECALL_TRANSFER_ID, AMOUNT));
    expect_rejection(fixture.send_recall(RECALL_TRANSFER_ID, AMOUNT));
    expect_error(
        fixture.attribute(ALLOCATE_TRANSFER_ID),
        RemoteLegError::InvalidTransferStatus,
    );

    let after = fixture.position();
    assert_eq!(after.attributed_principal, before.attributed_principal);
    assert_eq!(after.deployed_principal, before.deployed_principal);
    assert_eq!(after.recalled_custody, before.recalled_custody);
    assert_eq!(after.unattributed_custody, before.unattributed_custody);
    assert_eq!(after.active_transfer_kind, before.active_transfer_kind);
    assert_eq!(fixture.token_amount(fixture.custody), custody_before);
}

#[test]
fn the_same_trace_always_reaches_the_same_state() {
    let run = || {
        let mut fixture = Fixture::deployed();
        fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
        fixture.deploy(AMOUNT / 2).expect("deposit lands");
        fixture
            .configure_adapter(AMOUNT, 500, false)
            .expect("the adapter takes the test conditions");
        fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
        fixture
            .withdraw(RECALL_TRANSFER_ID, AMOUNT)
            .expect("withdrawal lands");
        fixture
            .send_recall(RECALL_TRANSFER_ID, AMOUNT)
            .expect("send lands");
        let position = fixture.position();
        let record = fixture.transfer(&RECALL_TRANSFER_ID);
        (
            position.attributed_principal,
            position.deployed_principal,
            position.recalled_custody,
            position.unattributed_custody,
            position.cumulative_realized_loss,
            record.assets_sent,
            record.realized_loss,
        )
    };
    assert_eq!(run(), run());
}

#[test]
fn a_recall_drawing_on_both_sources_resolves_exactly_once() {
    for seed in 0..12u64 {
        let mut random = Seeded(seed);
        let mut fixture = Fixture::deployed();
        fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);

        let deployed = random.upto(AMOUNT - 1);
        fixture.deploy(deployed).expect("deposit lands");
        let held = AMOUNT - fixture.position().deployed_principal;

        fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
        let record = fixture.transfer(&RECALL_TRANSFER_ID);
        assert_eq!(record.custody_principal_reserved, held);
        assert_eq!(fixture.position().recalled_custody, held);

        fixture
            .withdraw(RECALL_TRANSFER_ID, AMOUNT)
            .expect("withdrawal lands");
        fixture
            .send_recall(RECALL_TRANSFER_ID, AMOUNT)
            .expect("send lands");

        let record = fixture.transfer(&RECALL_TRANSFER_ID);
        assert_eq!(record.status, TransferStatus::Complete);
        assert_eq!(record.assets_sent, AMOUNT);
        assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);
        check_identities(&fixture);

        // A resolved cycle records itself once and frees the leg.
        let position = fixture.position();
        assert_eq!(position.latest_completed_transfer_id, RECALL_TRANSFER_ID);
        assert_eq!(position.active_transfer_kind, TransferKind::None);
    }
}

#[test]
fn the_leg_exposes_no_share_or_user_claim_instruction() {
    let source = include_str!("../src/lib.rs");
    let program = source
        .split_once("pub mod solevm_remote_leg {")
        .expect("the program module exists")
        .1;

    for line in program.lines().filter(|line| line.contains("pub fn ")) {
        let name = line
            .split("pub fn ")
            .nth(1)
            .and_then(|rest| rest.split(['(', '<']).next())
            .expect("a handler name");
        for banned in ["share", "claim", "mint", "user"] {
            assert!(!name.contains(banned), "{name} must not exist");
        }
    }
}
