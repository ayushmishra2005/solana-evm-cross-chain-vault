//! Evidence records, and their independent re-verification against the RPC.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::Res;
use crate::chain::Chain;
use crate::time::utc;

/// Bumped whenever the record layout changes.
pub(crate) const SCHEMA: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct Evidence {
    pub schema: u32,
    pub cluster: String,
    pub rpc_url: String,
    pub genesis_hash: String,
    pub run_label: String,
    pub started_at_utc: String,
    pub completed_at_utc: String,
    pub source: Source,
    pub build: Build,
    pub programs: Vec<Program>,
    pub mint: Mint,
    pub accounts: BTreeMap<String, String>,
    pub steps: Vec<Step>,
    pub final_state: Value,
    pub summary: Summary,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct Source {
    pub commit: String,
    /// Git tree of the working copy the binaries were built from.
    pub tree: String,
    pub dirty: bool,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct Build {
    pub toolchain: String,
    pub arch: String,
    pub rustflags: String,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct Program {
    pub name: String,
    pub program_id: String,
    pub owner: String,
    pub executable: bool,
    pub program_data: String,
    pub upgrade_authority: String,
    pub last_deploy_slot: u64,
    pub binary_len: u64,
    pub local_sha256: String,
    pub onchain_sha256: String,
    pub deploy_signature: Option<String>,
    pub deploy_slot: Option<u64>,
    pub deploy_time_utc: Option<String>,
    pub explorer_url: String,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct Mint {
    pub address: String,
    pub decimals: u8,
    pub mint_authority: String,
    pub label: String,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub(crate) struct Step {
    pub id: String,
    pub title: String,
    pub program: String,
    pub instruction: String,
    pub expected: String,
    pub outcome: String,
    pub error_code: Option<u32>,
    pub signature: String,
    pub slot: u64,
    pub block_time_utc: String,
    pub confirmation: String,
    pub compute_units: Option<u64>,
    pub fee_lamports: Option<u64>,
    pub explorer_url: String,
    pub accounts: BTreeMap<String, String>,
    pub before: Value,
    pub after: Value,
    pub details: Value,
    pub checks: Vec<Check>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct Check {
    pub what: String,
    pub passed: bool,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct Summary {
    pub transactions: usize,
    pub expected_successes: usize,
    pub expected_failures: usize,
    pub checks: usize,
    pub checks_passed: usize,
    pub all_finalized: bool,
    pub completed: bool,
}

/// What the loader stores for one upgradeable program.
pub(crate) struct OnChainProgram {
    pub owner: Pubkey,
    pub executable: bool,
    pub program_data: Pubkey,
    pub upgrade_authority: Option<Pubkey>,
    pub last_deploy_slot: u64,
    /// The deployed bytes, without the loader header or trailing padding.
    pub code: Vec<u8>,
}

/// Bytes of the loader's ProgramData header before the program bytes.
const PROGRAM_DATA_HEADER: usize = 45;

/// Reads an upgradeable program and trims its bytes to `binary_len`.
pub(crate) fn read_program(
    chain: &Chain,
    program_id: &Pubkey,
    binary_len: usize,
) -> Res<OnChainProgram> {
    let program = chain
        .account(program_id, "finalized")?
        .ok_or_else(|| format!("program {program_id} is not deployed"))?;
    let tag = program
        .data
        .get(0..4)
        .ok_or("program account is too short")?;
    if tag != 2u32.to_le_bytes() {
        return Err(format!("{program_id} is not an upgradeable program account").into());
    }
    let program_data = Pubkey::try_from(
        program
            .data
            .get(4..36)
            .ok_or("program account is too short")?,
    )
    .map_err(|error| format!("program data address: {error:?}"))?;

    let data = chain
        .account(&program_data, "finalized")?
        .ok_or("program data account is missing")?
        .data;
    let header = data
        .get(..PROGRAM_DATA_HEADER)
        .ok_or("program data is too short")?;
    if header.get(0..4) != Some(3u32.to_le_bytes().as_slice()) {
        return Err("program data account has the wrong loader tag".into());
    }
    let mut slot = [0u8; 8];
    slot.copy_from_slice(header.get(4..12).ok_or("program data is too short")?);
    let upgrade_authority = match header.get(12) {
        Some(1) => Some(
            Pubkey::try_from(header.get(13..45).ok_or("program data is too short")?)
                .map_err(|error| format!("upgrade authority: {error:?}"))?,
        ),
        _ => None,
    };
    let end = PROGRAM_DATA_HEADER
        .checked_add(binary_len)
        .ok_or("binary length overflows")?;
    let code = data
        .get(PROGRAM_DATA_HEADER..end)
        .ok_or("program data is shorter than the binary")?;
    let padding = data.get(end..).unwrap_or_default();
    if padding.iter().any(|byte| *byte != 0) {
        return Err("program data holds bytes beyond the local binary".into());
    }
    Ok(OnChainProgram {
        owner: program.owner,
        executable: program.executable,
        program_data,
        upgrade_authority,
        last_deploy_slot: u64::from_le_bytes(slot),
        code: code.to_vec(),
    })
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(
        String::with_capacity(bytes.len().saturating_mul(2)),
        |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        },
    )
}

/// Re-fetches every recorded program and transaction and compares them with
/// the record. It needs only RPC access, never a key.
pub(crate) fn verify(path: &Path) -> Res<()> {
    let evidence: Evidence = serde_json::from_str(&fs::read_to_string(path)?)?;
    let url = std::env::var("SOLEVM_RPC_URL").unwrap_or_else(|_| evidence.rpc_url.clone());
    let chain = Chain::new(&url);
    let mut problems = Vec::new();

    let genesis = chain.genesis_hash()?;
    if genesis != evidence.genesis_hash {
        problems.push(format!(
            "cluster genesis {genesis} differs from the recorded {}",
            evidence.genesis_hash
        ));
    }

    for program in &evidence.programs {
        let id = Pubkey::from_str(&program.program_id)
            .map_err(|error| format!("{}: {error:?}", program.program_id))?;
        let len = usize::try_from(program.binary_len)?;
        match read_program(&chain, &id, len) {
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
                    "program {:<22} {} sha256 {} {}",
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
        if let Some(signature) = &program.deploy_signature {
            let expected = Recorded {
                id: "deploy",
                signature,
                slot: program.deploy_slot,
                failed: None,
                code: None,
                program: bpf_loader_upgradeable::ID.to_string(),
            };
            verify_signature(&chain, &expected, &mut problems)?;
        }
    }

    for step in &evidence.steps {
        let expected = Recorded {
            id: &step.id,
            signature: &step.signature,
            slot: Some(step.slot),
            failed: Some(step.outcome != "success"),
            code: step.error_code,
            program: step.program.clone(),
        };
        verify_signature(&chain, &expected, &mut problems)?;
    }

    if problems.is_empty() {
        println!(
            "verified {} programs and {} transactions at finalized commitment against {url}",
            evidence.programs.len(),
            evidence.steps.len()
        );
        Ok(())
    } else {
        for problem in &problems {
            eprintln!("MISMATCH: {problem}");
        }
        Err(format!("{} mismatches", problems.len()).into())
    }
}

/// One recorded transaction and the result it must still show on chain.
struct Recorded<'r> {
    id: &'r str,
    signature: &'r str,
    slot: Option<u64>,
    failed: Option<bool>,
    code: Option<u32>,
    program: String,
}

fn verify_signature(chain: &Chain, recorded: &Recorded<'_>, problems: &mut Vec<String>) -> Res<()> {
    let Recorded { id, signature, .. } = *recorded;
    let Some(landed) = chain.transaction(signature, "finalized")? else {
        problems.push(format!(
            "{id} {signature}: not found at finalized commitment"
        ));
        println!("{id:<6} {signature} NOT FOUND");
        return Ok(());
    };
    let mut ok = recorded.slot.is_none_or(|slot| slot == landed.slot)
        && landed.programs.contains(&recorded.program);
    if let Some(expected_failure) = recorded.failed {
        ok = ok && landed.error.is_some() == expected_failure;
        if expected_failure {
            ok = ok && landed.custom_code() == recorded.code;
        }
    }
    let status = match landed.error {
        None => String::from("success"),
        Some(_) => format!(
            "failed({})",
            landed
                .custom_code()
                .map_or_else(String::new, |c| c.to_string())
        ),
    };
    println!(
        "{id:<6} {signature} slot {} {} {status} {}",
        landed.slot,
        landed.block_time.map_or_else(String::new, utc),
        if ok { "OK" } else { "MISMATCH" }
    );
    if !ok {
        problems.push(format!(
            "{id} {signature}: on-chain result differs from the record"
        ));
    }
    Ok(())
}
