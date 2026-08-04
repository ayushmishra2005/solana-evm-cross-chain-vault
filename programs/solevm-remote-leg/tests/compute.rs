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
use solevm_remote_leg::{InitializeParams, RemoteConfig};

const INITIALIZE_LIMIT: u64 = 40_000;
const FREEZE_LIMIT: u64 = 10_000;
const REJECTED_INITIALIZE_LIMIT: u64 = 25_000;

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
            (self.config, self.config_account()),
        ];
        (instruction, accounts)
    }

    fn config_account(&self) -> Account {
        let config = RemoteConfig {
            state_version: STATE_VERSION,
            bump: self.config_bump,
            custody_authority_bump: self.custody_authority_bump,
            frozen: false,
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
            config_version: CONFIG_VERSION,
            initialized_at: START_TIMESTAMP,
            reserved: [0u8; REMOTE_CONFIG_RESERVED],
        };

        let mut data = RemoteConfig::DISCRIMINATOR.to_vec();
        config.serialize(&mut data).expect("config encodes");
        data.resize(RemoteConfig::LEN, 0);

        Account {
            lamports: 10_000_000,
            data,
            owner: solevm_remote_leg::ID,
            executable: false,
            rent_epoch: 0,
        }
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
