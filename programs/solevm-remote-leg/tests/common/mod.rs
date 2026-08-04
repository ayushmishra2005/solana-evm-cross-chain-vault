//! Shared setup for the instruction tests.

#![allow(dead_code, unreachable_pub)]
#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use anchor_spl::token::spl_token::state::AccountState;
use litesvm::LiteSVM;
use litesvm::types::FailedTransactionMetadata;
use solana_account::Account;
use solana_instruction::error::InstructionError;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

use solevm_remote_leg::state::{CUSTODY_AUTHORITY_SEED, REMOTE_CONFIG_SEED};
use solevm_remote_leg::{InitializeParams, RemoteConfig, RemoteLegError};

pub const DEPLOYMENT_ID: [u8; 32] = [0x11; 32];
pub const VAULT_ID: [u8; 32] = [0x22; 32];
pub const SOURCE_APPLICATION_ID: [u8; 32] = [0x33; 32];
pub const LOCAL_APPLICATION_ID: [u8; 32] = [0x44; 32];
pub const SOURCE_CHAIN_ID: u32 = 8453;
pub const DESTINATION_CHAIN_ID: u32 = 900;
pub const CONTROL_LANE_ID: u32 = 1;
pub const REPORT_LANE_ID: u32 = 2;
pub const CONFIG_VERSION: u64 = 1;
pub const MINT_DECIMALS: u8 = 6;
pub const START_TIMESTAMP: i64 = 1_700_000_000;

/// The failure metadata is large, so it travels boxed.
pub type TxResult = Result<(), Box<FailedTransactionMetadata>>;

/// Anchor shifts program errors above its own reserved range.
pub const ANCHOR_ERROR_OFFSET: u32 = 6000;

/// Every account the initialize instruction reads.
#[derive(Clone, Copy, Debug)]
pub struct InitAccounts {
    pub administrator: Pubkey,
    pub remote_config: Pubkey,
    pub asset_mint: Pubkey,
    pub custody_token_account: Pubkey,
    pub outbound_escrow: Pubkey,
    pub token_program: Pubkey,
    pub system_program: Pubkey,
}

pub use anchor_lang::prelude::Pubkey;

/// A funded vault with a mint, a custody account and an escrow account.
pub struct Fixture {
    pub svm: LiteSVM,
    pub administrator: Keypair,
    pub guardian: Keypair,
    pub outsider: Keypair,
    pub verifier: Keypair,
    pub escrow_owner: Pubkey,
    pub mint: Pubkey,
    pub custody: Pubkey,
    pub escrow: Pubkey,
    pub config: Pubkey,
    pub custody_authority: Pubkey,
    pub custody_authority_bump: u8,
    pub params: InitializeParams,
}

impl Fixture {
    pub fn new() -> Self {
        Self::with_mint_decimals(MINT_DECIMALS)
    }

    pub fn with_mint_decimals(decimals: u8) -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program(solevm_remote_leg::ID, &program_bytes())
            .expect("program loads");

        let mut clock = svm.get_sysvar::<anchor_lang::solana_program::clock::Clock>();
        clock.unix_timestamp = START_TIMESTAMP;
        svm.set_sysvar(&clock);

        let administrator = Keypair::new();
        let guardian = Keypair::new();
        let outsider = Keypair::new();
        for signer in [&administrator, &guardian, &outsider] {
            svm.airdrop(&signer.pubkey(), 10_000_000_000).unwrap();
        }

        let (config, _) = Pubkey::find_program_address(
            &[REMOTE_CONFIG_SEED, &DEPLOYMENT_ID, &VAULT_ID],
            &solevm_remote_leg::ID,
        );
        let (custody_authority, custody_authority_bump) = Pubkey::find_program_address(
            &[CUSTODY_AUTHORITY_SEED, config.as_ref()],
            &solevm_remote_leg::ID,
        );

        let mint = Pubkey::new_unique();
        let custody = Pubkey::new_unique();
        let escrow = Pubkey::new_unique();
        let escrow_owner = Pubkey::new_unique();

        let mut fixture = Self {
            svm,
            administrator,
            guardian,
            outsider,
            verifier: Keypair::new(),
            escrow_owner,
            mint,
            custody,
            escrow,
            config,
            custody_authority,
            custody_authority_bump,
            params: InitializeParams {
                deployment_id: DEPLOYMENT_ID,
                vault_id: VAULT_ID,
                source_chain_id: SOURCE_CHAIN_ID,
                destination_chain_id: DESTINATION_CHAIN_ID,
                source_application_id: SOURCE_APPLICATION_ID,
                local_application_id: LOCAL_APPLICATION_ID,
                control_lane_id: CONTROL_LANE_ID,
                report_lane_id: REPORT_LANE_ID,
                config_version: CONFIG_VERSION,
                transport_verifier: Pubkey::new_unique(),
                emergency_guardian: Pubkey::default(),
            },
        };
        fixture.params.emergency_guardian = fixture.guardian.pubkey();
        fixture.params.transport_verifier = fixture.verifier.pubkey();

        fixture.write_mint(mint, decimals);
        fixture.write_token_account(custody, mint, custody_authority, None, None);
        fixture.write_token_account(escrow, mint, escrow_owner, None, None);
        fixture
    }

    /// Stores a mint directly so tests can choose its decimals.
    pub fn write_mint(&mut self, key: Pubkey, decimals: u8) {
        let mint = spl_token::state::Mint {
            mint_authority: None.into(),
            supply: 0,
            decimals,
            is_initialized: true,
            freeze_authority: None.into(),
        };
        let mut data = vec![0u8; spl_token::state::Mint::LEN];
        mint.pack_into_slice(&mut data);
        self.write_owned_account(key, data, spl_token::ID);
    }

    /// Stores a token account so tests can choose every field it validates.
    pub fn write_token_account(
        &mut self,
        key: Pubkey,
        mint: Pubkey,
        owner: Pubkey,
        delegate: Option<Pubkey>,
        close_authority: Option<Pubkey>,
    ) {
        let account = spl_token::state::Account {
            mint,
            owner,
            amount: 0,
            delegate: delegate.into(),
            state: AccountState::Initialized,
            is_native: None.into(),
            delegated_amount: if delegate.is_some() { 1 } else { 0 },
            close_authority: close_authority.into(),
        };
        let mut data = vec![0u8; spl_token::state::Account::LEN];
        account.pack_into_slice(&mut data);
        self.write_owned_account(key, data, spl_token::ID);
    }

    pub fn write_owned_account(&mut self, key: Pubkey, data: Vec<u8>, owner: Pubkey) {
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                key,
                Account {
                    lamports,
                    data,
                    owner,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    pub fn administrator_key(&self) -> Pubkey {
        self.administrator.pubkey()
    }

    pub fn guardian_key(&self) -> Pubkey {
        self.guardian.pubkey()
    }

    pub fn verifier_key(&self) -> Pubkey {
        self.verifier.pubkey()
    }

    pub fn verifier_keypair(&self) -> Keypair {
        self.verifier.insecure_clone()
    }

    pub fn fund(&mut self, signer: &Keypair) {
        self.svm.airdrop(&signer.pubkey(), 10_000_000_000).unwrap();
    }

    /// Sets up one more vault of the same deployment and returns its address.
    pub fn add_second_vault(&mut self, vault_id: [u8; 32]) -> Pubkey {
        let config = Self::config_address(&DEPLOYMENT_ID, &vault_id);
        let custody_authority = Self::custody_authority_address(&config).0;

        let custody = Pubkey::new_unique();
        let escrow = Pubkey::new_unique();
        let mint = self.mint;
        self.write_token_account(custody, mint, custody_authority, None, None);
        self.write_token_account(escrow, mint, Pubkey::new_unique(), None, None);

        let mut params = self.params.clone();
        params.vault_id = vault_id;
        let mut accounts = self.default_accounts();
        accounts.remote_config = config;
        accounts.custody_token_account = custody;
        accounts.outbound_escrow = escrow;

        self.initialize_with(accounts, params)
            .expect("second vault initializes");
        config
    }

    pub fn config_at(&self, key: Pubkey) -> RemoteConfig {
        let account = self.svm.get_account(&key).expect("config exists");
        RemoteConfig::try_deserialize(&mut account.data.as_slice()).expect("config decodes")
    }

    pub fn config_address(deployment_id: &[u8; 32], vault_id: &[u8; 32]) -> Pubkey {
        Pubkey::find_program_address(
            &[REMOTE_CONFIG_SEED, deployment_id, vault_id],
            &solevm_remote_leg::ID,
        )
        .0
    }

    pub fn custody_authority_address(config: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[CUSTODY_AUTHORITY_SEED, config.as_ref()],
            &solevm_remote_leg::ID,
        )
    }

    pub fn default_accounts(&self) -> InitAccounts {
        InitAccounts {
            administrator: self.administrator.pubkey(),
            remote_config: self.config,
            asset_mint: self.mint,
            custody_token_account: self.custody,
            outbound_escrow: self.escrow,
            token_program: spl_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
    }

    pub fn initialize(&mut self) -> TxResult {
        let accounts = self.default_accounts();
        let params = self.params.clone();
        self.initialize_with(accounts, params)
    }

    pub fn initialize_with_params(&mut self, params: InitializeParams) -> TxResult {
        let accounts = self.default_accounts();
        self.initialize_with(accounts, params)
    }

    pub fn initialize_with_accounts(&mut self, accounts: InitAccounts) -> TxResult {
        let params = self.params.clone();
        self.initialize_with(accounts, params)
    }

    pub fn initialize_with(
        &mut self,
        accounts: InitAccounts,
        params: InitializeParams,
    ) -> TxResult {
        let instruction = self.initialize_instruction(accounts, params);
        self.send(instruction, &[&self.administrator.insecure_clone()])
    }

    pub fn initialize_instruction(
        &self,
        accounts: InitAccounts,
        params: InitializeParams,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::InitializeRemoteLeg {
            administrator: accounts.administrator,
            remote_config: accounts.remote_config,
            asset_mint: accounts.asset_mint,
            custody_token_account: accounts.custody_token_account,
            outbound_escrow: accounts.outbound_escrow,
            token_program: accounts.token_program,
            system_program: accounts.system_program,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::InitializeRemoteLeg { params }.data(),
        }
    }

    pub fn freeze(&mut self, authority: &Keypair) -> TxResult {
        self.freeze_with(authority, self.config)
    }

    pub fn freeze_with(&mut self, authority: &Keypair, remote_config: Pubkey) -> TxResult {
        let instruction = self.freeze_instruction(authority.pubkey(), remote_config);
        self.send(instruction, &[&authority.insecure_clone()])
    }

    pub fn freeze_instruction(&self, authority: Pubkey, remote_config: Pubkey) -> Instruction {
        let metas = solevm_remote_leg::accounts::FreezeRemoteLeg {
            authority,
            remote_config,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::FreezeRemoteLeg {}.data(),
        }
    }

    pub fn send(&mut self, instruction: Instruction, signers: &[&Keypair]) -> TxResult {
        let payer = signers.first().map(|signer| signer.insecure_clone());
        let payer = payer.expect("at least one signer");
        self.send_as(instruction, &payer, signers)
    }

    pub fn send_as(
        &mut self,
        instruction: Instruction,
        payer: &Keypair,
        signers: &[&Keypair],
    ) -> TxResult {
        // A fresh blockhash keeps a repeated call from looking like a replay.
        self.svm.expire_blockhash();
        let transaction = Transaction::new_signed_with_payer(
            &[instruction],
            Some(&payer.pubkey()),
            signers,
            self.svm.latest_blockhash(),
        );
        self.svm
            .send_transaction(transaction)
            .map(|_| ())
            .map_err(Box::new)
    }

    /// Rewrites the raw bytes of the configuration account.
    pub fn overwrite_config_data(&mut self, mutate: impl FnOnce(&mut Vec<u8>)) {
        let mut account = self.svm.get_account(&self.config).expect("config exists");
        mutate(&mut account.data);
        self.svm.set_account(self.config, account).unwrap();
    }

    /// Moves the configuration account to another owning program.
    pub fn reassign_config(&mut self, owner: Pubkey) {
        self.reassign_account(self.config, owner);
    }

    /// Moves any account to another owning program.
    pub fn reassign_account(&mut self, key: Pubkey, owner: Pubkey) {
        let mut account = self.svm.get_account(&key).expect("account exists");
        account.owner = owner;
        self.svm.set_account(key, account).unwrap();
    }

    pub fn config(&self) -> RemoteConfig {
        let account = self.svm.get_account(&self.config).expect("config exists");
        RemoteConfig::try_deserialize(&mut account.data.as_slice()).expect("config decodes")
    }

    pub fn token_amount(&self, key: Pubkey) -> u64 {
        let account = self.svm.get_account(&key).expect("token account exists");
        spl_token::state::Account::unpack(&account.data)
            .expect("token account decodes")
            .amount
    }
}

/// Reads the program built by cargo build-sbf.
pub fn program_bytes() -> Vec<u8> {
    let directory = std::env::var("SBF_OUT_DIR")
        .unwrap_or_else(|_| format!("{}/target/deploy", env!("CARGO_MANIFEST_DIR")));
    let path = format!("{directory}/solevm_remote_leg.so");
    std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "missing {path}, build it first with \
             cargo-build-sbf --tools-version v1.54"
        )
    })
}

/// Numeric code Anchor reports for one program error.
pub fn code_of(error: RemoteLegError) -> u32 {
    error as u32 + ANCHOR_ERROR_OFFSET
}

/// Fails unless the transaction was rejected with the expected program error.
#[track_caller]
pub fn expect_error(result: TxResult, expected: RemoteLegError) {
    expect_custom_code(result, code_of(expected));
}

/// Fails unless the transaction was rejected with the expected Anchor error.
#[track_caller]
pub fn expect_anchor_error(result: TxResult, expected: anchor_lang::error::ErrorCode) {
    expect_custom_code(result, expected as u32);
}

#[track_caller]
pub fn expect_custom_code(result: TxResult, expected: u32) {
    let failure = result.expect_err("transaction should have failed");
    let actual = custom_code(&failure);
    assert_eq!(
        actual,
        Some(expected),
        "expected custom error {expected}, got {:?}",
        failure.err
    );
}

/// Fails unless the transaction was rejected before the program ran.
#[track_caller]
pub fn expect_rejection(result: TxResult) {
    assert!(result.is_err(), "transaction should have failed");
}

fn custom_code(failure: &FailedTransactionMetadata) -> Option<u32> {
    match &failure.err {
        TransactionError::InstructionError(_, InstructionError::Custom(code)) => Some(*code),
        _ => None,
    }
}
