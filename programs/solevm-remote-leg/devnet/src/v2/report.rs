//! Renders a V2 record as Markdown, so published tables come from the record
//! instead of being copied by hand. The first line states which cluster made
//! the record, so a localnet rehearsal can never read as devnet validation.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use super::evidence::Evidence;
use crate::Res;

pub(crate) fn render(path: &Path) -> Res<()> {
    let evidence: Evidence = serde_json::from_str(&fs::read_to_string(path)?)?;
    print!("{}", markdown(&evidence)?);
    Ok(())
}

fn markdown(evidence: &Evidence) -> Res<String> {
    let mut out = String::new();
    let summary = &evidence.summary;
    writeln!(
        out,
        "> **{}** — record kind `{}`, schema {}.",
        evidence.validation, evidence.kind, evidence.schema
    )?;
    writeln!(out)?;
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
        "| Node | `{}` |",
        evidence
            .node_version
            .get("solana-core")
            .and_then(|version| version.as_str())
            .unwrap_or("unknown")
    )?;
    for feature in &evidence.features {
        writeln!(
            out,
            "| Feature `{}` | {}: {} |",
            feature.id,
            feature.name,
            feature.activated_at_slot.map_or_else(
                || String::from("not active"),
                |slot| format!("active since slot {slot}")
            )
        )?;
    }
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
        "| Deployment authority | `{}`, network domain = genesis: {} |",
        evidence.authorization.deployment_authority,
        evidence.authorization.network_domain_is_genesis
    )?;
    writeln!(
        out,
        "| Risk public key | `{}` (P-256, generated in memory for this run) |",
        evidence.authorization.risk_public_key_hex
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
    if let Some(spent) = evidence.balances.net_spent_lamports {
        writeln!(out, "| Run cost | {spent} lamports, net of role balances |")?;
    }
    writeln!(out)?;

    writeln!(
        out,
        "| Program | Program id | Upgrade authority | On-chain sha256 = local build |"
    )?;
    writeln!(out, "|---|---|---|---|")?;
    for program in &evidence.programs {
        writeln!(
            out,
            "| `{}` | [`{}`]({}) | `{}` | `{}` ({}) |",
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
        )?;
    }
    writeln!(out)?;

    writeln!(
        out,
        "| Step | What it proves | Expected | Observed | Slot | Bytes | CU | Transaction |"
    )?;
    writeln!(out, "|---|---|---|---|---|---|---|---|")?;
    for step in &evidence.steps {
        writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | [{}]({}) |",
            step.id,
            step.title,
            step.expected,
            step.outcome,
            step.slot,
            step.transaction_bytes,
            step.compute_units
                .map_or_else(String::new, |units| units.to_string()),
            short(&step.signature),
            step.explorer_url
        )?;
    }
    writeln!(out)?;

    writeln!(
        out,
        "| Authorized transaction | Outcome | Bytes (limit) | Accounts | Total CU | Verifier CU | Leg CU | Fee | Receipt rent |"
    )?;
    writeln!(out, "|---|---|---|---|---|---|---|---|---|")?;
    for measurement in &evidence.measurements {
        let number =
            |value: Option<u64>| value.map_or_else(|| String::from("-"), |n| n.to_string());
        writeln!(
            out,
            "| {} | {} | {} ({}) | {} | {} | {} | {} | {} | {} |",
            measurement.step,
            measurement.outcome,
            measurement.transaction_bytes,
            measurement.packet_limit_bytes,
            measurement.account_keys,
            number(measurement.compute_units),
            number(measurement.verifier_compute_units),
            number(measurement.leg_compute_units),
            number(measurement.fee_lamports),
            number(measurement.receipt_rent_lamports),
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
