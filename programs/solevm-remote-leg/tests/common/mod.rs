//! Shared setup for the instruction tests.

#![allow(dead_code, unreachable_pub)]
#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

use anchor_lang::solana_program::bpf_loader_upgradeable;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use anchor_spl::token::spl_token::state::AccountState;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use litesvm::LiteSVM;
use litesvm::types::FailedTransactionMetadata;
use solana_account::Account;
use solana_instruction::error::InstructionError;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

use solevm_remote_leg::state::{CUSTODY_AUTHORITY_SEED, REMOTE_CONFIG_SEED};
use solevm_remote_leg::{
    CONSUMED_MESSAGE_SEED, ConsumedMessage, ControlStateParams, InitializeParams,
    MessageQuarantined, OutboundLane, QuarantineReason, REMOTE_POSITION_SEED, REPLAY_LANE_SEED,
    RISK_CONFIG_SEED, RecallSentMarker, RemoteConfig, RemoteLegError, RemotePosition, ReplayLane,
    ReportRecord, RiskConfig, STRATEGY_CONFIG_SEED, StrategyConfig, TRANSFER_RECORD_SEED,
    TransferKind, TransferRecord, TransferStatus,
};
use solevm_test_strategy::{ADAPTER_AUTHORITY_SEED, ADAPTER_STATE_SEED};

pub mod messages;

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
pub const MAX_REMOTE_ALLOCATION_BPS: u16 = 6_000;
pub const MAX_UPWARD_DEVIATION_BPS: u16 = 200;
pub const MAX_DOWNWARD_DEVIATION_BPS: u16 = 1_000;
pub const MAX_REPORT_AGE: u64 = 3_600;
pub const INITIAL_CONFIG_COMMITMENT: [u8; 32] = [0xAA; 32];
pub const WATERMARK_LAG: u64 = 2;
pub const MAX_REMOTE_PRINCIPAL: u64 = 10_000_000;
pub const ALLOCATE_TRANSFER_ID: [u8; 32] = [0x77; 32];
pub const RECALL_TRANSFER_ID: [u8; 32] = [0x88; 32];
/// keccak256("destination-ref") — bound to the EVM expected destination.
pub const E2E_DESTINATION_REFERENCE: [u8; 32] = [
    0xf6, 0xdd, 0x08, 0x4d, 0xd7, 0x3a, 0x0a, 0x71, 0xa5, 0x82, 0x55, 0x05, 0x94, 0x92, 0x1f, 0x2c,
    0x54, 0x72, 0x40, 0x04, 0x20, 0x60, 0x9c, 0xe6, 0x0e, 0x1a, 0x45, 0xcd, 0x58, 0x48, 0x64, 0x15,
];

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
    pub upgrade_authority: Pubkey,
    pub program_data: Pubkey,
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
    pub adapter_state: Pubkey,
    pub adapter_authority: Pubkey,
    pub adapter_vault: Pubkey,
    pub params: InitializeParams,
    pub last_logs: Vec<String>,
}

impl Fixture {
    pub fn new() -> Self {
        Self::with_mint_decimals(MINT_DECIMALS)
    }

    pub fn with_mint_decimals(decimals: u8) -> Self {
        Self::construct(decimals, false)
    }

    /// A fully initialized leg with fixed keys and the EVM destination reference.
    pub fn e2e() -> Self {
        let mut fixture = Self::construct(MINT_DECIMALS, true);
        fixture.initialize().expect("leg initializes");
        fixture
            .initialize_control_state()
            .expect("control state initializes");
        fixture.install_adapter();
        fixture
            .initialize_strategy_state(MAX_REMOTE_PRINCIPAL)
            .expect("strategy state initializes");
        fixture
            .initialize_outbound_state()
            .expect("outbound state initializes");
        fixture
    }

    fn construct(decimals: u8, deterministic: bool) -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program(solevm_remote_leg::ID, &program_bytes())
            .expect("program loads");

        let mut clock = svm.get_sysvar::<anchor_lang::solana_program::clock::Clock>();
        clock.unix_timestamp = START_TIMESTAMP;
        svm.set_sysvar(&clock);

        let administrator = if deterministic {
            seeded_keypair(1)
        } else {
            Keypair::new()
        };
        let guardian = if deterministic {
            seeded_keypair(2)
        } else {
            Keypair::new()
        };
        let outsider = if deterministic {
            seeded_keypair(3)
        } else {
            Keypair::new()
        };
        let verifier = if deterministic {
            seeded_keypair(4)
        } else {
            Keypair::new()
        };
        for signer in [&administrator, &guardian, &outsider, &verifier] {
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

        let mint = if deterministic {
            Pubkey::new_from_array([0xA1; 32])
        } else {
            Pubkey::new_unique()
        };
        let custody = if deterministic {
            Pubkey::new_from_array([0xA2; 32])
        } else {
            Pubkey::new_unique()
        };
        let escrow = if deterministic {
            Pubkey::new_from_array(E2E_DESTINATION_REFERENCE)
        } else {
            Pubkey::new_unique()
        };
        let escrow_owner = if deterministic {
            Pubkey::new_from_array([0xA4; 32])
        } else {
            Pubkey::new_unique()
        };
        let adapter_vault = if deterministic {
            Pubkey::new_from_array([0xA5; 32])
        } else {
            Pubkey::new_unique()
        };

        let mut fixture = Self {
            svm,
            administrator,
            guardian,
            outsider,
            verifier,
            escrow_owner,
            mint,
            custody,
            escrow,
            config,
            custody_authority,
            custody_authority_bump,
            adapter_state: Pubkey::default(),
            adapter_authority: Pubkey::default(),
            adapter_vault,
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
            last_logs: Vec::new(),
        };
        fixture.params.emergency_guardian = fixture.guardian.pubkey();
        fixture.params.transport_verifier = fixture.verifier.pubkey();

        fixture.write_mint(mint, decimals);
        fixture.write_token_account(custody, mint, custody_authority, None, None);
        fixture.write_token_account(escrow, mint, escrow_owner, None, None);
        let administrator = fixture.administrator.pubkey();
        fixture.write_program_data(Some(administrator));
        fixture
    }

    /// Where the upgradeable loader keeps the leg's upgrade authority.
    pub fn program_data_address() -> Pubkey {
        Pubkey::find_program_address(
            &[solevm_remote_leg::ID.as_ref()],
            &bpf_loader_upgradeable::ID,
        )
        .0
    }

    /// Records the leg as an upgradeable program with this upgrade authority.
    pub fn write_program_data(&mut self, upgrade_authority: Option<Pubkey>) {
        self.write_owned_account(
            Self::program_data_address(),
            program_data(upgrade_authority),
            bpf_loader_upgradeable::ID,
        );
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
            upgrade_authority: self.administrator.pubkey(),
            program_data: Self::program_data_address(),
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
        let upgrade_authority = accounts.upgrade_authority;
        let instruction = self.initialize_instruction(accounts, params);
        let administrator = self.administrator.insecure_clone();
        if upgrade_authority == administrator.pubkey() {
            return self.send(instruction, &[&administrator]);
        }
        let authority = self.keypair_for(upgrade_authority);
        self.send(instruction, &[&administrator, &authority])
    }

    /// The fixture key behind `key`, so a test may cast any of them in a role.
    fn keypair_for(&self, key: Pubkey) -> Keypair {
        [
            &self.administrator,
            &self.guardian,
            &self.outsider,
            &self.verifier,
        ]
        .into_iter()
        .find(|keypair| keypair.pubkey() == key)
        .expect("a fixture key")
        .insecure_clone()
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
            upgrade_authority: accounts.upgrade_authority,
            program_data: accounts.program_data,
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
        let result = self.svm.send_transaction(transaction);
        self.last_logs = match &result {
            Ok(meta) => meta.logs.clone(),
            Err(failure) => failure.meta.logs.clone(),
        };
        result.map(|_| ()).map_err(Box::new)
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

    // Control plane

    /// Initializes the leg and its control state, ready for messages.
    pub fn ready() -> Self {
        let mut fixture = Self::new();
        fixture.initialize().expect("leg initializes");
        fixture
            .initialize_control_state()
            .expect("control state initializes");
        fixture
    }

    pub fn control_params() -> ControlStateParams {
        ControlStateParams {
            max_remote_allocation_bps: MAX_REMOTE_ALLOCATION_BPS,
            max_upward_deviation_bps: MAX_UPWARD_DEVIATION_BPS,
            max_downward_deviation_bps: MAX_DOWNWARD_DEVIATION_BPS,
            max_report_age: MAX_REPORT_AGE,
            config_commitment: INITIAL_CONFIG_COMMITMENT,
            mandatory_watermark_lag: WATERMARK_LAG,
        }
    }

    pub fn risk_config_address(config: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(&[RISK_CONFIG_SEED, config.as_ref()], &solevm_remote_leg::ID).0
    }

    pub fn lane_address(config: &Pubkey, lane_id: u32) -> Pubkey {
        Pubkey::find_program_address(
            &[REPLAY_LANE_SEED, config.as_ref(), &lane_id.to_le_bytes()],
            &solevm_remote_leg::ID,
        )
        .0
    }

    pub fn record_address(config: &Pubkey, lane_id: u32, sequence: u64) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                CONSUMED_MESSAGE_SEED,
                config.as_ref(),
                &lane_id.to_le_bytes(),
                &sequence.to_le_bytes(),
            ],
            &solevm_remote_leg::ID,
        )
    }

    pub fn risk(&self) -> Pubkey {
        Self::risk_config_address(&self.config)
    }

    pub fn control_lane_key(&self) -> Pubkey {
        Self::lane_address(&self.config, CONTROL_LANE_ID)
    }

    pub fn record_key(&self, sequence: u64) -> Pubkey {
        Self::record_address(&self.config, CONTROL_LANE_ID, sequence).0
    }

    pub fn control_accounts(&self) -> ControlAccounts {
        ControlAccounts {
            administrator: self.administrator.pubkey(),
            remote_config: self.config,
            risk_config: self.risk(),
            control_lane: self.control_lane_key(),
            system_program: anchor_lang::system_program::ID,
        }
    }

    pub fn initialize_control_state(&mut self) -> TxResult {
        let accounts = self.control_accounts();
        self.initialize_control_state_with(accounts, Self::control_params())
    }

    pub fn initialize_control_state_params(&mut self, params: ControlStateParams) -> TxResult {
        let accounts = self.control_accounts();
        self.initialize_control_state_with(accounts, params)
    }

    pub fn initialize_control_state_with(
        &mut self,
        accounts: ControlAccounts,
        params: ControlStateParams,
    ) -> TxResult {
        let instruction = self.control_state_instruction(accounts, params);
        let signer = if accounts.administrator == self.administrator.pubkey() {
            self.administrator.insecure_clone()
        } else {
            self.outsider.insecure_clone()
        };
        self.send(instruction, &[&signer])
    }

    pub fn control_state_instruction(
        &self,
        accounts: ControlAccounts,
        params: ControlStateParams,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::InitializeControlState {
            administrator: accounts.administrator,
            remote_config: accounts.remote_config,
            risk_config: accounts.risk_config,
            control_lane: accounts.control_lane,
            system_program: accounts.system_program,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::InitializeControlState { params }.data(),
        }
    }

    pub fn update_accounts(&self, sequence: u64) -> UpdateAccounts {
        UpdateAccounts {
            transport_verifier: self.verifier.pubkey(),
            remote_config: self.config,
            risk_config: self.risk(),
            control_lane: self.control_lane_key(),
            consumed_message: self.record_key(sequence),
            system_program: anchor_lang::system_program::ID,
        }
    }

    /// Sends one canonical config update signed by the transport verifier.
    pub fn config_update(&mut self, sequence: u64, message_bytes: Vec<u8>) -> TxResult {
        let accounts = self.update_accounts(sequence);
        self.config_update_with(accounts, message_bytes, &self.verifier.insecure_clone())
    }

    pub fn config_update_with(
        &mut self,
        accounts: UpdateAccounts,
        message_bytes: Vec<u8>,
        verifier: &Keypair,
    ) -> TxResult {
        let instruction = self.config_update_instruction(accounts, message_bytes);
        let payer = self.administrator.insecure_clone();
        self.send_as(instruction, &payer, &[&payer, &verifier.insecure_clone()])
    }

    pub fn config_update_instruction(
        &self,
        accounts: UpdateAccounts,
        message_bytes: Vec<u8>,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::ProcessConfigUpdate {
            transport_verifier: accounts.transport_verifier,
            remote_config: accounts.remote_config,
            risk_config: accounts.risk_config,
            control_lane: accounts.control_lane,
            consumed_message: accounts.consumed_message,
            system_program: accounts.system_program,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::ProcessConfigUpdate { message_bytes }.data(),
        }
    }

    pub fn advance_watermark(
        &mut self,
        authority: &Keypair,
        new_minimum_sequence: u64,
    ) -> TxResult {
        let instruction = self.watermark_instruction(
            authority.pubkey(),
            self.control_lane_key(),
            self.position_key(),
            new_minimum_sequence,
        );
        self.send(instruction, &[&authority.insecure_clone()])
    }

    pub fn watermark_instruction(
        &self,
        administrator: Pubkey,
        replay_lane: Pubkey,
        remote_position: Pubkey,
        new_minimum_sequence: u64,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::AdvanceReplayWatermark {
            administrator,
            remote_config: self.config,
            replay_lane,
            remote_position,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::AdvanceReplayWatermark {
                new_minimum_sequence,
            }
            .data(),
        }
    }

    pub fn close_record(&mut self, sequence: u64) -> TxResult {
        let administrator = self.administrator.pubkey();
        self.close_record_to(sequence, administrator)
    }

    pub fn close_record_to(&mut self, sequence: u64, rent_destination: Pubkey) -> TxResult {
        let instruction = self.close_record_instruction(
            rent_destination,
            self.control_lane_key(),
            self.record_key(sequence),
        );
        let payer = self.outsider.insecure_clone();
        self.send_as(instruction, &payer, &[&payer])
    }

    pub fn close_record_instruction(
        &self,
        administrator: Pubkey,
        replay_lane: Pubkey,
        consumed_message: Pubkey,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::CloseConsumedMessage {
            remote_config: self.config,
            administrator,
            replay_lane,
            consumed_message,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::CloseConsumedMessage {}.data(),
        }
    }

    pub fn risk_config(&self) -> RiskConfig {
        let account = self.svm.get_account(&self.risk()).expect("risk exists");
        RiskConfig::try_deserialize(&mut account.data.as_slice()).expect("risk decodes")
    }

    pub fn control_lane(&self) -> ReplayLane {
        let account = self
            .svm
            .get_account(&self.control_lane_key())
            .expect("lane exists");
        ReplayLane::try_deserialize(&mut account.data.as_slice()).expect("lane decodes")
    }

    pub fn record(&self, sequence: u64) -> ConsumedMessage {
        let account = self
            .svm
            .get_account(&self.record_key(sequence))
            .expect("record exists");
        ConsumedMessage::try_deserialize(&mut account.data.as_slice()).expect("record decodes")
    }

    /// The consumed record written for one control-lane sequence.
    pub fn asset_record(&self, sequence: u64) -> ConsumedMessage {
        self.record(sequence)
    }

    /// True when a record account holds program owned data.
    pub fn record_exists(&self, sequence: u64) -> bool {
        self.svm
            .get_account(&self.record_key(sequence))
            .is_some_and(|account| {
                account.owner == solevm_remote_leg::ID && !account.data.is_empty()
            })
    }

    pub fn lamports(&self, key: Pubkey) -> u64 {
        self.svm
            .get_account(&key)
            .map_or(0, |account| account.lamports)
    }

    pub fn raw_data(&self, key: Pubkey) -> Vec<u8> {
        self.svm
            .get_account(&key)
            .map_or_else(Vec::new, |account| account.data)
    }

    /// Raises the highest consumed sequence the control lane records.
    ///
    /// The normal flow keeps an open transfer at the top of the lane, so this
    /// is the only way to reach the guard that protects it on its own.
    pub fn set_lane_highest(&mut self, sequence: u64) {
        let key = self.control_lane_key();
        let mut account = self.svm.get_account(&key).expect("lane exists");
        let offset = 8 + ReplayLane::HIGHEST_CONSUMED_OFFSET;
        account.data[offset..offset + 8].copy_from_slice(&sequence.to_le_bytes());
        self.svm.set_account(key, account).unwrap();
    }

    /// Moves the chain clock so timestamp rules can be exercised.
    pub fn set_time(&mut self, unix_timestamp: i64) {
        let mut clock = self
            .svm
            .get_sysvar::<anchor_lang::solana_program::clock::Clock>();
        clock.unix_timestamp = unix_timestamp;
        self.svm.set_sysvar(&clock);
    }

    /// Leaves lamports at an address that still holds no data.
    pub fn prefund(&mut self, key: Pubkey, lamports: u64) {
        self.svm
            .set_account(
                key,
                Account {
                    lamports,
                    data: Vec::new(),
                    owner: anchor_lang::system_program::ID,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    /// Rent a bare system account needs before it may hold lamports.
    pub fn empty_account_rent(&self) -> u64 {
        self.svm.minimum_balance_for_rent_exemption(0)
    }

    /// Rent the consumed record needs once it is allocated.
    pub fn record_rent(&self) -> u64 {
        self.svm
            .minimum_balance_for_rent_exemption(ConsumedMessage::LEN)
    }

    /// Builds and sends the next valid update, returning its message id.
    pub fn accept_next_update(&mut self) -> [u8; 32] {
        let lane = self.control_lane();
        let sequence = lane.highest_consumed_sequence + 1;
        let version = self.config().config_version;

        let (bytes, id) = messages::MessageBuilder::config_update()
            .sequence(sequence)
            .previous_commitment(lane.message_commitment)
            .config_body(|body| {
                body.previous_config_version = protocol_types::ConfigVersion::new(version);
                body.config_version = protocol_types::ConfigVersion::new(version + 1);
            })
            .encode_with_id();

        self.config_update(sequence, bytes).expect("update lands");
        id
    }

    /// Accepts a run of valid updates so the watermark can move.
    pub fn accept_updates(&mut self, count: u64) {
        for _ in 0..count {
            self.accept_next_update();
        }
    }

    // Strategy plane

    /// A leg with control state, a live adapter and strategy state.
    pub fn deployed() -> Self {
        let mut fixture = Self::ready();
        fixture.install_adapter();
        fixture
            .initialize_strategy_state(MAX_REMOTE_PRINCIPAL)
            .expect("strategy state initializes");
        fixture
    }

    pub fn strategy_config_address(config: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[STRATEGY_CONFIG_SEED, config.as_ref()],
            &solevm_remote_leg::ID,
        )
        .0
    }

    pub fn position_address(config: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[REMOTE_POSITION_SEED, config.as_ref()],
            &solevm_remote_leg::ID,
        )
        .0
    }

    pub fn transfer_address(config: &Pubkey, transfer_id: &[u8; 32]) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[TRANSFER_RECORD_SEED, config.as_ref(), transfer_id],
            &solevm_remote_leg::ID,
        )
    }

    pub fn strategy(&self) -> Pubkey {
        Self::strategy_config_address(&self.config)
    }

    pub fn position_key(&self) -> Pubkey {
        Self::position_address(&self.config)
    }

    pub fn transfer_key(&self, transfer_id: &[u8; 32]) -> Pubkey {
        Self::transfer_address(&self.config, transfer_id).0
    }

    pub fn asset_record_key(&self, sequence: u64) -> Pubkey {
        self.record_key(sequence)
    }

    /// Deploys the adapter program and initializes its state.
    pub fn install_adapter(&mut self) {
        self.svm
            .add_program(solevm_test_strategy::ID, &adapter_bytes())
            .expect("adapter loads");

        let (adapter_state, _) = Pubkey::find_program_address(
            &[ADAPTER_STATE_SEED, self.config.as_ref()],
            &solevm_test_strategy::ID,
        );
        let (adapter_authority, _) = Pubkey::find_program_address(
            &[ADAPTER_AUTHORITY_SEED, adapter_state.as_ref()],
            &solevm_test_strategy::ID,
        );
        self.adapter_state = adapter_state;
        self.adapter_authority = adapter_authority;
        self.write_token_account(self.adapter_vault, self.mint, adapter_authority, None, None);

        let metas = solevm_test_strategy::accounts::InitializeAdapter {
            payer: self.administrator.pubkey(),
            adapter_state,
            mint: self.mint,
            adapter_token_vault: self.adapter_vault,
            token_program: spl_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);

        let instruction = Instruction {
            program_id: solevm_test_strategy::ID,
            accounts: metas,
            data: solevm_test_strategy::instruction::InitializeAdapter {
                params: solevm_test_strategy::AdapterParams {
                    remote_leg_program: solevm_remote_leg::ID,
                    remote_config: self.config,
                    test_authority: self.administrator.pubkey(),
                    max_liquid_principal: u64::MAX,
                    loss_bps: 0,
                },
            }
            .data(),
        };
        let payer = self.administrator.insecure_clone();
        self.send_as(instruction, &payer, &[&payer])
            .expect("adapter initializes");
    }

    /// Shapes the adapter so one unwind path can be reproduced.
    pub fn configure_adapter(
        &mut self,
        max_liquid_principal: u64,
        loss_bps: u16,
        deposits_paused: bool,
    ) -> TxResult {
        let metas = solevm_test_strategy::accounts::ConfigureAdapter {
            test_authority: self.administrator.pubkey(),
            adapter_state: self.adapter_state,
        }
        .to_account_metas(None);

        let instruction = Instruction {
            program_id: solevm_test_strategy::ID,
            accounts: metas,
            data: solevm_test_strategy::instruction::ConfigureAdapterTestConditions {
                conditions: solevm_test_strategy::TestConditions {
                    max_liquid_principal,
                    loss_bps,
                    deposits_paused,
                },
            }
            .data(),
        };
        let payer = self.administrator.insecure_clone();
        self.send_as(instruction, &payer, &[&payer])
    }

    pub fn adapter(&self) -> solevm_test_strategy::AdapterState {
        let account = self
            .svm
            .get_account(&self.adapter_state)
            .expect("adapter exists");
        solevm_test_strategy::AdapterState::try_deserialize(&mut account.data.as_slice())
            .expect("adapter decodes")
    }

    pub fn strategy_accounts(&self) -> StrategyAccounts {
        StrategyAccounts {
            administrator: self.administrator.pubkey(),
            remote_config: self.config,
            risk_config: self.risk(),
            strategy_config: self.strategy(),
            remote_position: self.position_key(),
            adapter_program: solevm_test_strategy::ID,
            adapter_state: self.adapter_state,
            adapter_authority: self.adapter_authority,
            adapter_token_vault: self.adapter_vault,
            asset_mint: self.mint,
            token_program: spl_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
    }

    pub fn initialize_strategy_state(&mut self, max_remote_principal: u64) -> TxResult {
        let accounts = self.strategy_accounts();
        self.initialize_strategy_state_with(accounts, max_remote_principal)
    }

    pub fn initialize_strategy_state_with(
        &mut self,
        accounts: StrategyAccounts,
        max_remote_principal: u64,
    ) -> TxResult {
        let instruction = self.strategy_state_instruction(accounts, max_remote_principal);
        let signer = if accounts.administrator == self.administrator.pubkey() {
            self.administrator.insecure_clone()
        } else {
            self.outsider.insecure_clone()
        };
        self.send(instruction, &[&signer])
    }

    pub fn strategy_state_instruction(
        &self,
        accounts: StrategyAccounts,
        max_remote_principal: u64,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::InitializeStrategyState {
            administrator: accounts.administrator,
            remote_config: accounts.remote_config,
            risk_config: accounts.risk_config,
            strategy_config: accounts.strategy_config,
            remote_position: accounts.remote_position,
            adapter_program: accounts.adapter_program,
            adapter_state: accounts.adapter_state,
            adapter_authority: accounts.adapter_authority,
            adapter_token_vault: accounts.adapter_token_vault,
            asset_mint: accounts.asset_mint,
            token_program: accounts.token_program,
            system_program: accounts.system_program,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::InitializeStrategyState {
                max_remote_principal,
            }
            .data(),
        }
    }

    pub fn reconcile(&mut self) -> TxResult {
        let instruction = self.reconcile_instruction(self.custody);
        let payer = self.outsider.insecure_clone();
        self.send_as(instruction, &payer, &[&payer])
    }

    pub fn reconcile_instruction(&self, custody_token_account: Pubkey) -> Instruction {
        let metas = solevm_remote_leg::accounts::ReconcileCustody {
            remote_config: self.config,
            remote_position: self.position_key(),
            custody_token_account,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::ReconcileCustody {}.data(),
        }
    }

    pub fn allocate_accounts(&self, transfer_id: [u8; 32], sequence: u64) -> AssetAccounts {
        AssetAccounts {
            transport_verifier: self.verifier.pubkey(),
            remote_config: self.config,
            risk_config: self.risk(),
            strategy_config: self.strategy(),
            remote_position: self.position_key(),
            replay_lane: self.control_lane_key(),
            transfer_record: self.transfer_key(&transfer_id),
            consumed_message: self.asset_record_key(sequence),
            system_program: anchor_lang::system_program::ID,
        }
    }

    pub fn recall_accounts(&self, transfer_id: [u8; 32], sequence: u64) -> AssetAccounts {
        AssetAccounts {
            replay_lane: self.control_lane_key(),
            consumed_message: self.asset_record_key(sequence),
            ..self.allocate_accounts(transfer_id, sequence)
        }
    }

    pub fn allocate(&mut self, transfer_id: [u8; 32], sequence: u64, bytes: Vec<u8>) -> TxResult {
        let accounts = self.allocate_accounts(transfer_id, sequence);
        self.allocate_with(accounts, bytes)
    }

    pub fn allocate_with(&mut self, accounts: AssetAccounts, bytes: Vec<u8>) -> TxResult {
        let instruction = self.allocate_instruction(accounts, bytes);
        self.send_signed_by_verifier(instruction, accounts.transport_verifier)
    }

    pub fn allocate_instruction(&self, accounts: AssetAccounts, bytes: Vec<u8>) -> Instruction {
        let metas = solevm_remote_leg::accounts::ProcessAllocate {
            transport_verifier: accounts.transport_verifier,
            remote_config: accounts.remote_config,
            risk_config: accounts.risk_config,
            strategy_config: accounts.strategy_config,
            remote_position: accounts.remote_position,
            control_lane: accounts.replay_lane,
            transfer_record: accounts.transfer_record,
            consumed_message: accounts.consumed_message,
            system_program: accounts.system_program,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::ProcessAllocate {
                message_bytes: bytes,
            }
            .data(),
        }
    }

    pub fn recall(&mut self, transfer_id: [u8; 32], sequence: u64, bytes: Vec<u8>) -> TxResult {
        let accounts = self.recall_accounts(transfer_id, sequence);
        self.recall_with(accounts, bytes)
    }

    pub fn recall_with(&mut self, accounts: AssetAccounts, bytes: Vec<u8>) -> TxResult {
        let instruction = self.recall_instruction(accounts, bytes);
        self.send_signed_by_verifier(instruction, accounts.transport_verifier)
    }

    /// Signs with whichever key the accounts name as the transport verifier.
    fn send_signed_by_verifier(&mut self, instruction: Instruction, verifier: Pubkey) -> TxResult {
        let payer = self.administrator.insecure_clone();
        if verifier == payer.pubkey() {
            return self.send_as(instruction, &payer, &[&payer]);
        }
        let signer = if verifier == self.verifier.pubkey() {
            self.verifier.insecure_clone()
        } else {
            self.outsider.insecure_clone()
        };
        self.send_as(instruction, &payer, &[&payer, &signer])
    }

    /// Delivers a recall that `cosigner` co-signs in the guardian's place.
    pub fn recall_cosigned(
        &mut self,
        transfer_id: [u8; 32],
        sequence: u64,
        bytes: Vec<u8>,
        cosigner: &Keypair,
    ) -> TxResult {
        let accounts = self.recall_accounts(transfer_id, sequence);
        let instruction =
            self.recall_instruction_cosigned(accounts, bytes, Some(cosigner.pubkey()));
        let payer = self.administrator.insecure_clone();
        let verifier = self.verifier.insecure_clone();
        self.send_cosigned(instruction, &payer, &[&verifier, cosigner])
    }

    /// Sends with each distinct signer once, whatever roles they share.
    fn send_cosigned(
        &mut self,
        instruction: Instruction,
        payer: &Keypair,
        others: &[&Keypair],
    ) -> TxResult {
        let mut signers = vec![payer];
        for signer in others {
            if signers
                .iter()
                .all(|known| known.pubkey() != signer.pubkey())
            {
                signers.push(signer);
            }
        }
        self.send_as(instruction, payer, &signers)
    }

    pub fn recall_instruction(&self, accounts: AssetAccounts, bytes: Vec<u8>) -> Instruction {
        self.recall_instruction_cosigned(accounts, bytes, None)
    }

    pub fn recall_instruction_cosigned(
        &self,
        accounts: AssetAccounts,
        bytes: Vec<u8>,
        emergency_guardian: Option<Pubkey>,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::ProcessRecall {
            transport_verifier: accounts.transport_verifier,
            remote_config: accounts.remote_config,
            risk_config: accounts.risk_config,
            remote_position: accounts.remote_position,
            control_lane: accounts.replay_lane,
            transfer_record: accounts.transfer_record,
            consumed_message: accounts.consumed_message,
            system_program: accounts.system_program,
            emergency_guardian,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::ProcessRecall {
                message_bytes: bytes,
            }
            .data(),
        }
    }

    pub fn attribute(&mut self, transfer_id: [u8; 32]) -> TxResult {
        let instruction = self.attribute_instruction(self.transfer_key(&transfer_id), self.custody);
        let payer = self.outsider.insecure_clone();
        self.send_as(instruction, &payer, &[&payer])
    }

    /// Attributes with `cosigner` co-signing in the guardian's place.
    pub fn attribute_cosigned(&mut self, transfer_id: [u8; 32], cosigner: &Keypair) -> TxResult {
        let instruction = self.attribute_instruction_cosigned(
            self.transfer_key(&transfer_id),
            self.custody,
            Some(cosigner.pubkey()),
        );
        let payer = self.outsider.insecure_clone();
        self.send_cosigned(instruction, &payer, &[cosigner])
    }

    pub fn attribute_instruction(
        &self,
        transfer_record: Pubkey,
        custody_token_account: Pubkey,
    ) -> Instruction {
        self.attribute_instruction_cosigned(transfer_record, custody_token_account, None)
    }

    pub fn attribute_instruction_cosigned(
        &self,
        transfer_record: Pubkey,
        custody_token_account: Pubkey,
        emergency_guardian: Option<Pubkey>,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::AttributeAllocation {
            remote_config: self.config,
            remote_position: self.position_key(),
            transfer_record,
            custody_token_account,
            emergency_guardian,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::AttributeAllocation {}.data(),
        }
    }

    pub fn deploy(&mut self, maximum_amount: u64) -> TxResult {
        let accounts = self.strategy_accounts();
        self.deploy_with(accounts, maximum_amount)
    }

    pub fn deploy_with(&mut self, accounts: StrategyAccounts, maximum_amount: u64) -> TxResult {
        let instruction = self.deploy_instruction(accounts, maximum_amount);
        let payer = self.outsider.insecure_clone();
        self.send_as(instruction, &payer, &[&payer])
    }

    pub fn deploy_instruction(
        &self,
        accounts: StrategyAccounts,
        maximum_amount: u64,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::DeployToStrategy {
            remote_config: accounts.remote_config,
            strategy_config: accounts.strategy_config,
            remote_position: accounts.remote_position,
            custody_authority: self.custody_authority,
            custody_token_account: self.custody,
            adapter_caller: self.adapter_caller(),
            adapter_program: accounts.adapter_program,
            adapter_state: accounts.adapter_state,
            adapter_authority: accounts.adapter_authority,
            adapter_token_vault: accounts.adapter_token_vault,
            asset_mint: accounts.asset_mint,
            token_program: accounts.token_program,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::DeployToStrategy { maximum_amount }.data(),
        }
    }

    /// The identity the leg signs adapter calls with.
    pub fn adapter_caller(&self) -> Pubkey {
        solevm_remote_leg::RemoteConfig::adapter_caller(&self.config).0
    }

    /// Swaps the adapter program for its `hostile` build, keeping its state.
    pub fn install_hostile_adapter(&mut self) {
        self.svm
            .add_program(solevm_test_strategy::ID, &hostile_adapter_bytes())
            .expect("hostile adapter loads");
    }

    pub fn withdraw(&mut self, transfer_id: [u8; 32], maximum_principal: u64) -> TxResult {
        let accounts = self.strategy_accounts();
        let record = self.transfer_key(&transfer_id);
        let instruction = self.withdraw_instruction(accounts, record, maximum_principal);
        let payer = self.outsider.insecure_clone();
        self.send_as(instruction, &payer, &[&payer])
    }

    pub fn withdraw_instruction(
        &self,
        accounts: StrategyAccounts,
        transfer_record: Pubkey,
        maximum_principal: u64,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::WithdrawForRecall {
            remote_config: accounts.remote_config,
            strategy_config: accounts.strategy_config,
            remote_position: accounts.remote_position,
            transfer_record,
            custody_authority: self.custody_authority,
            custody_token_account: self.custody,
            adapter_caller: self.adapter_caller(),
            adapter_program: accounts.adapter_program,
            adapter_state: accounts.adapter_state,
            adapter_authority: accounts.adapter_authority,
            adapter_token_vault: accounts.adapter_token_vault,
            asset_mint: accounts.asset_mint,
            token_program: accounts.token_program,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::WithdrawForRecall { maximum_principal }.data(),
        }
    }

    pub fn send_recall(&mut self, transfer_id: [u8; 32], maximum_amount: u64) -> TxResult {
        let record = self.transfer_key(&transfer_id);
        let instruction = self.send_recall_instruction(record, self.escrow, maximum_amount);
        let payer = self.outsider.insecure_clone();
        self.send_as(instruction, &payer, &[&payer])
    }

    pub fn send_recall_instruction(
        &self,
        transfer_record: Pubkey,
        outbound_escrow: Pubkey,
        maximum_amount: u64,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::SendRecall {
            remote_config: self.config,
            remote_position: self.position_key(),
            transfer_record,
            custody_authority: self.custody_authority,
            custody_token_account: self.custody,
            outbound_escrow,
            token_program: spl_token::ID,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::SendRecall { maximum_amount }.data(),
        }
    }

    pub fn strategy_config(&self) -> StrategyConfig {
        let account = self
            .svm
            .get_account(&self.strategy())
            .expect("strategy exists");
        StrategyConfig::try_deserialize(&mut account.data.as_slice()).expect("strategy decodes")
    }

    pub fn position(&self) -> RemotePosition {
        let account = self
            .svm
            .get_account(&self.position_key())
            .expect("position exists");
        RemotePosition::try_deserialize(&mut account.data.as_slice()).expect("position decodes")
    }

    pub fn transfer(&self, transfer_id: &[u8; 32]) -> TransferRecord {
        let account = self
            .svm
            .get_account(&self.transfer_key(transfer_id))
            .expect("transfer exists");
        TransferRecord::try_deserialize(&mut account.data.as_slice()).expect("transfer decodes")
    }

    pub fn transfer_exists(&self, transfer_id: &[u8; 32]) -> bool {
        self.svm
            .get_account(&self.transfer_key(transfer_id))
            .is_some_and(|account| {
                account.owner == solevm_remote_leg::ID && !account.data.is_empty()
            })
    }

    /// Simulates assets arriving in a token account from the source chain.
    pub fn credit(&mut self, key: Pubkey, amount: u64) {
        let mut account = self.svm.get_account(&key).expect("token account exists");
        let mut token = spl_token::state::Account::unpack(&account.data).expect("token decodes");
        token.amount += amount;
        token.pack_into_slice(&mut account.data);
        self.svm.set_account(key, account).unwrap();
    }

    /// Accepts one allocation with the next valid sequence.
    pub fn accept_allocation(&mut self, transfer_id: [u8; 32], amount: u64) -> u64 {
        self.accept_allocation_at(transfer_id, amount, None)
    }

    pub fn accept_allocation_at(
        &mut self,
        transfer_id: [u8; 32],
        amount: u64,
        sequence: Option<u64>,
    ) -> u64 {
        let sequence = sequence.unwrap_or(self.control_lane().highest_consumed_sequence + 1);
        let bytes = self.allocate_bytes(transfer_id, amount, sequence);
        self.allocate(transfer_id, sequence, bytes)
            .expect("allocation lands");
        sequence
    }

    /// Encodes one Allocate against the shared inbound control lane.
    pub fn allocate_bytes(&self, transfer_id: [u8; 32], amount: u64, sequence: u64) -> Vec<u8> {
        let lane = self.control_lane();
        messages::MessageBuilder::allocate()
            .sequence(sequence)
            .previous_commitment(lane.message_commitment)
            .transfer_id(transfer_id)
            .allocate_body(|body| {
                body.amount = protocol_types::AssetAmount::new(u128::from(amount));
                body.minimum_destination_amount =
                    protocol_types::AssetAmount::new(u128::from(amount));
                body.config_version =
                    protocol_types::ConfigVersion::new(self.config().config_version);
            })
            .encode()
    }

    /// Accepts one recall with the next valid sequence.
    pub fn accept_recall(&mut self, transfer_id: [u8; 32], amount: u64) -> u64 {
        self.accept_recall_at(transfer_id, amount, None)
    }

    pub fn accept_recall_at(
        &mut self,
        transfer_id: [u8; 32],
        amount: u64,
        sequence: Option<u64>,
    ) -> u64 {
        let sequence = sequence.unwrap_or(self.control_lane().highest_consumed_sequence + 1);
        let bytes = self.recall_bytes(transfer_id, amount, sequence);
        self.recall(transfer_id, sequence, bytes)
            .expect("recall lands");
        sequence
    }

    /// Encodes one Recall against the shared inbound control lane.
    pub fn recall_bytes(&self, transfer_id: [u8; 32], amount: u64, sequence: u64) -> Vec<u8> {
        let lane = self.control_lane();
        messages::MessageBuilder::recall()
            .sequence(sequence)
            .previous_commitment(lane.message_commitment)
            .transfer_id(transfer_id)
            .recall_body(|body| {
                body.requested_amount = protocol_types::AssetAmount::new(u128::from(amount));
                body.minimum_return_amount = protocol_types::AssetAmount::new(u128::from(amount));
                body.config_version =
                    protocol_types::ConfigVersion::new(self.config().config_version);
            })
            .encode()
    }

    /// Asserts the last transfer message was consumed in order and held back.
    ///
    /// The lane moved on, the record keeps the original terms and the
    /// position holds it as the open cycle, without any financial effect.
    pub fn expect_quarantined(
        &self,
        transfer_id: [u8; 32],
        sequence: u64,
        kind: TransferKind,
        reason: QuarantineReason,
    ) {
        let event: MessageQuarantined = last_event(&self.last_logs());
        assert_eq!(event.reason, reason);
        assert_eq!(event.transfer_id, transfer_id);
        assert_eq!(event.sequence, sequence);
        assert_eq!(Some(event.message_class), kind.message_class());

        let lane = self.control_lane();
        assert_eq!(lane.highest_consumed_sequence, sequence);
        assert_eq!(event.lane_commitment, lane.message_commitment);
        let consumed = self.record(sequence);
        assert_eq!(Some(consumed.message_class), kind.message_class());
        assert_eq!(consumed.sequence, sequence);

        let record = self.transfer(&transfer_id);
        assert_eq!(record.status, TransferStatus::Quarantined);
        assert_eq!(record.transfer_kind, kind);
        assert_eq!(record.message_sequence, sequence);
        assert_eq!(record.attributed_amount, 0);
        assert_eq!(record.custody_principal_reserved, 0);

        let position = self.position();
        assert_eq!(position.active_transfer_status, TransferStatus::Quarantined);
        assert_eq!(position.active_transfer_kind, kind);
        assert_eq!(position.active_transfer_id, transfer_id);
        assert_eq!(position.active_transfer_sequence, sequence);
    }

    // Resolution

    /// Encodes one Admit for the transfer carried at `target_sequence`.
    pub fn resolve_bytes(
        &self,
        transfer_id: [u8; 32],
        target_sequence: u64,
        sequence: u64,
    ) -> Vec<u8> {
        let lane = self.control_lane();
        messages::MessageBuilder::resolve()
            .sequence(sequence)
            .previous_commitment(lane.message_commitment)
            .resolve_body(|body| {
                body.transfer_id = protocol_types::TransferId::new(transfer_id);
                body.target_sequence = protocol_types::Sequence::new(target_sequence);
                body.config_version =
                    protocol_types::ConfigVersion::new(self.config().config_version);
            })
            .encode()
    }

    pub fn resolve_instruction(
        &self,
        transfer_id: [u8; 32],
        sequence: u64,
        bytes: Vec<u8>,
        emergency_guardian: Option<Pubkey>,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::ProcessResolve {
            transport_verifier: self.verifier.pubkey(),
            remote_config: self.config,
            strategy_config: self.strategy(),
            remote_position: self.position_key(),
            control_lane: self.control_lane_key(),
            transfer_record: self.transfer_key(&transfer_id),
            consumed_message: self.asset_record_key(sequence),
            system_program: anchor_lang::system_program::ID,
            emergency_guardian,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::ProcessResolve {
                message_bytes: bytes,
            }
            .data(),
        }
    }

    /// Delivers one resolution signed by the transport verifier alone.
    pub fn resolve(&mut self, transfer_id: [u8; 32], sequence: u64, bytes: Vec<u8>) -> TxResult {
        let instruction = self.resolve_instruction(transfer_id, sequence, bytes, None);
        self.send_signed_by_verifier(instruction, self.verifier.pubkey())
    }

    /// Delivers one resolution that `cosigner` co-signs in the guardian's place.
    pub fn resolve_cosigned(
        &mut self,
        transfer_id: [u8; 32],
        sequence: u64,
        bytes: Vec<u8>,
        cosigner: &Keypair,
    ) -> TxResult {
        let instruction =
            self.resolve_instruction(transfer_id, sequence, bytes, Some(cosigner.pubkey()));
        let payer = self.administrator.insecure_clone();
        let verifier = self.verifier.insecure_clone();
        self.send_cosigned(instruction, &payer, &[&verifier, cosigner])
    }

    /// Admits the transfer carried at `target_sequence` with the next sequence.
    pub fn admit(&mut self, transfer_id: [u8; 32], target_sequence: u64) -> TxResult {
        let sequence = self.control_lane().highest_consumed_sequence + 1;
        let bytes = self.resolve_bytes(transfer_id, target_sequence, sequence);
        self.resolve(transfer_id, sequence, bytes)
    }

    /// Admits with the guardian co-signing, as a frozen leg requires.
    pub fn admit_cosigned(
        &mut self,
        transfer_id: [u8; 32],
        target_sequence: u64,
        cosigner: &Keypair,
    ) -> TxResult {
        let sequence = self.control_lane().highest_consumed_sequence + 1;
        let bytes = self.resolve_bytes(transfer_id, target_sequence, sequence);
        self.resolve_cosigned(transfer_id, sequence, bytes, cosigner)
    }

    /// Brings one full allocation all the way into attributed principal.
    pub fn fund_position(&mut self, transfer_id: [u8; 32], amount: u64) {
        self.accept_allocation(transfer_id, amount);
        self.credit(self.custody, amount);
        self.attribute(transfer_id).expect("attribution lands");
    }

    // Outbound reporting

    pub fn outbound_lane_key(&self) -> Pubkey {
        OutboundLane::address(&self.config, REPORT_LANE_ID).0
    }

    pub fn report_record_key(&self, sequence: u64) -> Pubkey {
        ReportRecord::address(&self.config, REPORT_LANE_ID, sequence).0
    }

    pub fn recall_marker_key(&self, transfer_id: &[u8; 32]) -> Pubkey {
        RecallSentMarker::address(&self.config, transfer_id).0
    }

    /// Every account the outbound instructions read.
    pub fn outbound_accounts(&self, sequence: u64) -> OutboundAccounts {
        OutboundAccounts {
            payer: self.outsider.pubkey(),
            remote_config: self.config,
            risk_config: self.risk(),
            strategy_config: self.strategy(),
            remote_position: self.position_key(),
            outbound_lane: self.outbound_lane_key(),
            custody_token_account: self.custody,
            adapter_state: self.adapter_state,
            report_record: self.report_record_key(sequence),
            system_program: anchor_lang::system_program::ID,
        }
    }

    pub fn initialize_outbound_state(&mut self) -> TxResult {
        let lane = self.outbound_lane_key();
        let administrator = self.administrator.pubkey();
        self.initialize_outbound_state_with(administrator, self.config, lane)
    }

    pub fn initialize_outbound_state_with(
        &mut self,
        administrator: Pubkey,
        remote_config: Pubkey,
        outbound_lane: Pubkey,
    ) -> TxResult {
        let metas = solevm_remote_leg::accounts::InitializeOutboundState {
            administrator,
            remote_config,
            outbound_lane,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);

        let instruction = Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::InitializeOutboundState {}.data(),
        };
        self.send_signed_by_named(instruction, administrator)
    }

    /// Signs with whichever known key the accounts name as the signer.
    fn send_signed_by_named(&mut self, instruction: Instruction, signer: Pubkey) -> TxResult {
        let payer = self.administrator.insecure_clone();
        if signer == payer.pubkey() {
            return self.send_as(instruction, &payer, &[&payer]);
        }
        let named = if signer == self.guardian.pubkey() {
            self.guardian.insecure_clone()
        } else if signer == self.verifier.pubkey() {
            self.verifier.insecure_clone()
        } else {
            self.outsider.insecure_clone()
        };
        self.send_as(instruction, &payer, &[&payer, &named])
    }

    pub fn publish_report(&mut self, epoch_id: u64) -> TxResult {
        let sequence = self.outbound_lane().next_sequence;
        let accounts = self.outbound_accounts(sequence);
        self.publish_report_with(accounts, epoch_id)
    }

    pub fn publish_report_with(&mut self, accounts: OutboundAccounts, epoch_id: u64) -> TxResult {
        let instruction = self.publish_report_instruction(accounts, epoch_id);
        let payer = self.outsider.insecure_clone();
        self.send_as(instruction, &payer, &[&payer])
    }

    pub fn publish_report_instruction(
        &self,
        accounts: OutboundAccounts,
        epoch_id: u64,
    ) -> Instruction {
        let metas = solevm_remote_leg::accounts::PublishRemoteReport {
            payer: accounts.payer,
            remote_config: accounts.remote_config,
            risk_config: accounts.risk_config,
            strategy_config: accounts.strategy_config,
            remote_position: accounts.remote_position,
            outbound_lane: accounts.outbound_lane,
            custody_token_account: accounts.custody_token_account,
            adapter_state: accounts.adapter_state,
            report_record: accounts.report_record,
            system_program: accounts.system_program,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::PublishRemoteReport { epoch_id }.data(),
        }
    }

    pub fn publish_recall_sent(&mut self, transfer_id: [u8; 32]) -> TxResult {
        let sequence = self.outbound_lane().next_sequence;
        let accounts = self.recall_sent_accounts(transfer_id, sequence);
        self.publish_recall_sent_with(accounts)
    }

    pub fn recall_sent_accounts(&self, transfer_id: [u8; 32], sequence: u64) -> RecallSentAccounts {
        RecallSentAccounts {
            payer: self.outsider.pubkey(),
            remote_config: self.config,
            risk_config: self.risk(),
            transfer_record: self.transfer_key(&transfer_id),
            outbound_lane: self.outbound_lane_key(),
            report_record: self.report_record_key(sequence),
            recall_sent_marker: self.recall_marker_key(&transfer_id),
            system_program: anchor_lang::system_program::ID,
        }
    }

    pub fn publish_recall_sent_with(&mut self, accounts: RecallSentAccounts) -> TxResult {
        let instruction = self.publish_recall_sent_instruction(accounts);
        let payer = self.outsider.insecure_clone();
        self.send_as(instruction, &payer, &[&payer])
    }

    pub fn publish_recall_sent_instruction(&self, accounts: RecallSentAccounts) -> Instruction {
        let metas = solevm_remote_leg::accounts::PublishRecallSent {
            payer: accounts.payer,
            remote_config: accounts.remote_config,
            risk_config: accounts.risk_config,
            transfer_record: accounts.transfer_record,
            outbound_lane: accounts.outbound_lane,
            report_record: accounts.report_record,
            recall_sent_marker: accounts.recall_sent_marker,
            system_program: accounts.system_program,
        }
        .to_account_metas(None);

        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: metas,
            data: solevm_remote_leg::instruction::PublishRecallSent {}.data(),
        }
    }

    pub fn outbound_lane(&self) -> OutboundLane {
        let account = self
            .svm
            .get_account(&self.outbound_lane_key())
            .expect("outbound lane exists");
        OutboundLane::try_deserialize(&mut account.data.as_slice()).expect("lane decodes")
    }

    pub fn outbound_lane_exists(&self) -> bool {
        self.svm
            .get_account(&self.outbound_lane_key())
            .is_some_and(|account| {
                account.owner == solevm_remote_leg::ID && !account.data.is_empty()
            })
    }

    pub fn report_record(&self, sequence: u64) -> ReportRecord {
        let account = self
            .svm
            .get_account(&self.report_record_key(sequence))
            .expect("report record exists");
        ReportRecord::try_deserialize(&mut account.data.as_slice()).expect("record decodes")
    }

    pub fn report_record_exists(&self, sequence: u64) -> bool {
        self.svm
            .get_account(&self.report_record_key(sequence))
            .is_some_and(|account| {
                account.owner == solevm_remote_leg::ID && !account.data.is_empty()
            })
    }

    pub fn recall_marker(&self, transfer_id: &[u8; 32]) -> RecallSentMarker {
        let account = self
            .svm
            .get_account(&self.recall_marker_key(transfer_id))
            .expect("marker exists");
        RecallSentMarker::try_deserialize(&mut account.data.as_slice()).expect("marker decodes")
    }

    /// Sends a publication and returns the event it emitted.
    pub fn publish_and_capture<T>(&mut self, instruction: Instruction) -> T
    where
        T: anchor_lang::Discriminator + anchor_lang::AnchorDeserialize,
    {
        let payer = self.outsider.insecure_clone();
        self.svm.expire_blockhash();
        let transaction = Transaction::new_signed_with_payer(
            &[instruction],
            Some(&payer.pubkey()),
            &[&payer],
            self.svm.latest_blockhash(),
        );
        let meta = self
            .svm
            .send_transaction(transaction)
            .expect("publication lands");
        last_event(&meta.logs)
    }

    /// Sends one publication signed by a chosen payer and reads its event.
    pub fn publish_and_capture_as<T>(&mut self, instruction: Instruction, payer: &Keypair) -> T
    where
        T: anchor_lang::Discriminator + anchor_lang::AnchorDeserialize,
    {
        self.svm.expire_blockhash();
        let transaction = Transaction::new_signed_with_payer(
            &[instruction],
            Some(&payer.pubkey()),
            &[payer],
            self.svm.latest_blockhash(),
        );
        let meta = self
            .svm
            .send_transaction(transaction)
            .expect("publication lands");
        last_event(&meta.logs)
    }

    /// How many accounts this program owns right now.
    pub fn owned_account_count(&self) -> usize {
        [
            self.config,
            self.risk(),
            self.strategy(),
            self.position_key(),
            self.outbound_lane_key(),
        ]
        .into_iter()
        .chain((1..8).map(|sequence| self.report_record_key(sequence)))
        .chain([
            self.recall_marker_key(&ALLOCATE_TRANSFER_ID),
            self.recall_marker_key(&RECALL_TRANSFER_ID),
        ])
        .filter(|key| {
            self.svm
                .get_account(key)
                .is_some_and(|account| account.owner == solevm_remote_leg::ID)
        })
        .count()
    }

    /// Gives one account to a stranger, keeping its lamports and size.
    pub fn give_account_to(&mut self, key: Pubkey, owner: Pubkey, space: usize) {
        let lamports = self.svm.minimum_balance_for_rent_exemption(space);
        self.svm
            .set_account(
                key,
                Account {
                    lamports,
                    data: vec![0u8; space],
                    owner,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    /// Forces a token balance, the way an outside transfer would.
    pub fn set_token_amount(&mut self, key: Pubkey, amount: u64) {
        let mut account = self.svm.get_account(&key).expect("token account exists");
        let mut token = spl_token::state::Account::unpack(&account.data).expect("token decodes");
        token.amount = amount;
        token.pack_into_slice(&mut account.data);
        self.svm.set_account(key, account).unwrap();
    }

    /// Rewrites the principal the adapter records, leaving its vault alone.
    pub fn set_adapter_principal(&mut self, principal: u64) {
        let mut account = self
            .svm
            .get_account(&self.adapter_state)
            .expect("adapter state exists");
        let offset = solevm_remote_leg::adapter::PRINCIPAL_OFFSET;
        account.data[offset..offset + 8].copy_from_slice(&principal.to_le_bytes());
        self.svm.set_account(self.adapter_state, account).unwrap();
    }

    /// Rewrites one byte of an account, the way corruption would.
    pub fn overwrite_byte(&mut self, key: Pubkey, offset: usize, value: u8) {
        let mut account = self.svm.get_account(&key).expect("account exists");
        account.data[offset] = value;
        self.svm.set_account(key, account).unwrap();
    }

    /// Shortens the adapter account, as a smaller layout would.
    pub fn truncate_adapter_state(&mut self, length: usize) {
        let mut account = self
            .svm
            .get_account(&self.adapter_state)
            .expect("adapter state exists");
        account.data.truncate(length);
        self.svm.set_account(self.adapter_state, account).unwrap();
    }

    /// Hands the adapter account to another program.
    pub fn reassign_adapter_state(&mut self, owner: Pubkey) {
        let mut account = self
            .svm
            .get_account(&self.adapter_state)
            .expect("adapter state exists");
        account.owner = owner;
        self.svm.set_account(self.adapter_state, account).unwrap();
    }

    /// Rewrites the account tag the adapter wrote.
    pub fn overwrite_adapter_tag(&mut self, tag: [u8; 8]) {
        let mut account = self
            .svm
            .get_account(&self.adapter_state)
            .expect("adapter state exists");
        account.data[..8].copy_from_slice(&tag);
        self.svm.set_account(self.adapter_state, account).unwrap();
    }

    /// Principal the adapter records in its own account.
    pub fn adapter_principal(&self) -> u64 {
        let account = self
            .svm
            .get_account(&self.adapter_state)
            .expect("adapter state exists");
        let offset = solevm_remote_leg::adapter::PRINCIPAL_OFFSET;
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&account.data[offset..offset + 8]);
        u64::from_le_bytes(bytes)
    }

    /// Logs of the last transaction this fixture sent.
    pub fn last_logs(&self) -> Vec<String> {
        self.last_logs.clone()
    }

    /// A leg funded, deployed and ready to publish reports.
    pub fn ready_to_publish() -> Self {
        let mut fixture = Self::deployed();
        fixture
            .initialize_outbound_state()
            .expect("outbound state initializes");
        fixture
    }
}

/// Every account the report publication instruction reads.
#[derive(Clone, Copy, Debug)]
pub struct OutboundAccounts {
    pub payer: Pubkey,
    pub remote_config: Pubkey,
    pub risk_config: Pubkey,
    pub strategy_config: Pubkey,
    pub remote_position: Pubkey,
    pub outbound_lane: Pubkey,
    pub custody_token_account: Pubkey,
    pub adapter_state: Pubkey,
    pub report_record: Pubkey,
    pub system_program: Pubkey,
}

/// Every account the recall notice instruction reads.
#[derive(Clone, Copy, Debug)]
pub struct RecallSentAccounts {
    pub payer: Pubkey,
    pub remote_config: Pubkey,
    pub risk_config: Pubkey,
    pub transfer_record: Pubkey,
    pub outbound_lane: Pubkey,
    pub report_record: Pubkey,
    pub recall_sent_marker: Pubkey,
    pub system_program: Pubkey,
}

/// Reads the last event of one type out of the transaction logs.
///
/// Anchor writes events as base64 program data, so the bytes travel whole.
pub fn last_event<T>(logs: &[String]) -> T
where
    T: anchor_lang::Discriminator + anchor_lang::AnchorDeserialize,
{
    logs.iter()
        .filter_map(|line| line.strip_prefix("Program data: "))
        .filter_map(|payload| BASE64.decode(payload).ok())
        .filter_map(|decoded| {
            let (tag, body) = decoded.split_at_checked(8)?;
            if tag != T::DISCRIMINATOR {
                return None;
            }
            T::try_from_slice(body).ok()
        })
        .next_back()
        .expect("an event of this type was emitted")
}

/// Every account the control state instruction reads.
#[derive(Clone, Copy, Debug)]
pub struct ControlAccounts {
    pub administrator: Pubkey,
    pub remote_config: Pubkey,
    pub risk_config: Pubkey,
    pub control_lane: Pubkey,
    pub system_program: Pubkey,
}

/// Every account the config update instruction reads.
#[derive(Clone, Copy, Debug)]
pub struct UpdateAccounts {
    pub transport_verifier: Pubkey,
    pub remote_config: Pubkey,
    pub risk_config: Pubkey,
    pub control_lane: Pubkey,
    pub consumed_message: Pubkey,
    pub system_program: Pubkey,
}

/// Every account the strategy instructions read.
#[derive(Clone, Copy, Debug)]
pub struct StrategyAccounts {
    pub administrator: Pubkey,
    pub remote_config: Pubkey,
    pub risk_config: Pubkey,
    pub strategy_config: Pubkey,
    pub remote_position: Pubkey,
    pub adapter_program: Pubkey,
    pub adapter_state: Pubkey,
    pub adapter_authority: Pubkey,
    pub adapter_token_vault: Pubkey,
    pub asset_mint: Pubkey,
    pub token_program: Pubkey,
    pub system_program: Pubkey,
}

/// Every account an allocate or recall message instruction reads.
#[derive(Clone, Copy, Debug)]
pub struct AssetAccounts {
    pub transport_verifier: Pubkey,
    pub remote_config: Pubkey,
    pub risk_config: Pubkey,
    pub strategy_config: Pubkey,
    pub remote_position: Pubkey,
    pub replay_lane: Pubkey,
    pub transfer_record: Pubkey,
    pub consumed_message: Pubkey,
    pub system_program: Pubkey,
}

/// Reads the program built by cargo build-sbf.
pub fn program_bytes() -> Vec<u8> {
    built_program("solevm_remote_leg", "SBF_OUT_DIR", "deploy")
}

/// Reads the strategy adapter built by cargo build-sbf.
pub fn adapter_bytes() -> Vec<u8> {
    built_program("solevm_test_strategy", "SBF_OUT_DIR", "deploy")
}

/// The loader's program data header, which precedes the program bytes.
pub fn program_data(upgrade_authority: Option<Pubkey>) -> Vec<u8> {
    // bincode of UpgradeableLoaderState::ProgramData: variant, slot, authority.
    let mut data = 3u32.to_le_bytes().to_vec();
    data.extend_from_slice(&0u64.to_le_bytes());
    match upgrade_authority {
        Some(authority) => {
            data.push(1);
            data.extend_from_slice(authority.as_ref());
        }
        None => data.resize(45, 0),
    }
    data
}

/// Reads the strategy adapter built with its `hostile` feature.
pub fn hostile_adapter_bytes() -> Vec<u8> {
    built_program(
        "solevm_test_strategy",
        "HOSTILE_SBF_OUT_DIR",
        "deploy-hostile",
    )
}

fn built_program(name: &str, directory_variable: &str, default_directory: &str) -> Vec<u8> {
    let mut paths = Vec::new();
    if let Ok(directory) = std::env::var(directory_variable) {
        paths.push(format!("{directory}/{name}.so"));
    }
    paths.push(format!(
        "{}/target/{default_directory}/{name}.so",
        env!("CARGO_MANIFEST_DIR")
    ));
    paths.push(format!(
        "{}/../target/{default_directory}/{name}.so",
        env!("CARGO_MANIFEST_DIR")
    ));
    for path in &paths {
        if let Ok(bytes) = std::fs::read(path) {
            return bytes;
        }
    }
    panic!(
        "missing {name}.so, looked in {}, build it first as the README describes",
        paths.join(", ")
    );
}

fn seeded_keypair(slot: u8) -> Keypair {
    Keypair::new_from_array([slot; 32])
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
