//! Compute measurements for the allocation and recall path.
//!
//! Each case drives the live fixture to a real state, then replays the next
//! instruction under Mollusk. That keeps the measured accounts identical to
//! the ones the program sees in the adversarial suites.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::prelude::Clock;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_spl::token::spl_token;
use common::{ALLOCATE_TRANSFER_ID, Fixture, MAX_REMOTE_PRINCIPAL, Pubkey, RECALL_TRANSFER_ID};
use mollusk_svm::Mollusk;
use mollusk_svm::program::loader_keys;
use mollusk_svm::result::ProgramResult;
use solana_account::Account;
use solana_signer::Signer;
use solevm_remote_leg::MessageClass;

const STRATEGY_STATE_LIMIT: u64 = 45_000;
const RECONCILE_LIMIT: u64 = 15_000;
const ALLOCATE_LIMIT: u64 = 170_000;
const ATTRIBUTE_LIMIT: u64 = 25_000;
const DEPLOY_LIMIT: u64 = 60_000;
const RECALL_LIMIT: u64 = 170_000;
const WITHDRAW_LIMIT: u64 = 70_000;
const SEND_LIMIT: u64 = 32_000;
/// A rejection before any account work stays cheap.
const REJECTED_LIMIT: u64 = 20_000;
/// A rejection after decoding still pays for the canonical message.
const REJECTED_MESSAGE_LIMIT: u64 = 120_000;

const AMOUNT: u64 = 1_000_000;

struct Bench {
    fixture: Fixture,
    mollusk: Mollusk,
}

impl Bench {
    /// A leg with control state and an installed adapter, nothing more.
    fn installed() -> Self {
        let mut fixture = Fixture::ready();
        fixture.install_adapter();
        Self::around(fixture)
    }

    /// A leg whose strategy state is already live.
    fn deployed() -> Self {
        Self::around(Fixture::deployed())
    }

    fn around(fixture: Fixture) -> Self {
        let mut mollusk = Mollusk::default();
        mollusk.add_program_with_loader_and_elf(
            &solevm_remote_leg::ID,
            &loader_keys::LOADER_V3,
            &common::program_bytes(),
        );
        mollusk.add_program_with_loader_and_elf(
            &solevm_test_strategy::ID,
            &loader_keys::LOADER_V3,
            &common::adapter_bytes(),
        );
        mollusk_svm_programs_token::token::add_program(&mut mollusk);
        mollusk.sysvars.clock.unix_timestamp = fixture.svm.get_sysvar::<Clock>().unix_timestamp;
        Self { fixture, mollusk }
    }

    /// Copies every account an instruction touches out of the live fixture.
    fn snapshot(&self, instruction: &Instruction) -> Vec<(Pubkey, Account)> {
        instruction
            .accounts
            .iter()
            .map(|meta| {
                let account = if meta.pubkey == spl_token::ID {
                    mollusk_svm_programs_token::token::keyed_account().1
                } else {
                    self.fixture
                        .svm
                        .get_account(&meta.pubkey)
                        .unwrap_or_default()
                };
                (meta.pubkey, account)
            })
            .collect()
    }

    #[track_caller]
    fn measure(&self, label: &str, instruction: Instruction, expect_success: bool, limit: u64) {
        let accounts = self.snapshot(&instruction);
        let result = self.mollusk.process_instruction(&instruction, &accounts);
        assert_eq!(
            result.program_result == ProgramResult::Success,
            expect_success,
            "{label} produced {:?}",
            result.program_result
        );

        let used = result.compute_units_consumed;
        println!("{label}: {used} compute units");
        assert!(used <= limit, "{label} used {used} units, limit is {limit}");
    }
}

#[test]
fn initializing_the_strategy_state_stays_within_its_compute_limit() {
    let bench = Bench::installed();
    let instruction = bench
        .fixture
        .strategy_state_instruction(bench.fixture.strategy_accounts(), MAX_REMOTE_PRINCIPAL);
    bench.measure(
        "initialize_strategy_state",
        instruction,
        true,
        STRATEGY_STATE_LIMIT,
    );
}

#[test]
fn reconciling_custody_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench.fixture.credit(bench.fixture.custody, AMOUNT);
    let instruction = bench.fixture.reconcile_instruction(bench.fixture.custody);
    bench.measure("reconcile_custody", instruction, true, RECONCILE_LIMIT);
}

#[test]
fn accepting_an_allocation_stays_within_its_compute_limit() {
    let bench = Bench::deployed();
    let bytes = bench
        .fixture
        .allocate_bytes(ALLOCATE_TRANSFER_ID, AMOUNT, 1);
    let accounts = bench.fixture.allocate_accounts(ALLOCATE_TRANSFER_ID, 1);
    let instruction = bench.fixture.allocate_instruction(accounts, bytes);
    bench.measure("process_allocate", instruction, true, ALLOCATE_LIMIT);
}

#[test]
fn attributing_part_of_an_allocation_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench
        .fixture
        .accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);
    bench.fixture.credit(bench.fixture.custody, AMOUNT / 4);
    let record = bench.fixture.transfer_key(&ALLOCATE_TRANSFER_ID);
    let instruction = bench
        .fixture
        .attribute_instruction(record, bench.fixture.custody);
    bench.measure(
        "attribute_allocation partial",
        instruction,
        true,
        ATTRIBUTE_LIMIT,
    );
}

#[test]
fn attributing_a_full_allocation_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench
        .fixture
        .accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);
    bench.fixture.credit(bench.fixture.custody, AMOUNT);
    let record = bench.fixture.transfer_key(&ALLOCATE_TRANSFER_ID);
    let instruction = bench
        .fixture
        .attribute_instruction(record, bench.fixture.custody);
    bench.measure(
        "attribute_allocation full",
        instruction,
        true,
        ATTRIBUTE_LIMIT,
    );
}

#[test]
fn deploying_to_the_strategy_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench.fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    let instruction = bench
        .fixture
        .deploy_instruction(bench.fixture.strategy_accounts(), AMOUNT);
    bench.measure("deploy_to_strategy", instruction, true, DEPLOY_LIMIT);
}

#[test]
fn accepting_a_recall_against_custody_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench.fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    let bytes = bench.fixture.recall_bytes(RECALL_TRANSFER_ID, AMOUNT, 1);
    let accounts = bench.fixture.recall_accounts(RECALL_TRANSFER_ID, 1);
    let instruction = bench.fixture.recall_instruction(accounts, bytes);
    bench.measure(
        "process_recall from custody",
        instruction,
        true,
        RECALL_LIMIT,
    );
}

#[test]
fn accepting_a_recall_against_the_strategy_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench.fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    bench.fixture.deploy(AMOUNT).expect("deposit lands");
    let bytes = bench.fixture.recall_bytes(RECALL_TRANSFER_ID, AMOUNT, 1);
    let accounts = bench.fixture.recall_accounts(RECALL_TRANSFER_ID, 1);
    let instruction = bench.fixture.recall_instruction(accounts, bytes);
    bench.measure(
        "process_recall from strategy",
        instruction,
        true,
        RECALL_LIMIT,
    );
}

#[test]
fn a_partial_withdrawal_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench.fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    bench.fixture.deploy(AMOUNT).expect("deposit lands");
    bench
        .fixture
        .configure_adapter(AMOUNT / 4, 0, false)
        .expect("the adapter takes the test conditions");
    bench.fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    let record = bench.fixture.transfer_key(&RECALL_TRANSFER_ID);
    let instruction =
        bench
            .fixture
            .withdraw_instruction(bench.fixture.strategy_accounts(), record, AMOUNT);
    bench.measure(
        "withdraw_for_recall partial",
        instruction,
        true,
        WITHDRAW_LIMIT,
    );
}

#[test]
fn a_full_withdrawal_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench.fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    bench.fixture.deploy(AMOUNT).expect("deposit lands");
    bench.fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    let record = bench.fixture.transfer_key(&RECALL_TRANSFER_ID);
    let instruction =
        bench
            .fixture
            .withdraw_instruction(bench.fixture.strategy_accounts(), record, AMOUNT);
    bench.measure(
        "withdraw_for_recall full",
        instruction,
        true,
        WITHDRAW_LIMIT,
    );
}

#[test]
fn a_partial_send_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench.fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    bench.fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    let record = bench.fixture.transfer_key(&RECALL_TRANSFER_ID);
    let instruction =
        bench
            .fixture
            .send_recall_instruction(record, bench.fixture.escrow, AMOUNT / 4);
    bench.measure("send_recall partial", instruction, true, SEND_LIMIT);
}

#[test]
fn a_full_send_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench.fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    bench.fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    let record = bench.fixture.transfer_key(&RECALL_TRANSFER_ID);
    let instruction = bench
        .fixture
        .send_recall_instruction(record, bench.fixture.escrow, AMOUNT);
    bench.measure("send_recall full", instruction, true, SEND_LIMIT);
}

#[test]
fn a_rejected_overlapping_transfer_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench
        .fixture
        .accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);
    let second = [0x77; 32];
    let bytes = bench.fixture.allocate_bytes(second, AMOUNT, 2);
    let accounts = bench.fixture.allocate_accounts(second, 2);
    let instruction = bench.fixture.allocate_instruction(accounts, bytes);
    bench.measure(
        "rejected overlapping allocation",
        instruction,
        false,
        REJECTED_MESSAGE_LIMIT,
    );
}

#[test]
fn a_rejected_cpi_account_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    bench.fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    let mut accounts = bench.fixture.strategy_accounts();
    accounts.adapter_token_vault = bench.fixture.custody;
    let instruction = bench.fixture.deploy_instruction(accounts, AMOUNT);
    bench.measure("rejected deploy vault", instruction, false, REJECTED_LIMIT);
}

#[test]
fn a_watermark_blocked_by_a_transfer_stays_within_its_compute_limit() {
    let mut bench = Bench::deployed();
    let sequence = bench
        .fixture
        .accept_allocation_at(ALLOCATE_TRANSFER_ID, AMOUNT, Some(10));
    bench
        .fixture
        .set_lane_highest(MessageClass::Allocate, sequence + 8);

    let administrator = bench.fixture.administrator.pubkey();
    let lane = bench.fixture.lane_key(MessageClass::Allocate);
    let position = bench.fixture.position_key();
    let instruction = bench.fixture.watermark_instruction(
        administrator,
        lane,
        Some(position),
        MessageClass::Allocate,
        sequence + 1,
    );
    bench.measure(
        "blocked advance_replay_watermark",
        instruction,
        false,
        REJECTED_LIMIT,
    );
}
