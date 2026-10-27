//! LiteSVM fixture that runs the built verifier beside the native
//! secp256r1 precompile.

#![allow(dead_code, unreachable_pub)]
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use anchor_lang::solana_program::clock::Clock;
use anchor_lang::solana_program::instruction::Instruction;
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
pub const KEY_NOT_BEFORE: i64 = NOW - 86_400;
pub const KEY_NOT_AFTER: i64 = NOW + 86_400;

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
            nonce: "n_9d02c6e4".into(),
            agent: AGENT.into(),
            amount: "340".into(),
            currency: CURRENCY.into(),
            recipient: RECIPIENT.into(),
            category: CATEGORY.into(),
        }
    }
}

impl VerdictClaims {
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
        let signing_input = es256_jws_adapter::signing_input(
            claims.header_json().as_bytes(),
            claims.payload_json().as_bytes(),
        );
        let signature = normalize_signature(&signer.sign(&signing_input).unwrap()).unwrap();
        Self {
            signing_input,
            signature,
            public_key: signer.public_key().unwrap(),
            nonce: claims.nonce.clone(),
        }
    }

    pub fn precompile(&self) -> Instruction {
        secp256r1_instruction(&self.public_key, &self.signature, &self.signing_input).unwrap()
    }

    pub fn nonce_hash(&self) -> [u8; 32] {
        nonce_hash(self.nonce.as_bytes())
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
        svm.add_program(es256_authorization_verifier::ID, &program_bytes())
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

    /// Sends one transaction with a fresh blockhash, so a repeat is a new
    /// transaction rather than a history duplicate.
    pub fn send(&mut self, instructions: &[Instruction], signers: &[&Keypair]) -> TxResult {
        let transaction = self.transaction(instructions, signers);
        self.svm.send_transaction(transaction).map_err(Box::new)
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
        let instruction = Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::InitializeConfig {
                upgrade_authority: authority.pubkey(),
                program_data: program_data_address(),
                config: config_address(environment),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::InitializeConfig {
                params: InitializeConfigParams {
                    environment,
                    administrator: self.administrator.pubkey(),
                    consumer: self.consumer.pubkey(),
                },
            }
            .data(),
        };
        self.send(&[instruction], &[authority])
    }

    pub fn register_key(
        &mut self,
        kid: &str,
        public_key: [u8; 33],
    ) -> Result<Pubkey, Box<FailedTransactionMetadata>> {
        let trusted_key = trusted_key_address(self.config, kid);
        let instruction = Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::RegisterTrustedKey {
                administrator: self.administrator.pubkey(),
                config: self.config,
                trusted_key,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::RegisterTrustedKey {
                params: RegisterTrustedKeyParams {
                    kid: kid.into(),
                    public_key,
                    not_before: KEY_NOT_BEFORE,
                    not_after: KEY_NOT_AFTER,
                },
            }
            .data(),
        };
        let administrator = self.administrator.insecure_clone();
        self.send(&[instruction], &[&administrator])?;
        Ok(trusted_key)
    }

    pub fn revoke_key(&mut self, trusted_key: Pubkey) -> TxResult {
        let instruction = Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::RevokeTrustedKey {
                administrator: self.administrator.pubkey(),
                config: self.config,
                trusted_key,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::RevokeTrustedKey {}.data(),
        };
        let administrator = self.administrator.insecure_clone();
        self.send(&[instruction], &[&administrator])
    }

    pub fn register_mandate(
        &mut self,
        params: RegisterMandateParams,
    ) -> Result<Pubkey, Box<FailedTransactionMetadata>> {
        let mandate = mandate_address(self.config, &params.mandate_hash);
        let instruction = Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::RegisterMandate {
                administrator: self.administrator.pubkey(),
                config: self.config,
                mandate,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::RegisterMandate { params }.data(),
        };
        let administrator = self.administrator.insecure_clone();
        self.send(&[instruction], &[&administrator])?;
        Ok(mandate)
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
        let consumer = self.consumer.insecure_clone();
        self.send(&instructions, &[&consumer])
    }

    pub fn sign(&self, claims: &VerdictClaims) -> SignedVerdict {
        SignedVerdict::sign(&self.signer, claims)
    }

    pub fn receipt_address(&self, nonce_hash: &[u8; 32]) -> Pubkey {
        Pubkey::find_program_address(
            &[RECEIPT_SEED, self.config.as_ref(), nonce_hash],
            &es256_authorization_verifier::ID,
        )
        .0
    }

    pub fn receipt(&self, nonce_hash: &[u8; 32]) -> Option<ConsumedAuthorization> {
        let account = self.svm.get_account(&self.receipt_address(nonce_hash))?;
        if account.lamports == 0 {
            return None;
        }
        Some(ConsumedAuthorization::try_deserialize(&mut account.data.as_slice()).unwrap())
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

pub fn mandate_address(config: Pubkey, mandate_hash: &[u8; 32]) -> Pubkey {
    Pubkey::find_program_address(
        &[MANDATE_SEED, config.as_ref(), mandate_hash],
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
fn program_data(upgrade_authority: Option<Pubkey>) -> Vec<u8> {
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

/// Reads the program built by `cargo build-sbf`.
fn program_bytes() -> Vec<u8> {
    let mut paths = Vec::new();
    if let Ok(directory) = std::env::var("SBF_OUT_DIR") {
        paths.push(format!("{directory}/es256_authorization_verifier.so"));
    }
    paths.push(format!(
        "{}/target/deploy/es256_authorization_verifier.so",
        env!("CARGO_MANIFEST_DIR")
    ));
    for path in &paths {
        if let Ok(bytes) = std::fs::read(path) {
            return bytes;
        }
    }
    panic!(
        "missing es256_authorization_verifier.so, looked in {}; run cargo-build-sbf first",
        paths.join(", ")
    );
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
