//! The V2 evidence record, and its independent re-verification against the
//! RPC it names. Verification reads public data only and needs no key.
//!
//! A record says which cluster produced it. A localnet record is a
//! rehearsal on a throwaway validator: once that validator stops, nothing in
//! it can be re-checked, and it is never devnet validation.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use anchor_lang::AccountDeserialize;
use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use es256_authorization_verifier::{DeploymentAuthority, DeploymentReceipt, RiskKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use solevm_remote_leg::DeployAuthorizationPolicy;

use crate::Res;
use crate::chain::Chain;
use crate::evidence::{Build, Check, Mint, Program, Source, Summary, read_program, sha256_hex};
use crate::time::utc;

/// Bumped whenever the record layout changes. V1 records are schema 1.
pub(crate) const SCHEMA: u32 = 2;
pub(crate) const KIND: &str = "solevm-v2-deployment-authorization";

#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct Evidence {
    pub schema: u32,
    pub kind: String,
    /// `localnet` or `devnet`.
    pub cluster: String,
    /// What the record may be cited as.
    pub validation: String,
    pub rpc_url: String,
    pub genesis_hash: String,
    pub node_version: Value,
    pub features: Vec<Feature>,
    /// Localnet only: the devnet feature set the validator was started to
    /// match, and the gates it deactivated to do so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub localnet_feature_mirror: Option<Value>,
    pub run_label: String,
    pub started_at_utc: String,
    pub completed_at_utc: String,
    pub source: Source,
    pub build: Build,
    pub programs: Vec<Program>,
    pub mint: Mint,
    /// Upgrade authorities and signer roles, public keys only.
    pub authorities: BTreeMap<String, String>,
    pub namespace: BTreeMap<String, String>,
    pub strategy: Value,
    pub authorization: Authorization,
    pub steps: Vec<Step>,
    pub measurements: Vec<Measurement>,
    pub balances: Balances,
    pub final_state: Value,
    pub summary: Summary,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub(crate) struct Feature {
    pub id: String,
    pub name: String,
    pub activated_at_slot: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct Authorization {
    pub deployment_authority: String,
    pub administrator: String,
    pub network_domain_hex: String,
    /// The network domain is the cluster's genesis hash.
    pub network_domain_is_genesis: bool,
    pub policy_generation: u64,
    pub risk_key_account: String,
    /// Compressed SEC1 P-256 public key; the private half was never stored.
    pub risk_public_key_hex: String,
    pub risk_key_not_before: i64,
    pub risk_key_not_after: i64,
    pub authorization_policy: String,
    pub adopted_at: i64,
    pub receipts: Vec<Receipt>,
}

/// One spent approval. Receipts are permanent and never change after the
/// consume that creates them, so their bytes can be re-checked later.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub(crate) struct Receipt {
    pub approval: String,
    pub step: String,
    pub address: String,
    pub nonce_hex: String,
    pub amount: u64,
    pub issued_at: i64,
    pub expires_at: i64,
    pub consumed_at: i64,
    pub message_sha256: String,
    pub lamports: u64,
    pub data_len: usize,
    pub data_sha256: String,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub(crate) struct Step {
    pub id: String,
    pub title: String,
    pub program: String,
    pub instruction: String,
    pub expected: String,
    pub outcome: String,
    /// For an expected failure: the instruction that must fail, and the
    /// exact error it must fail with, as the RPC encodes it.
    pub expected_error: Option<ExpectedError>,
    /// The transaction error as the cluster recorded it.
    pub observed_error: Option<Value>,
    pub failed_instruction: Option<u8>,
    pub error_code: Option<u32>,
    /// The node's simulation of the same signed bytes, made before an
    /// expected failure was submitted. A prediction, not evidence.
    pub simulation: Option<Simulation>,
    pub signature: String,
    pub slot: u64,
    pub block_time_utc: String,
    pub confirmation: String,
    pub compute_units: Option<u64>,
    pub fee_lamports: Option<u64>,
    pub transaction_version: String,
    pub transaction_bytes: usize,
    /// Top-level programs in instruction order.
    pub instructions: Vec<String>,
    pub explorer_url: String,
    pub accounts: BTreeMap<String, String>,
    pub before: Value,
    pub after: Value,
    pub details: Value,
    pub checks: Vec<Check>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub(crate) struct ExpectedError {
    pub instruction_index: u8,
    pub error: Value,
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub(crate) struct Simulation {
    pub error: Option<Value>,
    pub compute_units: Option<u64>,
    pub matched_expectation: bool,
}

/// Size and cost of one authorized deployment transaction.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub(crate) struct Measurement {
    pub step: String,
    pub outcome: String,
    pub transaction_version: String,
    pub transaction_bytes: usize,
    pub packet_limit_bytes: usize,
    pub instructions: usize,
    pub account_keys: usize,
    pub compute_units: Option<u64>,
    pub verifier_compute_units: Option<u64>,
    pub leg_compute_units: Option<u64>,
    pub fee_lamports: Option<u64>,
    pub receipt_rent_lamports: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct Balances {
    /// Lamports per signer role at the start and end of the run.
    pub before: BTreeMap<String, u64>,
    pub after: BTreeMap<String, u64>,
    /// What the run cost the deployer, net of the lamports still held by
    /// the roles it funded.
    pub net_spent_lamports: Option<u64>,
}

/// Re-fetches every recorded program, transaction and permanent verifier
/// account, and compares each with the record.
pub(crate) fn verify(path: &Path) -> Res<()> {
    let evidence: Evidence = serde_json::from_str(&fs::read_to_string(path)?)?;
    if evidence.schema != SCHEMA || evidence.kind != KIND {
        return Err(format!(
            "{} is not a V2 record (schema {}, kind '{}'); V1 records use `verify`",
            path.display(),
            evidence.schema,
            evidence.kind
        )
        .into());
    }
    let url = std::env::var("SOLEVM_RPC_URL").unwrap_or_else(|_| evidence.rpc_url.clone());
    let chain = Chain::new(&url);
    let mut problems = Vec::new();
    println!(
        "{} record '{}' ({})",
        evidence.cluster, evidence.run_label, evidence.validation
    );

    let genesis = chain.genesis_hash()?;
    if genesis != evidence.genesis_hash {
        return Err(format!(
            "{url} has genesis {genesis}, but the record was made on {}",
            evidence.genesis_hash
        )
        .into());
    }

    for program in &evidence.programs {
        let id = key(&program.program_id)?;
        match read_program(&chain, &id, usize::try_from(program.binary_len)?) {
            Ok(found) => {
                let digest = sha256_hex(&found.code);
                let authority = found
                    .upgrade_authority
                    .map(|key| key.to_string())
                    .unwrap_or_default();
                let matches = found.owner == bpf_loader_upgradeable::ID
                    && found.executable
                    && digest == program.onchain_sha256
                    && digest == program.local_sha256
                    && authority == program.upgrade_authority;
                println!(
                    "program {:<30} {} sha256 {} {}",
                    program.name,
                    program.program_id,
                    digest,
                    if matches { "OK" } else { "MISMATCH" }
                );
                if !matches {
                    problems.push(format!("program {} differs from the record", program.name));
                }
            }
            Err(error) => problems.push(format!("program {}: {error}", program.name)),
        }
    }

    for step in &evidence.steps {
        verify_step(&chain, step, &mut problems)?;
    }
    verify_authorization(&chain, &evidence, &mut problems)?;

    if problems.is_empty() {
        println!(
            "verified {} programs, {} transactions and {} receipts at finalized commitment against {url}",
            evidence.programs.len(),
            evidence.steps.len(),
            evidence.authorization.receipts.len()
        );
        Ok(())
    } else {
        for problem in &problems {
            eprintln!("MISMATCH: {problem}");
        }
        Err(format!("{} mismatches", problems.len()).into())
    }
}

fn verify_step(chain: &Chain, step: &Step, problems: &mut Vec<String>) -> Res<()> {
    let Some(landed) = chain.transaction(&step.signature, "finalized")? else {
        problems.push(format!(
            "{} {}: not found at finalized commitment",
            step.id, step.signature
        ));
        println!("{:<5} {} NOT FOUND", step.id, step.signature);
        return Ok(());
    };
    let mut ok = landed.slot == step.slot
        && landed.programs == step.instructions
        && landed.error == step.observed_error;
    match &step.expected_error {
        None => ok = ok && landed.error.is_none(),
        Some(expected) => {
            let wanted = serde_json::json!({
                "InstructionError": [expected.instruction_index, expected.error]
            });
            ok = ok && landed.error.as_ref() == Some(&wanted);
        }
    }
    let status = landed.error.as_ref().map_or_else(
        || String::from("success"),
        |error| format!("failed {error}"),
    );
    println!(
        "{:<5} {} slot {} {} {status} {}",
        step.id,
        step.signature,
        landed.slot,
        landed.block_time.map_or_else(String::new, utc),
        if ok { "OK" } else { "MISMATCH" }
    );
    if !ok {
        problems.push(format!(
            "{} {}: on-chain result differs from the record",
            step.id, step.signature
        ));
    }
    Ok(())
}

fn verify_authorization(chain: &Chain, evidence: &Evidence, problems: &mut Vec<String>) -> Res<()> {
    let record = &evidence.authorization;
    let verifier = evidence
        .programs
        .iter()
        .find(|program| program.name == "es256_authorization_verifier")
        .ok_or("the record names no verifier program")?;
    let verifier = key(&verifier.program_id)?;
    let leg = evidence
        .programs
        .iter()
        .find(|program| program.name == "solevm_remote_leg")
        .ok_or("the record names no leg program")?;
    let leg = key(&leg.program_id)?;

    let authority: Option<DeploymentAuthority> =
        load(chain, &key(&record.deployment_authority)?, &verifier)?;
    let genesis = solana_hash::Hash::from_str(&evidence.genesis_hash)
        .map_err(|error| format!("genesis: {error:?}"))?
        .to_bytes();
    let ok = authority.is_some_and(|authority| {
        authority.network_domain == genesis
            && crate::evidence::hex(&authority.network_domain) == record.network_domain_hex
            && authority.administrator.to_string() == record.administrator
            && authority.remote_leg_program == leg
            && evidence.namespace.get("remote_config") == Some(&authority.remote_config.to_string())
    });
    report(
        problems,
        "deployment authority binds this leg, namespace and genesis",
        ok,
    );

    let risk_key: Option<RiskKey> = load(chain, &key(&record.risk_key_account)?, &verifier)?;
    let ok = risk_key.is_some_and(|risk_key| {
        crate::evidence::hex(&risk_key.public_key) == record.risk_public_key_hex
            && risk_key.config.to_string() == record.deployment_authority
            && risk_key.not_before == record.risk_key_not_before
            && risk_key.not_after == record.risk_key_not_after
    });
    report(
        problems,
        "risk key holds the recorded public key and window",
        ok,
    );

    let policy: Option<DeployAuthorizationPolicy> =
        load(chain, &key(&record.authorization_policy)?, &leg)?;
    let ok = policy.is_some_and(|policy| {
        policy.authorization_config.to_string() == record.deployment_authority
            && policy.adopted_at == record.adopted_at
    });
    report(problems, "leg policy adopted this deployment authority", ok);

    for receipt in &record.receipts {
        let address = key(&receipt.address)?;
        let account = chain.account(&address, "finalized")?;
        let ok = account.is_some_and(|account| {
            account.owner == verifier
                && sha256_hex(&account.data) == receipt.data_sha256
                && DeploymentReceipt::try_deserialize(&mut account.data.as_slice()).is_ok_and(
                    |stored| {
                        stored.amount == receipt.amount
                            && stored.consumed_at == receipt.consumed_at
                            && crate::evidence::hex(&stored.nonce) == receipt.nonce_hex
                            && crate::evidence::hex(&stored.message_hash) == receipt.message_sha256
                    },
                )
        });
        report(
            problems,
            &format!(
                "receipt {} ({}) is unchanged",
                receipt.approval, receipt.address
            ),
            ok,
        );
    }
    Ok(())
}

fn report(problems: &mut Vec<String>, what: &str, ok: bool) {
    println!("{what}: {}", if ok { "OK" } else { "MISMATCH" });
    if !ok {
        problems.push(what.to_owned());
    }
}

fn load<T: AccountDeserialize>(chain: &Chain, address: &Pubkey, owner: &Pubkey) -> Res<Option<T>> {
    let Some(account) = chain.account(address, "finalized")? else {
        return Ok(None);
    };
    if account.owner != *owner {
        return Ok(None);
    }
    Ok(T::try_deserialize(&mut account.data.as_slice()).ok())
}

fn key(text: &str) -> Res<Pubkey> {
    Pubkey::from_str(text).map_err(|error| format!("{text}: {error:?}").into())
}
