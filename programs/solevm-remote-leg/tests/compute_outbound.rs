//! Compute measurements for the outbound reporting path.
//!
//! Each case drives the live fixture to a real state, then replays the next
//! instruction under Mollusk.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::prelude::Clock;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_spl::token::spl_token;
use common::{ALLOCATE_TRANSFER_ID, Fixture, Pubkey, RECALL_TRANSFER_ID};
use mollusk_svm::Mollusk;
use mollusk_svm::program::loader_keys;
use mollusk_svm::result::ProgramResult;
use solana_account::Account;
use solana_signer::Signer;

const OUTBOUND_STATE_LIMIT: u64 = 20_000;
const EMPTY_REPORT_LIMIT: u64 = 85_000;
const FULL_REPORT_LIMIT: u64 = 85_000;
const RECONCILED_REPORT_LIMIT: u64 = 85_000;
const FROZEN_REPORT_LIMIT: u64 = 85_000;
const CHAINED_REPORT_LIMIT: u64 = 85_000;
const RECALL_SENT_LIMIT: u64 = 70_000;
/// A repeat is refused before the message is built, so it stays cheap.
const REPEAT_LIMIT: u64 = 20_000;
/// The record address is checked after the bytes exist, as the order requires.
const REJECTED_RECORD_LIMIT: u64 = 75_000;

const EPOCH: u64 = 12;
const AMOUNT: u64 = 1_000_000;

struct Bench {
    fixture: Fixture,
    mollusk: Mollusk,
}

impl Bench {
    fn publishing() -> Self {
        Self::around(Fixture::ready_to_publish())
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
        mollusk.compute_budget.compute_unit_limit = 1_400_000;
        Self { fixture, mollusk }
    }

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

    fn report_instruction(&self, epoch_id: u64) -> Instruction {
        let sequence = self.fixture.outbound_lane().next_sequence;
        let accounts = self.fixture.outbound_accounts(sequence);
        self.fixture.publish_report_instruction(accounts, epoch_id)
    }
}

#[test]
fn initializing_the_outbound_state_stays_within_its_compute_limit() {
    let bench = Bench::around(Fixture::deployed());
    let administrator = bench.fixture.administrator.pubkey();
    let lane = bench.fixture.outbound_lane_key();

    let metas = anchor_lang::ToAccountMetas::to_account_metas(
        &solevm_remote_leg::accounts::InitializeOutboundState {
            administrator,
            remote_config: bench.fixture.config,
            outbound_lane: lane,
            system_program: anchor_lang::system_program::ID,
        },
        None,
    );
    let instruction = Instruction {
        program_id: solevm_remote_leg::ID,
        accounts: metas,
        data: anchor_lang::InstructionData::data(
            &solevm_remote_leg::instruction::InitializeOutboundState {},
        ),
    };

    bench.measure(
        "initialize outbound state",
        instruction,
        true,
        OUTBOUND_STATE_LIMIT,
    );
}

#[test]
fn publishing_an_empty_report_stays_within_its_compute_limit() {
    let bench = Bench::publishing();
    bench.measure(
        "publish empty report",
        bench.report_instruction(EPOCH),
        true,
        EMPTY_REPORT_LIMIT,
    );
}

#[test]
fn publishing_a_report_of_a_full_position_stays_within_its_compute_limit() {
    let mut bench = Bench::publishing();
    bench.fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    bench.fixture.deploy(AMOUNT).expect("the deposit lands");

    bench.measure(
        "publish full report",
        bench.report_instruction(EPOCH),
        true,
        FULL_REPORT_LIMIT,
    );
}

#[test]
fn publishing_a_report_after_a_surplus_arrives_stays_within_its_compute_limit() {
    let mut bench = Bench::publishing();
    bench.fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    bench.fixture.credit(bench.fixture.custody, 5_000);

    bench.measure(
        "publish report after reconciliation",
        bench.report_instruction(EPOCH),
        true,
        RECONCILED_REPORT_LIMIT,
    );
}

#[test]
fn publishing_a_report_while_frozen_stays_within_its_compute_limit() {
    let mut bench = Bench::publishing();
    bench.fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    let guardian = bench.fixture.guardian.insecure_clone();
    bench.fixture.freeze(&guardian).expect("the leg freezes");

    bench.measure(
        "publish report while frozen",
        bench.report_instruction(EPOCH),
        true,
        FROZEN_REPORT_LIMIT,
    );
}

#[test]
fn publishing_a_second_report_stays_within_its_compute_limit() {
    let mut bench = Bench::publishing();
    bench
        .fixture
        .publish_report(EPOCH)
        .expect("the first report lands");

    bench.measure(
        "publish chained report",
        bench.report_instruction(EPOCH + 1),
        true,
        CHAINED_REPORT_LIMIT,
    );
}

#[test]
fn publishing_a_recall_notice_stays_within_its_compute_limit() {
    let mut bench = Bench::publishing();
    complete_recall(&mut bench.fixture);

    let sequence = bench.fixture.outbound_lane().next_sequence;
    let accounts = bench
        .fixture
        .recall_sent_accounts(RECALL_TRANSFER_ID, sequence);
    let instruction = bench.fixture.publish_recall_sent_instruction(accounts);

    bench.measure(
        "publish recall notice",
        instruction,
        true,
        RECALL_SENT_LIMIT,
    );
}

#[test]
fn a_repeated_recall_notice_stays_within_its_compute_limit() {
    let mut bench = Bench::publishing();
    complete_recall(&mut bench.fixture);
    bench
        .fixture
        .publish_recall_sent(RECALL_TRANSFER_ID)
        .expect("the first notice lands");

    let sequence = bench.fixture.outbound_lane().next_sequence;
    let accounts = bench
        .fixture
        .recall_sent_accounts(RECALL_TRANSFER_ID, sequence);
    let instruction = bench.fixture.publish_recall_sent_instruction(accounts);

    bench.measure(
        "reject repeated recall notice",
        instruction,
        false,
        REPEAT_LIMIT,
    );
}

#[test]
fn rejecting_a_report_with_a_foreign_record_stays_within_its_compute_limit() {
    let mut bench = Bench::publishing();
    let record = bench.fixture.report_record_key(1);
    bench
        .fixture
        .give_account_to(record, Pubkey::new_unique(), 64);

    bench.measure(
        "reject invalid report record",
        bench.report_instruction(EPOCH),
        false,
        REJECTED_RECORD_LIMIT,
    );
}

/// Drives one recall all the way to a completed send.
fn complete_recall(fixture: &mut Fixture) {
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("the deposit lands");
    fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    fixture
        .withdraw(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the withdrawal lands");
    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the send lands");
}
