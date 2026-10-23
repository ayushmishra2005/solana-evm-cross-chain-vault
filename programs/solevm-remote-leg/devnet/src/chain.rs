//! The few JSON-RPC calls the driver makes, retried through rate limits.

use std::str::FromStr;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anchor_lang::prelude::Pubkey;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use solana_hash::Hash;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::Res;

pub(crate) const DEVNET_GENESIS: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
pub(crate) const TESTNET_GENESIS: &str = "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY";
pub(crate) const MAINNET_GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";

const ATTEMPTS: u32 = 8;
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(90);
const REBROADCAST_EVERY: Duration = Duration::from_secs(8);

pub(crate) struct Chain {
    url: String,
    agent: ureq::Agent,
}

/// An account as the RPC returns it.
pub(crate) struct Fetched {
    pub owner: Pubkey,
    pub executable: bool,
    pub data: Vec<u8>,
}

/// A transaction the cluster executed, successfully or not.
#[derive(Clone, Debug)]
pub(crate) struct Landed {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub error: Option<Value>,
    pub logs: Vec<String>,
    pub fee: Option<u64>,
    pub compute_units: Option<u64>,
    /// Programs the transaction invoked at the top level.
    pub programs: Vec<String>,
}

impl Landed {
    /// The custom program error code, when the transaction failed with one.
    pub(crate) fn custom_code(&self) -> Option<u32> {
        let code = self
            .error
            .as_ref()?
            .pointer("/InstructionError/1/Custom")?
            .as_u64()?;
        u32::try_from(code).ok()
    }

    /// The error name Anchor logged, when the transaction failed with one.
    pub(crate) fn logged_error(&self) -> Option<String> {
        self.logs.iter().find_map(|line| {
            let rest = line.split("Error Code: ").nth(1)?;
            rest.split('.').next().map(str::to_owned)
        })
    }
}

impl Chain {
    pub(crate) fn new(url: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            url: url.to_owned(),
            agent,
        }
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    fn call(&self, method: &str, params: Value) -> Res<Value> {
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let mut delay = Duration::from_millis(500);
        let mut last = String::new();
        for _ in 0..ATTEMPTS {
            match self.agent.post(&self.url).send_json(&request) {
                Ok(mut response) => {
                    let status = response.status().as_u16();
                    if status == 429 || status >= 500 {
                        last = format!("HTTP {status}");
                    } else {
                        let body: Value = response.body_mut().read_json()?;
                        if let Some(error) = body.get("error") {
                            return Err(format!("{method}: {error}").into());
                        }
                        return body
                            .get("result")
                            .cloned()
                            .ok_or_else(|| format!("{method}: response has no result").into());
                    }
                }
                Err(error) => last = error.to_string(),
            }
            sleep(delay);
            delay = delay.saturating_mul(2).min(Duration::from_secs(8));
        }
        Err(format!("{method}: {last} after {ATTEMPTS} attempts").into())
    }

    pub(crate) fn genesis_hash(&self) -> Res<String> {
        text(&self.call("getGenesisHash", json!([]))?)
    }

    pub(crate) fn latest_blockhash(&self) -> Res<Hash> {
        let result = self.call("getLatestBlockhash", json!([{ "commitment": "confirmed" }]))?;
        let value = result
            .pointer("/value/blockhash")
            .and_then(Value::as_str)
            .ok_or("getLatestBlockhash: no blockhash")?;
        Hash::from_str(value).map_err(|error| format!("blockhash {value}: {error:?}").into())
    }

    pub(crate) fn rent_exempt_minimum(&self, len: usize) -> Res<u64> {
        self.call("getMinimumBalanceForRentExemption", json!([len]))?
            .as_u64()
            .ok_or_else(|| "getMinimumBalanceForRentExemption: not a number".into())
    }

    pub(crate) fn balance(&self, key: &Pubkey) -> Res<u64> {
        self.call(
            "getBalance",
            json!([key.to_string(), { "commitment": "confirmed" }]),
        )?
        .pointer("/value")
        .and_then(Value::as_u64)
        .ok_or_else(|| "getBalance: no value".into())
    }

    pub(crate) fn account(&self, key: &Pubkey, commitment: &str) -> Res<Option<Fetched>> {
        let config = json!({ "encoding": "base64", "commitment": commitment });
        let result = self.call("getAccountInfo", json!([key.to_string(), config]))?;
        fetched(&result["value"])
    }

    /// Reads many accounts at one slot, no earlier than `min_slot`.
    pub(crate) fn accounts(
        &self,
        keys: &[Pubkey],
        min_slot: Option<u64>,
    ) -> Res<(u64, Vec<Option<Fetched>>)> {
        let mut config = json!({ "encoding": "base64", "commitment": "confirmed" });
        if let Some(slot) = min_slot {
            config["minContextSlot"] = json!(slot);
        }
        let names: Vec<String> = keys.iter().map(ToString::to_string).collect();
        let result = self.call("getMultipleAccounts", json!([names, config]))?;
        let slot = result
            .pointer("/context/slot")
            .and_then(Value::as_u64)
            .ok_or("getMultipleAccounts: no context slot")?;
        let values = result["value"]
            .as_array()
            .ok_or("getMultipleAccounts: no value")?
            .iter()
            .map(fetched)
            .collect::<Res<Vec<_>>>()?;
        Ok((slot, values))
    }

    fn send(&self, transaction: &Transaction, skip_preflight: bool) -> Res<String> {
        let wire =
            wincode::serialize(transaction).map_err(|error| format!("serialize: {error:?}"))?;
        let config = json!({
            "encoding": "base64",
            "skipPreflight": skip_preflight,
            "preflightCommitment": "confirmed",
            "maxRetries": 0,
        });
        text(&self.call("sendTransaction", json!([BASE64.encode(wire), config]))?)
    }

    fn statuses(&self, signatures: &[String]) -> Res<Vec<Value>> {
        let mut all = Vec::with_capacity(signatures.len());
        for chunk in signatures.chunks(200) {
            let result = self.call(
                "getSignatureStatuses",
                json!([chunk, { "searchTransactionHistory": true }]),
            )?;
            let values = result["value"]
                .as_array()
                .ok_or("getSignatureStatuses: no value")?;
            all.extend(values.iter().cloned());
        }
        Ok(all)
    }

    /// Waits until every signature reaches `level` ("confirmed" or "finalized").
    pub(crate) fn wait_all(
        &self,
        signatures: &[String],
        level: &str,
        timeout: Duration,
    ) -> Res<()> {
        let started = Instant::now();
        loop {
            let statuses = self.statuses(signatures)?;
            let pending = statuses
                .iter()
                .filter(|status| !reached(status, level))
                .count();
            if pending == 0 {
                return Ok(());
            }
            if started.elapsed() > timeout {
                return Err(format!("{pending} transactions did not reach {level}").into());
            }
            sleep(Duration::from_secs(2));
        }
    }

    pub(crate) fn transaction(&self, signature: &str, commitment: &str) -> Res<Option<Landed>> {
        let config = json!({
            "encoding": "json",
            "commitment": commitment,
            "maxSupportedTransactionVersion": 0,
        });
        let result = self.call("getTransaction", json!([signature, config]))?;
        if result.is_null() {
            return Ok(None);
        }
        let keys: Vec<&str> = result
            .pointer("/transaction/message/accountKeys")
            .and_then(Value::as_array)
            .map(|keys| keys.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let programs = result
            .pointer("/transaction/message/instructions")
            .and_then(Value::as_array)
            .map(|instructions| {
                instructions
                    .iter()
                    .filter_map(|instruction| instruction["programIdIndex"].as_u64())
                    .filter_map(|index| usize::try_from(index).ok())
                    .filter_map(|index| keys.get(index).map(|key| (*key).to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(Landed {
            signature: signature.to_owned(),
            slot: result["slot"].as_u64().ok_or("getTransaction: no slot")?,
            block_time: result["blockTime"].as_i64(),
            error: result
                .pointer("/meta/err")
                .filter(|error| !error.is_null())
                .cloned(),
            logs: result
                .pointer("/meta/logMessages")
                .and_then(Value::as_array)
                .map(|lines| {
                    lines
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            fee: result.pointer("/meta/fee").and_then(Value::as_u64),
            compute_units: result
                .pointer("/meta/computeUnitsConsumed")
                .and_then(Value::as_u64),
            programs,
        }))
    }

    /// Signs, sends and waits until the transaction is confirmed.
    ///
    /// Expected failures skip preflight, so they land on chain with a
    /// signature instead of being refused by the RPC node.
    pub(crate) fn execute(
        &self,
        instructions: &[Instruction],
        payer: &Keypair,
        signers: &[&Keypair],
        expect_failure: bool,
    ) -> Res<Landed> {
        let mut keys: Vec<&Keypair> = vec![payer];
        for signer in signers {
            if keys.iter().all(|known| known.pubkey() != signer.pubkey()) {
                keys.push(signer);
            }
        }
        let blockhash = self.latest_blockhash()?;
        let transaction = Transaction::new_signed_with_payer(
            instructions,
            Some(&payer.pubkey()),
            keys.as_slice(),
            blockhash,
        );
        let signature = transaction
            .signatures
            .first()
            .map(ToString::to_string)
            .ok_or("transaction has no signature")?;

        let returned = self.send(&transaction, expect_failure)?;
        if returned != signature {
            return Err(format!("RPC returned signature {returned}, expected {signature}").into());
        }

        let started = Instant::now();
        let mut broadcast = Instant::now();
        let wanted = std::slice::from_ref(&signature);
        loop {
            let status = self
                .statuses(wanted)?
                .into_iter()
                .next()
                .unwrap_or(Value::Null);
            if reached(&status, "confirmed") {
                break;
            }
            if started.elapsed() > CONFIRM_TIMEOUT {
                return Err(format!("{signature} was not confirmed in time").into());
            }
            if broadcast.elapsed() > REBROADCAST_EVERY {
                // A dropped transaction is resent unchanged; duplicates are ignored.
                let _ = self.send(&transaction, true);
                broadcast = Instant::now();
            }
            sleep(Duration::from_secs(1));
        }

        for _ in 0..ATTEMPTS {
            if let Some(landed) = self.transaction(&signature, "confirmed")? {
                return Ok(landed);
            }
            sleep(Duration::from_secs(1));
        }
        Err(format!("{signature} is confirmed but getTransaction does not return it").into())
    }
}

fn reached(status: &Value, level: &str) -> bool {
    match status.get("confirmationStatus").and_then(Value::as_str) {
        Some("finalized") => true,
        Some("confirmed") => level == "confirmed",
        _ => false,
    }
}

fn fetched(value: &Value) -> Res<Option<Fetched>> {
    if value.is_null() {
        return Ok(None);
    }
    let data = value
        .pointer("/data/0")
        .and_then(Value::as_str)
        .ok_or("account has no base64 data")?;
    let owner = value["owner"].as_str().ok_or("account has no owner")?;
    Ok(Some(Fetched {
        owner: Pubkey::from_str(owner).map_err(|error| format!("owner {owner}: {error:?}"))?,
        executable: value["executable"].as_bool().unwrap_or(false),
        data: BASE64.decode(data)?,
    }))
}

fn text(value: &Value) -> Res<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("expected a string, found {value}").into())
}
