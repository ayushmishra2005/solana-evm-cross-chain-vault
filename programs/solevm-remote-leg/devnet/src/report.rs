//! Renders recorded evidence as Markdown, so published tables are generated
//! from the record instead of being copied by hand.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::Res;
use crate::evidence::Evidence;

pub(crate) fn render(path: &Path) -> Res<()> {
    let evidence: Evidence = serde_json::from_str(&fs::read_to_string(path)?)?;
    print!("{}", markdown(&evidence)?);
    Ok(())
}

fn markdown(evidence: &Evidence) -> Res<String> {
    let mut out = String::new();
    let summary = &evidence.summary;
    writeln!(out, "| Field | Value |")?;
    writeln!(out, "|---|---|")?;
    writeln!(
        out,
        "| Cluster | {} (genesis `{}`) |",
        evidence.cluster, evidence.genesis_hash
    )?;
    writeln!(out, "| RPC | `{}` |", evidence.rpc_url)?;
    writeln!(
        out,
        "| Run | `{}`, {} to {} |",
        evidence.run_label, evidence.started_at_utc, evidence.completed_at_utc
    )?;
    writeln!(
        out,
        "| Source | commit `{}`{}, tree `{}` |",
        evidence.source.commit,
        if evidence.source.dirty {
            " plus uncommitted changes"
        } else {
            ""
        },
        evidence.source.tree
    )?;
    writeln!(
        out,
        "| Build | {}, {}, `{}` |",
        evidence.build.toolchain, evidence.build.arch, evidence.build.rustflags
    )?;
    writeln!(
        out,
        "| Result | {} transactions ({} expected successes, {} expected rejections), {}/{} checks passed, all finalized: {} |",
        summary.transactions,
        summary.expected_successes,
        summary.expected_failures,
        summary.checks_passed,
        summary.checks,
        summary.all_finalized
    )?;
    writeln!(out)?;

    writeln!(
        out,
        "| Program | Program id | Upgrade authority | On-chain sha256 = local build | Deploy transaction |"
    )?;
    writeln!(out, "|---|---|---|---|---|")?;
    for program in &evidence.programs {
        let deploy = program.deploy_signature.as_ref().map_or_else(
            || String::from("not recorded"),
            |signature| {
                format!(
                    "[{}]({}) slot {}",
                    short(signature),
                    tx_url(evidence, signature),
                    program.deploy_slot.unwrap_or(0)
                )
            },
        );
        writeln!(
            out,
            "| `{}` | [`{}`]({}) | `{}` | `{}` ({}) | {} |",
            program.name,
            program.program_id,
            program.explorer_url,
            program.upgrade_authority,
            program.onchain_sha256,
            if program.onchain_sha256 == program.local_sha256 {
                "match"
            } else {
                "MISMATCH"
            },
            deploy
        )?;
    }
    writeln!(out)?;

    writeln!(
        out,
        "| Step | What it proves | Instruction | Expected | Observed | Slot | Block time (UTC) | Transaction |"
    )?;
    writeln!(out, "|---|---|---|---|---|---|---|---|")?;
    for step in &evidence.steps {
        writeln!(
            out,
            "| {} | {} | `{}` | {} | {} | {} | {} | [{}]({}) |",
            step.id,
            step.title,
            step.instruction,
            step.expected,
            step.outcome,
            step.slot,
            step.block_time_utc,
            short(&step.signature),
            step.explorer_url
        )?;
    }
    writeln!(out)?;

    writeln!(out, "| Account | Address |")?;
    writeln!(out, "|---|---|")?;
    for (role, address) in &evidence.accounts {
        writeln!(
            out,
            "| {role} | [`{address}`]({}) |",
            address_url(evidence, address)
        )?;
    }
    Ok(out)
}

fn short(signature: &str) -> String {
    let head: String = signature.chars().take(8).collect();
    let tail: String = signature
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

fn suffix(evidence: &Evidence) -> String {
    if evidence.cluster == "devnet" {
        String::from("?cluster=devnet")
    } else {
        format!("?cluster=custom&customUrl={}", encode(&evidence.rpc_url))
    }
}

pub(crate) fn tx_url(evidence: &Evidence, signature: &str) -> String {
    format!(
        "https://explorer.solana.com/tx/{signature}{}",
        suffix(evidence)
    )
}

pub(crate) fn address_url(evidence: &Evidence, address: &str) -> String {
    format!(
        "https://explorer.solana.com/address/{address}{}",
        suffix(evidence)
    )
}

/// Percent-encodes everything but unreserved characters.
fn encode(text: &str) -> String {
    text.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
        out
    })
}
