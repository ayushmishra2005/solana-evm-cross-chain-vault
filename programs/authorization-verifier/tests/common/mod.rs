//! LiteSVM fixture that runs the built verifier beside the native
//! secp256r1 precompile.

#![allow(dead_code, unreachable_pub)]
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

pub mod deployment;

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use anchor_lang::solana_program::clock::Clock;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use es256_authorization_verifier::{
    CONFIG_SEED, ConsumedAuthorization, Environment, InitializeConfigParams, MANDATE_SEED,
    RECEIPT_SEED, RegisterMandateParams, RegisterTrustedKeyParams, TRUSTED_KEY_SEED, VerifierError,
    mandate_hash, nonce_hash,
};
use es256_jws_adapter::{SyntheticSigner, normalize_signature, secp256r1_instruction};
use litesvm::LiteSVM;
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use solana_instruction::error::InstructionError;
use solana_keypair::Keypair;
use solana_precompile_error::PrecompileError;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

pub const NOW: i64 = 1_800_000_000;
pub const KID: &str = "verdict-test-01";
pub const MANDATE_ID: &str = "mnd_7f3a";
pub const AMOUNT_MICROS: u64 = 340_000_000;
pub const CURRENCY: &str = "USD";
pub const RECIPIENT: &str = "vendor:acme_saas";
pub const POLICY_VERSION: &str = "pol_2026-08-12.1";
pub const AGENT: &str = "agt_procurement_01";
pub const CATEGORY: &str = "saas";
pub const NONCE: &str = "n_9d02c6e4";
pub const KEY_NOT_BEFORE: i64 = NOW - 86_400;
pub const KEY_NOT_AFTER: i64 = NOW + 86_400;

/// Address the test-only CPI forwarder is loaded at.
pub const FORWARDER_ID: Pubkey = Pubkey::new_from_array([0xF0; 32]);

/// `SystemError::AccountAlreadyInUse`, returned when `init` finds the
/// account already allocated.
pub const ACCOUNT_ALREADY_IN_USE: u32 = 0;
/// `SystemError::ResultWithNegativeLamports`, returned by an overdrawn
/// transfer.
pub const RESULT_WITH_NEGATIVE_LAMPORTS: u32 = 1;

/// Account positions in `consume_authorization`.
pub mod consume_account {
    pub const CONSUMER: usize = 0;
    pub const CONFIG: usize = 1;
    pub const TRUSTED_KEY: usize = 2;
    pub const MANDATE: usize = 3;
    pub const RECEIPT: usize = 4;
    pub const INSTRUCTIONS_SYSVAR: usize = 5;
}

pub type TxResult = Result<TransactionMetadata, Box<FailedTransactionMetadata>>;

/// The claims of one synthetic verdict, as JSON fragments.
#[derive(Clone, Debug)]
pub struct VerdictClaims {
    pub kid: String,
    pub iss: String,
    pub jti: String,
    pub iat: i64,
    pub exp: i64,
    pub verdict: String,
    pub mandate: String,
    pub policy_version: String,
    pub nonce: String,
    pub agent: String,
    /// The raw JSON number token.
    pub amount: String,
    pub currency: String,
    pub recipient: String,
    pub category: String,
}

impl Default for VerdictClaims {
    fn default() -> Self {
        Self {
            kid: KID.into(),
            iss: "https://verdicts.saifuro.com".into(),
            jti: "dec_5b2e".into(),
            iat: NOW,
            exp: NOW + 300,
            verdict: "allow".into(),
            mandate: MANDATE_ID.into(),
            policy_version: POLICY_VERSION.into(),
            nonce: NONCE.into(),
            agent: AGENT.into(),
            amount: "340".into(),
            currency: CURRENCY.into(),
            recipient: RECIPIENT.into(),
            category: CATEGORY.into(),
        }
    }
}

impl VerdictClaims {
    /// Claims issued at `iat` with the standard 300-second lifetime.
    pub fn issued_at(iat: i64) -> Self {
        Self {
            iat,
            exp: iat + 300,
            ..Self::default()
        }
    }

    pub fn header_json(&self) -> String {
        format!(r#"{{"alg":"ES256","typ":"JWT","kid":"{}"}}"#, self.kid)
    }

    pub fn payload_json(&self) -> String {
        format!(
            concat!(
                r#"{{"iss":"{}","jti":"{}","iat":{},"exp":{},"verdict":"{}","mandate":"{}","#,
                r#""policy_version":"{}","nonce":"{}","request":{{"agent":"{}","amount":{},"#,
                r#""currency":"{}","recipient":"{}","category":"{}"}}}}"#,
            ),
            self.iss,
            self.jti,
            self.iat,
            self.exp,
            self.verdict,
            self.mandate,
            self.policy_version,
            self.nonce,
            self.agent,
            self.amount,
            self.currency,
            self.recipient,
            self.category,
        )
    }

    pub fn signing_input(&self) -> Vec<u8> {
        es256_jws_adapter::signing_input(
            self.header_json().as_bytes(),
            self.payload_json().as_bytes(),
        )
    }
}

/// A signed verdict and the precompile key it is presented with.
#[derive(Clone, Debug)]
pub struct SignedVerdict {
    pub signing_input: Vec<u8>,
    pub signature: [u8; 64],
    pub public_key: [u8; 33],
    pub nonce: String,
}

impl SignedVerdict {
    pub fn sign(signer: &SyntheticSigner, claims: &VerdictClaims) -> Self {
        Self::sign_input(signer, claims.signing_input(), &claims.nonce)
    }

    /// Signs arbitrary bytes as if they were a signing input. `nonce` only
    /// selects the receipt address the consume instruction names.
    pub fn sign_input(signer: &SyntheticSigner, signing_input: Vec<u8>, nonce: &str) -> Self {
        let signature = normalize_signature(&signer.sign(&signing_input).unwrap()).unwrap();
        Self {
            signing_input,
            signature,
            public_key: signer.public_key().unwrap(),
            nonce: nonce.into(),
        }
    }

    /// The canonical precompile instruction. The adapter refuses messages
    /// over the verifier's limit, so oversized attack inputs are built by
    /// hand in the same layout.
    pub fn precompile(&self) -> Instruction {
        if self.signing_input.len() > es256_jws_adapter::MAX_SIGNING_INPUT_LEN {
            let mut body = self.public_key.to_vec();
            body.extend_from_slice(&self.signature);
            body.extend_from_slice(&self.signing_input);
            return raw_precompile(1, 0, &[Offsets::canonical(self.signing_input.len())], &body);
        }
        secp256r1_instruction(&self.public_key, &self.signature, &self.signing_input).unwrap()
    }

    pub fn nonce_hash(&self) -> [u8; 32] {
        nonce_hash(self.nonce.as_bytes())
    }
}

/// The seven little-endian offsets of one native signature entry.
#[derive(Clone, Copy, Debug)]
pub struct Offsets {
    pub signature_offset: u16,
    pub signature_instruction: u16,
    pub public_key_offset: u16,
    pub public_key_instruction: u16,
    pub message_offset: u16,
    pub message_len: u16,
    pub message_instruction: u16,
}

impl Offsets {
    /// The layout the verifier accepts for a message of `message_len` bytes.
    pub fn canonical(message_len: usize) -> Self {
        Self {
            signature_offset: 49,
            signature_instruction: u16::MAX,
            public_key_offset: 16,
            public_key_instruction: u16::MAX,
            message_offset: 113,
            message_len: u16::try_from(message_len).unwrap(),
            message_instruction: u16::MAX,
        }
    }

    fn bytes(self) -> Vec<u8> {
        [
            self.signature_offset,
            self.signature_instruction,
            self.public_key_offset,
            self.public_key_instruction,
            self.message_offset,
            self.message_len,
            self.message_instruction,
        ]
        .iter()
        .flat_map(|field| field.to_le_bytes())
        .collect()
    }
}

/// A secp256r1 instruction with a hand-built header, for layout attacks.
pub fn raw_precompile(count: u8, padding: u8, entries: &[Offsets], body: &[u8]) -> Instruction {
    let mut data = vec![count, padding];
    for entry in entries {
        data.extend(entry.bytes());
    }
    data.extend_from_slice(body);
    Instruction {
        program_id: solana_sdk_ids::secp256r1_program::ID,
        accounts: Vec::new(),
        data,
    }
}

pub fn approved_mandate() -> RegisterMandateParams {
    RegisterMandateParams {
        mandate_hash: mandate_hash(MANDATE_ID.as_bytes()),
        mandate_id: MANDATE_ID.into(),
        amount_micros: AMOUNT_MICROS,
        currency: CURRENCY.into(),
        recipient: RECIPIENT.into(),
        policy_version: POLICY_VERSION.into(),
        agent: AGENT.into(),
        category: CATEGORY.into(),
    }
}

pub struct Fixture {
    pub svm: LiteSVM,
    pub deployer: Keypair,
    pub administrator: Keypair,
    pub consumer: Keypair,
    pub outsider: Keypair,
    pub config: Pubkey,
    pub signer: SyntheticSigner,
    pub trusted_key: Pubkey,
    pub mandate: Pubkey,
}

impl Fixture {
    /// A production config with one trusted key and one approved mandate.
    pub fn new() -> Self {
        let mut fixture = Self::deployed();
        fixture
            .initialize_config(&fixture.deployer.insecure_clone(), Environment::Production)
            .expect("config initializes");
        let public_key = fixture.signer.public_key().unwrap();
        fixture.trusted_key = fixture
            .register_key(KID, public_key)
            .expect("trusted key registers");
        fixture.mandate = fixture
            .register_mandate(approved_mandate())
            .expect("mandate registers");
        fixture
    }

    /// The program deployed with `deployer` as upgrade authority, before any
    /// config exists.
    pub fn deployed() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program(
            es256_authorization_verifier::ID,
            &built_program("es256_authorization_verifier"),
        )
        .expect("program loads");
        let mut clock = svm.get_sysvar::<Clock>();
        clock.unix_timestamp = NOW;
        svm.set_sysvar(&clock);

        let deployer = Keypair::new();
        let administrator = Keypair::new();
        let consumer = Keypair::new();
        let outsider = Keypair::new();
        for keypair in [&deployer, &administrator, &consumer, &outsider] {
            svm.airdrop(&keypair.pubkey(), 10_000_000_000).unwrap();
        }
        svm.set_account(
            program_data_address(),
            solana_account::Account {
                lamports: 1_000_000_000,
                data: program_data(Some(deployer.pubkey())),
                owner: bpf_loader_upgradeable::ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();

        Self {
            svm,
            deployer,
            administrator,
            consumer,
            outsider,
            config: config_address(Environment::Production),
            signer: SyntheticSigner::generate().unwrap(),
            trusted_key: Pubkey::default(),
            mandate: Pubkey::default(),
        }
    }

    /// Loads the test-only CPI forwarder at [`FORWARDER_ID`].
    pub fn add_cpi_forwarder(&mut self) {
        self.svm
            .add_program(FORWARDER_ID, &built_program("es256_cpi_forwarder"))
            .expect("forwarder loads");
    }

    pub fn set_clock(&mut self, unix_timestamp: i64) {
        let mut clock = self.svm.get_sysvar::<Clock>();
        clock.unix_timestamp = unix_timestamp;
        self.svm.set_sysvar(&clock);
    }

    /// Sends one transaction with a fresh blockhash, so a repeat is a new
    /// transaction rather than a history duplicate.
    pub fn send(&mut self, instructions: &[Instruction], signers: &[&Keypair]) -> TxResult {
        let transaction = self.transaction(instructions, signers);
        self.svm.send_transaction(transaction).map_err(Box::new)
    }

    /// Sends `instructions` paid and signed by the configured consumer.
    pub fn send_as_consumer(&mut self, instructions: &[Instruction]) -> TxResult {
        let consumer = self.consumer.insecure_clone();
        self.send(instructions, &[&consumer])
    }

    pub fn transaction(
        &mut self,
        instructions: &[Instruction],
        signers: &[&Keypair],
    ) -> Transaction {
        self.svm.expire_blockhash();
        Transaction::new_signed_with_payer(
            instructions,
            Some(&signers[0].pubkey()),
            signers,
            self.svm.latest_blockhash(),
        )
    }

    pub fn initialize_config(&mut self, authority: &Keypair, environment: Environment) -> TxResult {
        let params = InitializeConfigParams {
            environment,
            administrator: self.administrator.pubkey(),
            consumer: self.consumer.pubkey(),
        };
        self.initialize_config_with(authority, program_data_address(), params)
    }

    pub fn initialize_config_with(
        &mut self,
        authority: &Keypair,
        program_data: Pubkey,
        params: InitializeConfigParams,
    ) -> TxResult {
        let instruction = Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::InitializeConfig {
                upgrade_authority: authority.pubkey(),
                program_data,
                config: config_address(params.environment),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::InitializeConfig { params }.data(),
        };
        self.send(&[instruction], &[authority])
    }

    /// Initializes a second config for `environment` and makes it current.
    pub fn switch_to_new_config(&mut self, environment: Environment) {
        let deployer = self.deployer.insecure_clone();
        self.initialize_config(&deployer, environment)
            .expect("config initializes");
        self.config = config_address(environment);
    }

    pub fn register_key(
        &mut self,
        kid: &str,
        public_key: [u8; 33],
    ) -> Result<Pubkey, Box<FailedTransactionMetadata>> {
        self.register_key_with(kid, public_key, KEY_NOT_BEFORE, KEY_NOT_AFTER)
    }

    pub fn register_key_with(
        &mut self,
        kid: &str,
        public_key: [u8; 33],
        not_before: i64,
        not_after: i64,
    ) -> Result<Pubkey, Box<FailedTransactionMetadata>> {
        let params = RegisterTrustedKeyParams {
            kid: kid.into(),
            public_key,
            not_before,
            not_after,
        };
        let trusted_key = trusted_key_address(self.config, kid);
        let instruction =
            self.register_key_instruction(self.administrator.pubkey(), trusted_key, params);
        let administrator = self.administrator.insecure_clone();
        self.send(&[instruction], &[&administrator])?;
        Ok(trusted_key)
    }

    pub fn register_key_instruction(
        &self,
        administrator: Pubkey,
        trusted_key: Pubkey,
        params: RegisterTrustedKeyParams,
    ) -> Instruction {
        Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::RegisterTrustedKey {
                administrator,
                config: self.config,
                trusted_key,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::RegisterTrustedKey { params }.data(),
        }
    }

    pub fn revoke_key(&mut self, trusted_key: Pubkey) -> TxResult {
        let instruction = self.revoke_key_instruction(self.administrator.pubkey(), trusted_key);
        let administrator = self.administrator.insecure_clone();
        self.send(&[instruction], &[&administrator])
    }

    pub fn revoke_key_instruction(
        &self,
        administrator: Pubkey,
        trusted_key: Pubkey,
    ) -> Instruction {
        Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::RevokeTrustedKey {
                administrator,
                config: self.config,
                trusted_key,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::RevokeTrustedKey {}.data(),
        }
    }

    pub fn register_mandate(
        &mut self,
        params: RegisterMandateParams,
    ) -> Result<Pubkey, Box<FailedTransactionMetadata>> {
        let mandate = mandate_address(self.config, &params.mandate_hash);
        let instruction = self.register_mandate_instruction(self.administrator.pubkey(), params);
        let administrator = self.administrator.insecure_clone();
        self.send(&[instruction], &[&administrator])?;
        Ok(mandate)
    }

    pub fn register_mandate_instruction(
        &self,
        administrator: Pubkey,
        params: RegisterMandateParams,
    ) -> Instruction {
        Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::RegisterMandate {
                administrator,
                config: self.config,
                mandate: mandate_address(self.config, &params.mandate_hash),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::RegisterMandate { params }.data(),
        }
    }

    pub fn revoke_mandate(&mut self, mandate: Pubkey) -> TxResult {
        let instruction = self.revoke_mandate_instruction(self.administrator.pubkey(), mandate);
        let administrator = self.administrator.insecure_clone();
        self.send(&[instruction], &[&administrator])
    }

    pub fn revoke_mandate_instruction(
        &self,
        administrator: Pubkey,
        mandate: Pubkey,
    ) -> Instruction {
        Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::RevokeMandate {
                administrator,
                config: self.config,
                mandate,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::RevokeMandate {}.data(),
        }
    }

    pub fn consume_instruction(&self, nonce_hash: [u8; 32]) -> Instruction {
        Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::ConsumeAuthorization {
                consumer: self.consumer.pubkey(),
                config: self.config,
                trusted_key: self.trusted_key,
                mandate: self.mandate,
                receipt: self.receipt_address(&nonce_hash),
                instructions_sysvar: solana_sdk_ids::sysvar::instructions::ID,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::ConsumeAuthorization { nonce_hash }
                .data(),
        }
    }

    /// The approved shape: the precompile, then consume.
    pub fn consume(&mut self, verdict: &SignedVerdict) -> TxResult {
        let instructions = [
            verdict.precompile(),
            self.consume_instruction(verdict.nonce_hash()),
        ];
        self.send_as_consumer(&instructions)
    }

    /// `consume_instruction` relayed through the CPI forwarder.
    pub fn forwarded_consume_instruction(&self, nonce_hash: [u8; 32]) -> Instruction {
        let consume = self.consume_instruction(nonce_hash);
        let mut accounts = vec![AccountMeta::new_readonly(
            es256_authorization_verifier::ID,
            false,
        )];
        accounts.extend(consume.accounts);
        Instruction {
            program_id: FORWARDER_ID,
            accounts,
            data: consume.data,
        }
    }

    pub fn sign(&self, claims: &VerdictClaims) -> SignedVerdict {
        SignedVerdict::sign(&self.signer, claims)
    }

    /// The trusted key's signature over arbitrary bytes.
    pub fn sign_input(&self, signing_input: Vec<u8>) -> SignedVerdict {
        SignedVerdict::sign_input(&self.signer, signing_input, NONCE)
    }

    /// The trusted key's signature over a raw header and payload.
    pub fn sign_json(&self, header_json: &str, payload_json: &str) -> SignedVerdict {
        self.sign_input(es256_jws_adapter::signing_input(
            header_json.as_bytes(),
            payload_json.as_bytes(),
        ))
    }

    pub fn receipt_address(&self, nonce_hash: &[u8; 32]) -> Pubkey {
        receipt_address(self.config, nonce_hash)
    }

    pub fn receipt(&self, nonce_hash: &[u8; 32]) -> Option<ConsumedAuthorization> {
        let account = self.svm.get_account(&self.receipt_address(nonce_hash))?;
        if account.lamports == 0 {
            return None;
        }
        Some(ConsumedAuthorization::try_deserialize(&mut account.data.as_slice()).unwrap())
    }

    /// No account, not even an empty one, exists at the receipt address.
    pub fn assert_no_receipt(&self, nonce_hash: &[u8; 32]) {
        let account = self.svm.get_account(&self.receipt_address(nonce_hash));
        assert!(
            account.is_none_or(|account| account.lamports == 0 && account.data.is_empty()),
            "a rejected attempt left a receipt account"
        );
    }

    pub fn assert_no_account(&self, address: &Pubkey) {
        let account = self.svm.get_account(address);
        assert!(
            account.is_none_or(|account| account.lamports == 0 && account.data.is_empty()),
            "an account exists at {address}"
        );
    }

    pub fn balance(&self, address: &Pubkey) -> u64 {
        self.svm.get_balance(address).unwrap_or(0)
    }
}

pub fn config_address(environment: Environment) -> Pubkey {
    Pubkey::find_program_address(
        &[CONFIG_SEED, &environment.seed()],
        &es256_authorization_verifier::ID,
    )
    .0
}

pub fn trusted_key_address(config: Pubkey, kid: &str) -> Pubkey {
    Pubkey::find_program_address(
        &[TRUSTED_KEY_SEED, config.as_ref(), kid.as_bytes()],
        &es256_authorization_verifier::ID,
    )
    .0
}

/// The one address every kid longer than a seed derives; no valid kid can
/// ever occupy it.
pub fn over_long_key_id_address(config: Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[TRUSTED_KEY_SEED, config.as_ref(), &[]],
        &es256_authorization_verifier::ID,
    )
    .0
}

pub fn mandate_address(config: Pubkey, mandate_hash: &[u8; 32]) -> Pubkey {
    Pubkey::find_program_address(
        &[MANDATE_SEED, config.as_ref(), mandate_hash],
        &es256_authorization_verifier::ID,
    )
    .0
}

pub fn receipt_address(config: Pubkey, nonce_hash: &[u8; 32]) -> Pubkey {
    Pubkey::find_program_address(
        &[RECEIPT_SEED, config.as_ref(), nonce_hash],
        &es256_authorization_verifier::ID,
    )
    .0
}

pub fn program_data_address() -> Pubkey {
    Pubkey::find_program_address(
        &[es256_authorization_verifier::ID.as_ref()],
        &bpf_loader_upgradeable::ID,
    )
    .0
}

/// The loader's program-data header: variant, slot, then the authority.
pub fn program_data(upgrade_authority: Option<Pubkey>) -> Vec<u8> {
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

/// The directory the built programs are read from: `SBF_OUT_DIR` when set,
/// with no fallback, so a run against one architecture can never silently
/// load the other.
pub fn sbf_out_dir() -> String {
    std::env::var("SBF_OUT_DIR")
        .unwrap_or_else(|_| format!("{}/target/deploy", env!("CARGO_MANIFEST_DIR")))
}

/// Reads a program built by `cargo-build-sbf`.
pub fn built_program(name: &str) -> Vec<u8> {
    let path = format!("{}/{name}.so", sbf_out_dir());
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("cannot read {path}: {error}; run cargo-build-sbf first"))
}

/// The failing instruction and its error.
pub fn instruction_error(result: TxResult) -> (u8, InstructionError) {
    match result {
        Ok(_) => panic!("transaction unexpectedly succeeded"),
        Err(failure) => match failure.err {
            TransactionError::InstructionError(index, error) => (index, error),
            other => panic!("expected an instruction error, got {other:?}"),
        },
    }
}

/// The verifier program itself rejected instruction `index`.
pub fn assert_verifier_error(result: TxResult, index: u8, expected: VerifierError) {
    assert_eq!(
        instruction_error(result),
        (index, InstructionError::Custom(u32::from(expected))),
        "expected verifier error {expected:?}"
    );
}

/// Anchor's account validation rejected instruction `index`.
pub fn assert_anchor_error(result: TxResult, index: u8, expected: anchor_lang::error::ErrorCode) {
    let name = format!("{expected:?}");
    assert_eq!(
        instruction_error(result),
        (index, InstructionError::Custom(u32::from(expected))),
        "expected Anchor error {name}"
    );
}

/// The native secp256r1 program rejected instruction `index`.
pub fn assert_precompile_error(result: TxResult, index: u8, expected: PrecompileError) {
    let name = format!("{expected:?}");
    assert_eq!(
        instruction_error(result),
        (index, InstructionError::Custom(expected as u32)),
        "expected native precompile error {name}"
    );
}

/// Serialized size of a legacy transaction: signature count, signatures and
/// message.
pub fn transaction_size(transaction: &Transaction) -> usize {
    1 + 64 * transaction.signatures.len() + transaction.message_data().len()
}
