//! Runs the verifier's evidence flow against a program already deployed on
//! devnet: a Sandbox config, one synthetic trusted key and mandate, one
//! accepted allow verdict, then the same verdict replayed under a fresh
//! blockhash. Prints a JSON summary of everything it landed.
//!
//! The P-256 signing key is generated in memory and never written out.

use std::str::FromStr;
use std::time::{Duration, Instant};
use std::{env, thread};

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use es256_authorization_verifier::{
    CONFIG_SEED, ConsumedAuthorization, Environment, InitializeConfigParams, JTI_DOMAIN,
    MANDATE_SEED, RECEIPT_SEED, RegisterMandateParams, RegisterTrustedKeyParams, TRUSTED_KEY_SEED,
    VERDICT_LIFETIME_SECONDS, domain_hash, kid_seed, mandate_hash, nonce_hash,
};
use es256_jws_adapter::{SyntheticSigner, normalize_signature, secp256r1_instruction};
use serde_json::{Value, json};
use solana_hash::Hash;
use solana_instruction::Instruction;
use solana_keypair::{Keypair, read_keypair_file};
use solana_signer::Signer;
use solana_transaction::Transaction;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const DEVNET_RPC_URL: &str = "https://api.devnet.solana.com";
const DEVNET_GENESIS_HASH: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const CLOCK_SYSVAR: &str = "SysvarC1ock11111111111111111111111111111111";

const KID: &str = "sandbox-devnet-01";
const MANDATE_ID: &str = "mnd_devnet_01";
/// The signed `request.amount`; the mandate stores it scaled by 10^6.
const AMOUNT: &str = "340";
const AMOUNT_MICROS: u64 = 340_000_000;
const CURRENCY: &str = "USD";
const RECIPIENT: &str = "vendor:acme_saas";
const POLICY_VERSION: &str = "pol_2026-08-12.1";
const AGENT: &str = "agt_procurement_01";
const CATEGORY: &str = "saas";
const ISSUER: &str = "https://verdicts.saifuro.com";

/// Covers the receipt's rent and both consume fees with room to spare.
const CONSUMER_FUNDING: u64 = 20_000_000;
const KEY_VALIDITY_SECONDS: i64 = 30 * 86_400;
/// `SystemError::AccountAlreadyInUse`, raised when `init` meets the receipt.
const ACCOUNT_ALREADY_IN_USE: u64 = 0;
const FINALITY_TIMEOUT: Duration = Duration::from_secs(120);

fn main() -> Result<()> {
    let rpc = Rpc {
        url: env::var("RPC_URL").unwrap_or_else(|_| DEVNET_RPC_URL.into()),
    };
    let authority = read_keypair_file(required_env("AUTHORITY_KEYPAIR")?)?;
    let consumer = read_keypair_file(required_env("CONSUMER_KEYPAIR")?)?;
    if authority.pubkey() == consumer.pubkey() {
        return Err("the consumer must differ from the authority".into());
    }

    let genesis = rpc.call("getGenesisHash", json!([]))?;
    if genesis != DEVNET_GENESIS_HASH {
        return Err(format!("refusing to run: genesis {genesis} is not devnet").into());
    }

    let program_id = es256_authorization_verifier::ID;
    let program_data = pda(&[program_id.as_ref()], &bpf_loader_upgradeable::ID);
    check_deployment(&rpc, &program_id, &program_data, &authority.pubkey())?;

    let config = pda(&[CONFIG_SEED, &Environment::Sandbox.seed()], &program_id);
    if rpc.account(&config)?.is_some() {
        return Err(format!("Sandbox config {config} already exists").into());
    }
    let trusted_key = pda(
        &[TRUSTED_KEY_SEED, config.as_ref(), kid_seed(KID)],
        &program_id,
    );
    let mandate_params = RegisterMandateParams {
        mandate_hash: mandate_hash(MANDATE_ID.as_bytes()),
        mandate_id: MANDATE_ID.into(),
        amount_micros: AMOUNT_MICROS,
        currency: CURRENCY.into(),
        recipient: RECIPIENT.into(),
        policy_version: POLICY_VERSION.into(),
        agent: AGENT.into(),
        category: CATEGORY.into(),
    };
    let mandate = pda(
        &[MANDATE_SEED, config.as_ref(), &mandate_params.mandate_hash],
        &program_id,
    );

    let signer = SyntheticSigner::generate().map_err(|error| format!("{error:?}"))?;
    let public_key = signer.public_key().map_err(|error| format!("{error:?}"))?;
    let run_tag = hex(public_key.get(1..5).unwrap_or_default());
    let now = rpc.cluster_time()?;
    let key_params = RegisterTrustedKeyParams {
        kid: KID.into(),
        public_key,
        not_before: now - 3_600,
        not_after: now + KEY_VALIDITY_SECONDS,
    };

    let funding = rpc.execute(
        "fund consumer",
        &[solana_system_interface::instruction::transfer(
            &authority.pubkey(),
            &consumer.pubkey(),
            CONSUMER_FUNDING,
        )],
        &[&authority],
    )?;
    let initialize = rpc.execute(
        "initialize Sandbox config",
        &[Instruction {
            program_id,
            accounts: es256_authorization_verifier::accounts::InitializeConfig {
                upgrade_authority: authority.pubkey(),
                program_data,
                config,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::InitializeConfig {
                params: InitializeConfigParams {
                    environment: Environment::Sandbox,
                    administrator: authority.pubkey(),
                    consumer: consumer.pubkey(),
                },
            }
            .data(),
        }],
        &[&authority],
    )?;
    let register_key = rpc.execute(
        "register trusted key",
        &[Instruction {
            program_id,
            accounts: es256_authorization_verifier::accounts::RegisterTrustedKey {
                administrator: authority.pubkey(),
                config,
                trusted_key,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::RegisterTrustedKey {
                params: key_params.clone(),
            }
            .data(),
        }],
        &[&authority],
    )?;
    let register_mandate = rpc.execute(
        "register mandate",
        &[Instruction {
            program_id,
            accounts: es256_authorization_verifier::accounts::RegisterMandate {
                administrator: authority.pubkey(),
                config,
                mandate,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::RegisterMandate {
                params: mandate_params.clone(),
            }
            .data(),
        }],
        &[&authority],
    )?;

    // Issued against the cluster clock, which may differ from local time.
    let iat = rpc.cluster_time()?;
    let exp = iat + VERDICT_LIFETIME_SECONDS;
    let jti = format!("dec_devnet_{run_tag}");
    let nonce = format!("n_devnet_{run_tag}");
    let header_json = format!(r#"{{"alg":"ES256","typ":"JWT","kid":"{KID}"}}"#);
    let payload_json = format!(
        concat!(
            r#"{{"iss":"{ISSUER}","jti":"{jti}","iat":{iat},"exp":{exp},"verdict":"allow","#,
            r#""mandate":"{MANDATE_ID}","policy_version":"{POLICY_VERSION}","nonce":"{nonce}","#,
            r#""request":{{"agent":"{AGENT}","amount":{AMOUNT},"currency":"{CURRENCY}","#,
            r#""recipient":"{RECIPIENT}","category":"{CATEGORY}"}}}}"#,
        ),
        ISSUER = ISSUER,
        jti = jti,
        iat = iat,
        exp = exp,
        MANDATE_ID = MANDATE_ID,
        POLICY_VERSION = POLICY_VERSION,
        nonce = nonce,
        AGENT = AGENT,
        AMOUNT = AMOUNT,
        CURRENCY = CURRENCY,
        RECIPIENT = RECIPIENT,
        CATEGORY = CATEGORY,
    );
    let signing_input =
        es256_jws_adapter::signing_input(header_json.as_bytes(), payload_json.as_bytes());
    let signature = signer
        .sign(&signing_input)
        .and_then(|raw| normalize_signature(&raw))
        .map_err(|error| format!("{error:?}"))?;
    let nonce_hash = nonce_hash(nonce.as_bytes());
    let receipt = pda(&[RECEIPT_SEED, config.as_ref(), &nonce_hash], &program_id);
    let consume = [
        secp256r1_instruction(&public_key, &signature, &signing_input)
            .map_err(|error| format!("{error:?}"))?,
        Instruction {
            program_id,
            accounts: es256_authorization_verifier::accounts::ConsumeAuthorization {
                consumer: consumer.pubkey(),
                config,
                trusted_key,
                mandate,
                receipt,
                instructions_sysvar: solana_sdk_ids::sysvar::instructions::ID,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::ConsumeAuthorization { nonce_hash }
                .data(),
        },
    ];

    let accepted = rpc.execute("consume authorization", &consume, &[&consumer])?;
    let receipt_account = rpc
        .account(&receipt)?
        .ok_or("no receipt account after the accepted consume")?;
    let stored = ConsumedAuthorization::try_deserialize(&mut receipt_account.data.as_slice())?;
    let signing_input_hash = solana_sha256_hasher::hash(&signing_input).to_bytes();
    let jti_hash = domain_hash(JTI_DOMAIN, jti.as_bytes());
    let receipt_bump =
        Pubkey::find_program_address(&[RECEIPT_SEED, config.as_ref(), &nonce_hash], &program_id).1;
    let receipt_checks = [
        ("owner", receipt_account.owner == program_id.to_string()),
        ("config", stored.config == config),
        ("nonce_hash", stored.nonce_hash == nonce_hash),
        (
            "signing_input_hash",
            stored.signing_input_hash == signing_input_hash,
        ),
        ("jti_hash", stored.jti_hash == jti_hash),
        ("mandate", stored.mandate == mandate),
        ("expires_at", stored.expires_at == exp),
        (
            "consumed_at",
            iat - 60 <= stored.consumed_at && stored.consumed_at < exp,
        ),
        ("bump", stored.bump == receipt_bump),
    ];
    if let Some((field, _)) = receipt_checks.iter().find(|(_, ok)| !ok) {
        return Err(format!("receipt field {field} does not match the verdict").into());
    }

    let before = Snapshot::take(
        &rpc,
        &consumer.pubkey(),
        [config, trusted_key, mandate, receipt],
    )?;
    let replay_transaction = rpc.sign_with_new_blockhash(&consume, &[&consumer], &accepted)?;
    let simulation = rpc.simulate(&replay_transaction)?;
    let expected_error = json!({"InstructionError": [1, {"Custom": ACCOUNT_ALREADY_IN_USE}]});
    if simulation["err"] != expected_error {
        return Err(format!("replay simulation returned {}", simulation["err"]).into());
    }
    eprintln!("replay simulation rejected as expected; submitting it without preflight");
    // Preflight would drop the transaction; it is sent only to record the
    // rejection on chain.
    let replay = rpc.submit(&replay_transaction, true)?;
    if replay.error != expected_error {
        return Err(format!("replay landed with {}", replay.error).into());
    }
    let after = Snapshot::take(
        &rpc,
        &consumer.pubkey(),
        [config, trusted_key, mandate, receipt],
    )?;
    let accounts_unchanged = before.accounts == after.accounts;
    let consumer_delta = before
        .consumer_lamports
        .checked_sub(after.consumer_lamports);
    if !accounts_unchanged || consumer_delta != Some(replay.fee) {
        return Err("the rejected replay changed state beyond its fee".into());
    }

    let summary = json!({
        "network": "devnet",
        "rpc_url": rpc.url,
        "genesis_hash": genesis,
        "program_id": program_id.to_string(),
        "program_data": program_data.to_string(),
        "upgrade_authority": authority.pubkey().to_string(),
        "administrator": authority.pubkey().to_string(),
        "consumer": consumer.pubkey().to_string(),
        "config": {"address": config.to_string(), "environment": "Sandbox"},
        "trusted_key": {
            "address": trusted_key.to_string(),
            "kid": KID,
            "public_key_hex": hex(&public_key),
            "not_before": key_params.not_before,
            "not_after": key_params.not_after,
        },
        "mandate": {
            "address": mandate.to_string(),
            "mandate_hash_hex": hex(&mandate_params.mandate_hash),
            "mandate_id": MANDATE_ID,
            "amount_micros": AMOUNT_MICROS,
            "currency": CURRENCY,
            "recipient": RECIPIENT,
            "policy_version": POLICY_VERSION,
            "agent": AGENT,
            "category": CATEGORY,
        },
        "verdict": {
            "header": header_json,
            "payload": payload_json,
            "signing_input_len": signing_input.len(),
            "signature_hex": hex(&signature),
        },
        "transactions": {
            "fund_consumer": funding.to_json(),
            "initialize_config": initialize.to_json(),
            "register_trusted_key": register_key.to_json(),
            "register_mandate": register_mandate.to_json(),
            "consume": accepted.to_json(),
            "replay_simulation": {
                "err": simulation["err"],
                "units_consumed": simulation["unitsConsumed"],
                "logs": simulation["logs"],
            },
            "replay": replay.to_json(),
        },
        "receipt": {
            "address": receipt.to_string(),
            "owner": receipt_account.owner,
            "lamports": receipt_account.lamports,
            "data_len": receipt_account.data.len(),
            "config": stored.config.to_string(),
            "nonce_hash_hex": hex(&stored.nonce_hash),
            "signing_input_hash_hex": hex(&stored.signing_input_hash),
            "jti_hash_hex": hex(&stored.jti_hash),
            "mandate": stored.mandate.to_string(),
            "consumed_at": stored.consumed_at,
            "expires_at": stored.expires_at,
            "bump": stored.bump,
            "checks": receipt_checks.iter().map(|(field, _)| *field).collect::<Vec<_>>(),
        },
        "atomicity": {
            "accounts_compared": [config.to_string(), trusted_key.to_string(), mandate.to_string(), receipt.to_string()],
            "accounts_unchanged": accounts_unchanged,
            "receipt_data_sha256_before": hex(&solana_sha256_hasher::hash(&before.receipt_data()).to_bytes()),
            "receipt_data_sha256_after": hex(&solana_sha256_hasher::hash(&after.receipt_data()).to_bytes()),
            "consumer_lamports_before": before.consumer_lamports,
            "consumer_lamports_after": after.consumer_lamports,
            "replay_fee": replay.fee,
        },
    });
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn required_env(name: &str) -> Result<String> {
    env::var(name).map_err(|_| format!("set {name} to a keypair path").into())
}

fn pda(seeds: &[&[u8]], program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(seeds, program_id).0
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The program is deployed by the upgradeable loader under `authority`.
fn check_deployment(
    rpc: &Rpc,
    program_id: &Pubkey,
    program_data: &Pubkey,
    authority: &Pubkey,
) -> Result<()> {
    let program = rpc
        .account(program_id)?
        .ok_or_else(|| format!("program {program_id} is not deployed"))?;
    if !program.executable || program.owner != bpf_loader_upgradeable::ID.to_string() {
        return Err(format!("{program_id} is not an upgradeable program").into());
    }
    let data = rpc
        .account(program_data)?
        .ok_or("program data account is missing")?
        .data;
    // Variant 3, then the slot, then an optional authority.
    let recorded = match (data.get(..4), data.get(12), data.get(13..45)) {
        (Some([3, 0, 0, 0]), Some(1), Some(key)) => Pubkey::try_from(key).ok(),
        _ => None,
    };
    if recorded != Some(*authority) {
        return Err(format!("upgrade authority is not {authority}").into());
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct Account {
    lamports: u64,
    owner: String,
    executable: bool,
    data: Vec<u8>,
}

struct Snapshot {
    accounts: Vec<Option<Account>>,
    consumer_lamports: u64,
}

impl Snapshot {
    /// The receipt must be the last of `accounts`.
    fn take(rpc: &Rpc, consumer: &Pubkey, accounts: [Pubkey; 4]) -> Result<Self> {
        Ok(Self {
            accounts: accounts
                .iter()
                .map(|address| rpc.account(address))
                .collect::<Result<_>>()?,
            consumer_lamports: rpc.balance(consumer)?,
        })
    }

    fn receipt_data(&self) -> Vec<u8> {
        match self.accounts.last() {
            Some(Some(account)) => account.data.clone(),
            _ => Vec::new(),
        }
    }
}

/// A transaction observed at finalized commitment.
struct Landed {
    signature: String,
    blockhash: Hash,
    slot: u64,
    block_time: Value,
    fee: u64,
    compute_units: Value,
    error: Value,
    size: usize,
}

impl Landed {
    fn to_json(&self) -> Value {
        json!({
            "signature": self.signature,
            "explorer": format!("https://explorer.solana.com/tx/{}?cluster=devnet", self.signature),
            "slot": self.slot,
            "block_time": self.block_time,
            "fee": self.fee,
            "compute_units": self.compute_units,
            "err": self.error,
            "size_bytes": self.size,
            "commitment": "finalized",
        })
    }
}

struct Rpc {
    url: String,
}

impl Rpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let body: Value = ureq::post(&self.url)
            .send_json(&request)?
            .body_mut()
            .read_json()?;
        match body.get("error") {
            Some(error) => Err(format!("{method}: {error}").into()),
            None => Ok(body.get("result").cloned().unwrap_or(Value::Null)),
        }
    }

    fn account(&self, address: &Pubkey) -> Result<Option<Account>> {
        let result = self.call(
            "getAccountInfo",
            json!([address.to_string(), {"encoding": "base64", "commitment": "finalized"}]),
        )?;
        let value = &result["value"];
        if value.is_null() {
            return Ok(None);
        }
        Ok(Some(Account {
            lamports: value["lamports"].as_u64().ok_or("missing lamports")?,
            owner: value["owner"].as_str().ok_or("missing owner")?.into(),
            executable: value["executable"].as_bool().unwrap_or(false),
            data: STANDARD.decode(value["data"][0].as_str().ok_or("missing data")?)?,
        }))
    }

    fn balance(&self, address: &Pubkey) -> Result<u64> {
        let result = self.call(
            "getBalance",
            json!([address.to_string(), {"commitment": "finalized"}]),
        )?;
        Ok(result["value"].as_u64().ok_or("missing balance")?)
    }

    /// `Clock::unix_timestamp` at finalized commitment, which trails the
    /// time a transaction will execute at.
    fn cluster_time(&self) -> Result<i64> {
        let address = Pubkey::from_str(CLOCK_SYSVAR)?;
        let data = self.account(&address)?.ok_or("clock sysvar missing")?.data;
        let bytes = data.get(32..40).ok_or("clock sysvar too short")?;
        Ok(i64::from_le_bytes(<[u8; 8]>::try_from(bytes)?))
    }

    fn latest_blockhash(&self) -> Result<Hash> {
        let result = self.call("getLatestBlockhash", json!([{"commitment": "finalized"}]))?;
        Ok(Hash::from_str(
            result["value"]["blockhash"]
                .as_str()
                .ok_or("missing blockhash")?,
        )?)
    }

    fn sign(&self, instructions: &[Instruction], signers: &[&Keypair]) -> Result<Transaction> {
        let payer = signers.first().ok_or("no fee payer")?.pubkey();
        Ok(Transaction::new_signed_with_payer(
            instructions,
            Some(&payer),
            signers,
            self.latest_blockhash()?,
        ))
    }

    /// Signs under a blockhash other than `previous`'s, so the transaction
    /// is new rather than a duplicate the cluster would drop.
    fn sign_with_new_blockhash(
        &self,
        instructions: &[Instruction],
        signers: &[&Keypair],
        previous: &Landed,
    ) -> Result<Transaction> {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            let transaction = self.sign(instructions, signers)?;
            if transaction.message.recent_blockhash != previous.blockhash {
                return Ok(transaction);
            }
            thread::sleep(Duration::from_secs(2));
        }
        Err("no new blockhash within a minute".into())
    }

    fn simulate(&self, transaction: &Transaction) -> Result<Value> {
        let result = self.call(
            "simulateTransaction",
            json!([encode(transaction)?, {"encoding": "base64", "sigVerify": true, "commitment": "finalized"}]),
        )?;
        Ok(result["value"].clone())
    }

    /// Simulates, sends and waits for finality, failing on any error.
    fn execute(
        &self,
        label: &str,
        instructions: &[Instruction],
        signers: &[&Keypair],
    ) -> Result<Landed> {
        let transaction = self.sign(instructions, signers)?;
        let simulation = self.simulate(&transaction)?;
        if !simulation["err"].is_null() {
            return Err(format!(
                "{label}: simulation failed with {} {}",
                simulation["err"], simulation["logs"]
            )
            .into());
        }
        let landed = self.submit(&transaction, false)?;
        if !landed.error.is_null() {
            return Err(format!("{label}: landed with {}", landed.error).into());
        }
        eprintln!("{label}: {} (slot {})", landed.signature, landed.slot);
        Ok(landed)
    }

    /// Sends and waits until the transaction is finalized, whatever its
    /// outcome.
    fn submit(&self, transaction: &Transaction, skip_preflight: bool) -> Result<Landed> {
        let signature = self
            .call(
                "sendTransaction",
                json!([encode(transaction)?, {
                    "encoding": "base64",
                    "skipPreflight": skip_preflight,
                    "preflightCommitment": "finalized",
                }]),
            )?
            .as_str()
            .ok_or("sendTransaction returned no signature")?
            .to_owned();
        let deadline = Instant::now() + FINALITY_TIMEOUT;
        while Instant::now() < deadline {
            thread::sleep(Duration::from_secs(3));
            let found = self.call(
                "getTransaction",
                json!([signature, {"encoding": "json", "commitment": "finalized", "maxSupportedTransactionVersion": 0}]),
            )?;
            if found.is_null() {
                continue;
            }
            let meta = &found["meta"];
            return Ok(Landed {
                signature,
                blockhash: transaction.message.recent_blockhash,
                slot: found["slot"].as_u64().ok_or("missing slot")?,
                block_time: found["blockTime"].clone(),
                fee: meta["fee"].as_u64().ok_or("missing fee")?,
                compute_units: meta["computeUnitsConsumed"].clone(),
                error: meta["err"].clone(),
                size: bincode::serialize(transaction)?.len(),
            });
        }
        Err(format!("{signature} was not finalized within {FINALITY_TIMEOUT:?}").into())
    }
}

fn encode(transaction: &Transaction) -> Result<String> {
    Ok(STANDARD.encode(bincode::serialize(transaction)?))
}
