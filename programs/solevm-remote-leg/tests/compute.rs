//! Compute measurements with practical regression thresholds.
//!
//! The limits leave room for normal toolchain drift. They exist to catch a
//! large regression, not to pin an exact number.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_lang::{AnchorSerialize, Discriminator, InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use anchor_spl::token::spl_token::state::AccountState;
use common::{
    CONFIG_VERSION, CONTROL_LANE_ID, DEPLOYMENT_ID, DESTINATION_CHAIN_ID, LOCAL_APPLICATION_ID,
    Pubkey, REPORT_LANE_ID, SOURCE_APPLICATION_ID, SOURCE_CHAIN_ID, START_TIMESTAMP, VAULT_ID,
};
use mollusk_svm::Mollusk;
use mollusk_svm::program::{
    create_program_account_loader_v3, keyed_account_for_system_program, loader_keys,
};
use mollusk_svm::result::ProgramResult;
use solana_account::Account;
use solevm_remote_leg::state::{
    CUSTODY_AUTHORITY_SEED, REMOTE_CONFIG_RESERVED, REMOTE_CONFIG_SEED, STATE_VERSION,
};
use solevm_remote_leg::{
    CONSUMED_MESSAGE_SEED, ConsumedMessage, ControlStateParams, InitializeParams, MessageClass,
    REPLAY_LANE_SEED, RISK_CONFIG_RESERVED, RISK_CONFIG_SEED, RemoteConfig, ReplayLane, RiskConfig,
};

use common::messages::MessageBuilder;

const INITIALIZE_LIMIT: u64 = 40_000;
const FREEZE_LIMIT: u64 = 10_000;
const REJECTED_INITIALIZE_LIMIT: u64 = 25_000;
const CONTROL_STATE_LIMIT: u64 = 45_000;
const CONFIG_UPDATE_LIMIT: u64 = 140_000;
const REJECTED_UPDATE_LIMIT: u64 = 50_000;
const WATERMARK_LIMIT: u64 = 12_000;
const CLOSE_RECORD_LIMIT: u64 = 14_000;

const WATERMARK_LAG: u64 = 2;
const INITIAL_COMMITMENT: [u8; 32] = [0xAA; 32];

struct Bench {
    mollusk: Mollusk,
    administrator: Pubkey,
    guardian: Pubkey,
    verifier: Pubkey,
    mint: Pubkey,
    custody: Pubkey,
    escrow: Pubkey,
    config: Pubkey,
    config_bump: u8,
    custody_authority: Pubkey,
    custody_authority_bump: u8,
}

impl Bench {
    fn new() -> Self {
        let mut mollusk = Mollusk::default();
        mollusk.add_program_with_loader_and_elf(
            &solevm_remote_leg::ID,
            &loader_keys::LOADER_V3,
            &common::program_bytes(),
        );
        mollusk.sysvars.clock.unix_timestamp = START_TIMESTAMP;

        let (config, config_bump) = Pubkey::find_program_address(
            &[REMOTE_CONFIG_SEED, &DEPLOYMENT_ID, &VAULT_ID],
            &solevm_remote_leg::ID,
        );
        let (custody_authority, custody_authority_bump) = Pubkey::find_program_address(
            &[CUSTODY_AUTHORITY_SEED, config.as_ref()],
            &solevm_remote_leg::ID,
        );

        Self {
            mollusk,
            administrator: Pubkey::new_unique(),
            guardian: Pubkey::new_unique(),
            verifier: Pubkey::new_unique(),
            mint: Pubkey::new_unique(),
            custody: Pubkey::new_unique(),
            escrow: Pubkey::new_unique(),
            config,
            config_bump,
            custody_authority,
            custody_authority_bump,
        }
    }

    fn params(&self) -> InitializeParams {
        InitializeParams {
            deployment_id: DEPLOYMENT_ID,
            vault_id: VAULT_ID,
            source_chain_id: SOURCE_CHAIN_ID,
            destination_chain_id: DESTINATION_CHAIN_ID,
            source_application_id: SOURCE_APPLICATION_ID,
            local_application_id: LOCAL_APPLICATION_ID,
            control_lane_id: CONTROL_LANE_ID,
            report_lane_id: REPORT_LANE_ID,
            config_version: CONFIG_VERSION,
            transport_verifier: self.verifier,
            emergency_guardian: self.guardian,
        }
    }

    fn initialize(&self, params: InitializeParams) -> (Instruction, Vec<(Pubkey, Account)>) {
        let instruction = Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: solevm_remote_leg::accounts::InitializeRemoteLeg {
                administrator: self.administrator,
                remote_config: self.config,
                asset_mint: self.mint,
                custody_token_account: self.custody,
                outbound_escrow: self.escrow,
                token_program: spl_token::ID,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: solevm_remote_leg::instruction::InitializeRemoteLeg { params }.data(),
        };

        let accounts = vec![
            (self.administrator, funded_account()),
            (self.config, Account::default()),
            (self.mint, mint_account(6)),
            (
                self.custody,
                token_account(self.mint, self.custody_authority),
            ),
            (self.escrow, token_account(self.mint, Pubkey::new_unique())),
            (
                spl_token::ID,
                create_program_account_loader_v3(&spl_token::ID),
            ),
            keyed_account_for_system_program(),
        ];
        (instruction, accounts)
    }

    fn freeze(&self, authority: Pubkey) -> (Instruction, Vec<(Pubkey, Account)>) {
        let instruction = Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: solevm_remote_leg::accounts::FreezeRemoteLeg {
                authority,
                remote_config: self.config,
            }
            .to_account_metas(None),
            data: solevm_remote_leg::instruction::FreezeRemoteLeg {}.data(),
        };

        let accounts = vec![
            (authority, funded_account()),
            (self.config, self.config_account(CONFIG_VERSION, false)),
        ];
        (instruction, accounts)
    }

    fn config_account(&self, config_version: u64, frozen: bool) -> Account {
        let config = RemoteConfig {
            state_version: STATE_VERSION,
            bump: self.config_bump,
            custody_authority_bump: self.custody_authority_bump,
            frozen,
            administrator: self.administrator,
            emergency_guardian: self.guardian,
            transport_verifier: self.verifier,
            asset_mint: self.mint,
            token_program: spl_token::ID,
            custody_authority: self.custody_authority,
            custody_token_account: self.custody,
            outbound_escrow: self.escrow,
            source_chain_id: SOURCE_CHAIN_ID,
            destination_chain_id: DESTINATION_CHAIN_ID,
            source_application_id: SOURCE_APPLICATION_ID,
            local_application_id: LOCAL_APPLICATION_ID,
            deployment_id: DEPLOYMENT_ID,
            vault_id: VAULT_ID,
            control_lane_id: CONTROL_LANE_ID,
            report_lane_id: REPORT_LANE_ID,
            config_version,
            initialized_at: START_TIMESTAMP,
            reserved: [0u8; REMOTE_CONFIG_RESERVED],
        };
        owned_account(RemoteConfig::DISCRIMINATOR, &config, RemoteConfig::LEN)
    }

    // Control plane

    fn risk_address(&self) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[RISK_CONFIG_SEED, self.config.as_ref()],
            &solevm_remote_leg::ID,
        )
    }

    fn lane_address(&self, class: MessageClass) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                REPLAY_LANE_SEED,
                self.config.as_ref(),
                &[class.to_u8()],
                &CONTROL_LANE_ID.to_le_bytes(),
            ],
            &solevm_remote_leg::ID,
        )
    }

    fn record_address(&self, sequence: u64) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                CONSUMED_MESSAGE_SEED,
                self.config.as_ref(),
                &[MessageClass::ConfigUpdate.to_u8()],
                &CONTROL_LANE_ID.to_le_bytes(),
                &sequence.to_le_bytes(),
            ],
            &solevm_remote_leg::ID,
        )
    }

    fn control_state(&self) -> (Instruction, Vec<(Pubkey, Account)>) {
        let risk = self.risk_address().0;
        let allocate = self.lane_address(MessageClass::Allocate).0;
        let recall = self.lane_address(MessageClass::Recall).0;
        let config_update = self.lane_address(MessageClass::ConfigUpdate).0;

        let instruction = Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: solevm_remote_leg::accounts::InitializeControlState {
                administrator: self.administrator,
                remote_config: self.config,
                risk_config: risk,
                allocate_lane: allocate,
                recall_lane: recall,
                config_update_lane: config_update,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: solevm_remote_leg::instruction::InitializeControlState {
                params: ControlStateParams {
                    max_remote_allocation_bps: 6_000,
                    max_upward_deviation_bps: 200,
                    max_downward_deviation_bps: 1_000,
                    max_report_age: 3_600,
                    config_commitment: INITIAL_COMMITMENT,
                    mandatory_watermark_lag: WATERMARK_LAG,
                },
            }
            .data(),
        };

        let accounts = vec![
            (self.administrator, funded_account()),
            (self.config, self.config_account(CONFIG_VERSION, false)),
            (risk, Account::default()),
            (allocate, Account::default()),
            (recall, Account::default()),
            (config_update, Account::default()),
            keyed_account_for_system_program(),
        ];
        (instruction, accounts)
    }

    /// One config update, with the lane already at the given state.
    fn update(&self, state: LaneState) -> (Instruction, Vec<(Pubkey, Account)>) {
        let risk = self.risk_address().0;
        let lane = self.lane_address(MessageClass::ConfigUpdate).0;
        let record = self.record_address(state.sequence).0;

        let instruction = Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: solevm_remote_leg::accounts::ProcessConfigUpdate {
                transport_verifier: self.verifier,
                remote_config: self.config,
                risk_config: risk,
                config_update_lane: lane,
                consumed_message: record,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: solevm_remote_leg::instruction::ProcessConfigUpdate {
                message_bytes: state.message.clone(),
            }
            .data(),
        };

        let record_account = if state.prefunded {
            funded_account()
        } else {
            Account::default()
        };

        let accounts = vec![
            (self.verifier, funded_account()),
            (
                self.config,
                self.config_account(state.config_version, false),
            ),
            (risk, self.risk_account(state.config_version)),
            (
                lane,
                self.lane_account(
                    MessageClass::ConfigUpdate,
                    state.minimum_sequence,
                    state.highest_sequence,
                    state.commitment,
                ),
            ),
            (record, record_account),
            keyed_account_for_system_program(),
        ];
        (instruction, accounts)
    }

    fn watermark(&self, new_minimum: u64) -> (Instruction, Vec<(Pubkey, Account)>) {
        let lane = self.lane_address(MessageClass::ConfigUpdate).0;
        let instruction = Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: solevm_remote_leg::accounts::AdvanceReplayWatermark {
                administrator: self.administrator,
                remote_config: self.config,
                replay_lane: lane,
                remote_position: None,
            }
            .to_account_metas(None),
            data: solevm_remote_leg::instruction::AdvanceReplayWatermark {
                message_class: MessageClass::ConfigUpdate,
                new_minimum_sequence: new_minimum,
            }
            .data(),
        };

        let accounts = vec![
            (self.administrator, funded_account()),
            (self.config, self.config_account(CONFIG_VERSION, false)),
            (
                lane,
                self.lane_account(MessageClass::ConfigUpdate, 1, 6, [0x31; 32]),
            ),
        ];
        (instruction, accounts)
    }

    fn close(&self, sequence: u64) -> (Instruction, Vec<(Pubkey, Account)>) {
        let lane = self.lane_address(MessageClass::ConfigUpdate).0;
        let (record, bump) = self.record_address(sequence);

        let instruction = Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: solevm_remote_leg::accounts::CloseConsumedMessage {
                remote_config: self.config,
                administrator: self.administrator,
                replay_lane: lane,
                consumed_message: record,
            }
            .to_account_metas(None),
            data: solevm_remote_leg::instruction::CloseConsumedMessage {}.data(),
        };

        let accounts = vec![
            (self.config, self.config_account(CONFIG_VERSION, false)),
            (self.administrator, funded_account()),
            (
                lane,
                self.lane_account(MessageClass::ConfigUpdate, 4, 6, [0x31; 32]),
            ),
            (record, self.record_account(sequence, bump)),
        ];
        (instruction, accounts)
    }

    fn risk_account(&self, config_version: u64) -> Account {
        let risk = RiskConfig {
            state_version: STATE_VERSION,
            bump: self.risk_address().1,
            max_remote_allocation_bps: 6_000,
            max_upward_deviation_bps: 200,
            max_downward_deviation_bps: 1_000,
            max_report_age: 3_600,
            config_version,
            config_commitment: INITIAL_COMMITMENT,
            initialized_at: START_TIMESTAMP,
            last_update_at: START_TIMESTAMP,
            reserved: [0u8; RISK_CONFIG_RESERVED],
        };
        owned_account(RiskConfig::DISCRIMINATOR, &risk, RiskConfig::LEN)
    }

    fn lane_account(
        &self,
        class: MessageClass,
        minimum: u64,
        highest: u64,
        commitment: [u8; 32],
    ) -> Account {
        let lane = ReplayLane {
            state_version: STATE_VERSION,
            bump: self.lane_address(class).1,
            message_class: class,
            lane_id: CONTROL_LANE_ID,
            minimum_acceptable_sequence: minimum,
            highest_consumed_sequence: highest,
            message_commitment: commitment,
            mandatory_watermark_lag: WATERMARK_LAG,
            last_accepted_at: START_TIMESTAMP,
        };
        owned_account(ReplayLane::DISCRIMINATOR, &lane, ReplayLane::LEN)
    }

    fn record_account(&self, sequence: u64, bump: u8) -> Account {
        let record = ConsumedMessage {
            state_version: STATE_VERSION,
            bump,
            message_class: MessageClass::ConfigUpdate,
            lane_id: CONTROL_LANE_ID,
            sequence,
            message_id: [0x42; 32],
        };
        owned_account(
            ConsumedMessage::DISCRIMINATOR,
            &record,
            ConsumedMessage::LEN,
        )
    }

    #[track_caller]
    fn measure(
        &self,
        label: &str,
        instruction: Instruction,
        accounts: &[(Pubkey, Account)],
        expect_success: bool,
        limit: u64,
    ) -> u64 {
        let result = self.mollusk.process_instruction(&instruction, accounts);
        assert_eq!(
            result.program_result == ProgramResult::Success,
            expect_success,
            "{label} produced {:?}",
            result.program_result
        );

        let used = result.compute_units_consumed;
        println!("{label}: {used} compute units");
        assert!(used <= limit, "{label} used {used} units, limit is {limit}");
        used
    }
}

/// The lane and message state one measured update starts from.
struct LaneState {
    message: Vec<u8>,
    sequence: u64,
    config_version: u64,
    minimum_sequence: u64,
    highest_sequence: u64,
    commitment: [u8; 32],
    prefunded: bool,
}

impl LaneState {
    /// A fresh lane taking its first update.
    fn first() -> Self {
        Self {
            message: MessageBuilder::config_update().encode(),
            sequence: 1,
            config_version: CONFIG_VERSION,
            minimum_sequence: 1,
            highest_sequence: 0,
            commitment: [0u8; 32],
            prefunded: false,
        }
    }

    /// A lane that already holds four updates and takes the fifth.
    fn later() -> Self {
        let commitment = [0x31; 32];
        let message = MessageBuilder::config_update()
            .sequence(5)
            .previous_commitment(commitment)
            .config_body(|body| {
                body.previous_config_version = protocol_types::ConfigVersion::new(5);
                body.config_version = protocol_types::ConfigVersion::new(6);
            })
            .encode();

        Self {
            message,
            sequence: 5,
            config_version: 5,
            minimum_sequence: 1,
            highest_sequence: 4,
            commitment,
            prefunded: false,
        }
    }

    fn prefunded(mut self) -> Self {
        self.prefunded = true;
        self
    }
}

/// Serializes one account into a program owned buffer of its exact width.
fn owned_account<T: AnchorSerialize>(discriminator: &[u8], value: &T, width: usize) -> Account {
    let mut data = discriminator.to_vec();
    value.serialize(&mut data).expect("account encodes");
    data.resize(width, 0);

    Account {
        lamports: 10_000_000,
        data,
        owner: solevm_remote_leg::ID,
        executable: false,
        rent_epoch: 0,
    }
}

fn funded_account() -> Account {
    Account {
        lamports: 1_000_000_000,
        data: Vec::new(),
        owner: anchor_lang::system_program::ID,
        executable: false,
        rent_epoch: 0,
    }
}

fn mint_account(decimals: u8) -> Account {
    let mint = spl_token::state::Mint {
        mint_authority: None.into(),
        supply: 0,
        decimals,
        is_initialized: true,
        freeze_authority: None.into(),
    };
    let mut data = vec![0u8; spl_token::state::Mint::LEN];
    mint.pack_into_slice(&mut data);
    Account {
        lamports: 10_000_000,
        data,
        owner: spl_token::ID,
        executable: false,
        rent_epoch: 0,
    }
}

fn token_account(mint: Pubkey, owner: Pubkey) -> Account {
    let account = spl_token::state::Account {
        mint,
        owner,
        amount: 0,
        delegate: None.into(),
        state: AccountState::Initialized,
        is_native: None.into(),
        delegated_amount: 0,
        close_authority: None.into(),
    };
    let mut data = vec![0u8; spl_token::state::Account::LEN];
    account.pack_into_slice(&mut data);
    Account {
        lamports: 10_000_000,
        data,
        owner: spl_token::ID,
        executable: false,
        rent_epoch: 0,
    }
}

#[test]
fn setting_up_the_remote_leg_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let (instruction, accounts) = bench.initialize(bench.params());
    bench.measure(
        "initialize_remote_leg",
        instruction,
        &accounts,
        true,
        INITIALIZE_LIMIT,
    );
}

#[test]
fn freezing_by_the_administrator_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let (instruction, accounts) = bench.freeze(bench.administrator);
    bench.measure(
        "freeze_remote_leg by administrator",
        instruction,
        &accounts,
        true,
        FREEZE_LIMIT,
    );
}

#[test]
fn freezing_by_the_guardian_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let (instruction, accounts) = bench.freeze(bench.guardian);
    bench.measure(
        "freeze_remote_leg by guardian",
        instruction,
        &accounts,
        true,
        FREEZE_LIMIT,
    );
}

#[test]
fn a_rejected_setup_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let mut params = bench.params();
    params.config_version = 0;
    let (instruction, accounts) = bench.initialize(params);
    bench.measure(
        "rejected initialize_remote_leg",
        instruction,
        &accounts,
        false,
        REJECTED_INITIALIZE_LIMIT,
    );
}

#[test]
fn a_rejected_freeze_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let (instruction, accounts) = bench.freeze(Pubkey::new_unique());
    bench.measure(
        "rejected freeze_remote_leg",
        instruction,
        &accounts,
        false,
        FREEZE_LIMIT,
    );
}

#[test]
fn setting_up_the_control_state_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let (instruction, accounts) = bench.control_state();
    bench.measure(
        "initialize_control_state",
        instruction,
        &accounts,
        true,
        CONTROL_STATE_LIMIT,
    );
}

#[test]
fn the_first_config_update_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let (instruction, accounts) = bench.update(LaneState::first());
    bench.measure(
        "process_config_update first",
        instruction,
        &accounts,
        true,
        CONFIG_UPDATE_LIMIT,
    );
}

#[test]
fn a_later_config_update_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let (instruction, accounts) = bench.update(LaneState::later());
    bench.measure(
        "process_config_update later",
        instruction,
        &accounts,
        true,
        CONFIG_UPDATE_LIMIT,
    );
}

#[test]
fn a_config_update_onto_a_prefunded_record_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let (instruction, accounts) = bench.update(LaneState::first().prefunded());
    bench.measure(
        "process_config_update prefunded record",
        instruction,
        &accounts,
        true,
        CONFIG_UPDATE_LIMIT,
    );
}

#[test]
fn a_rejected_replay_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let mut state = LaneState::first();
    // The lane already consumed this sequence, so the message is a replay.
    state.highest_sequence = 1;
    let (instruction, accounts) = bench.update(state);
    bench.measure(
        "process_config_update rejected replay",
        instruction,
        &accounts,
        false,
        REJECTED_UPDATE_LIMIT,
    );
}

#[test]
fn a_rejected_sequence_below_the_watermark_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let mut state = LaneState::first();
    state.minimum_sequence = 4;
    state.highest_sequence = 6;
    let (instruction, accounts) = bench.update(state);
    bench.measure(
        "process_config_update below watermark",
        instruction,
        &accounts,
        false,
        REJECTED_UPDATE_LIMIT,
    );
}

#[test]
fn advancing_the_watermark_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let (instruction, accounts) = bench.watermark(4);
    bench.measure(
        "advance_replay_watermark",
        instruction,
        &accounts,
        true,
        WATERMARK_LIMIT,
    );
}

#[test]
fn closing_a_consumed_record_stays_within_its_compute_limit() {
    let bench = Bench::new();
    let (instruction, accounts) = bench.close(2);
    bench.measure(
        "close_consumed_message",
        instruction,
        &accounts,
        true,
        CLOSE_RECORD_LIMIT,
    );
}

#[test]
fn a_rejected_setup_costs_less_than_a_successful_one() {
    let bench = Bench::new();
    let (accepted, accepted_accounts) = bench.initialize(bench.params());
    let accepted_units = bench.measure(
        "initialize_remote_leg",
        accepted,
        &accepted_accounts,
        true,
        INITIALIZE_LIMIT,
    );

    let mut params = bench.params();
    params.config_version = 0;
    let (rejected, rejected_accounts) = bench.initialize(params);
    let rejected_units = bench.measure(
        "rejected initialize_remote_leg",
        rejected,
        &rejected_accounts,
        false,
        REJECTED_INITIALIZE_LIMIT,
    );

    assert!(rejected_units < accepted_units);
}
