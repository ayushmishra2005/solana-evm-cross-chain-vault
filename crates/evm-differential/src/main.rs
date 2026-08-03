use std::io::Write;
use std::process::ExitCode;

use evm_differential::abi::encode_bundle;
use evm_differential::action::ActionKind;
use evm_differential::generator::family_name;
use evm_differential::scenario::Scenario;
use evm_differential::{RunConfig, build_run};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Bundle,
    Stats,
    Describe(u32),
}

#[derive(Clone, Copy, Debug)]
struct Args {
    config: RunConfig,
    mode: Mode,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    let scenarios = match build_run(args.config) {
        Ok(scenarios) => scenarios,
        Err(reason) => {
            eprintln!(
                "generation failed at seed {} with {} cases: {reason}",
                args.config.seed, args.config.cases
            );
            eprintln!(
                "reproduce with: cargo run -p evm-differential -- --seed {} --cases {} --steps {}",
                args.config.seed, args.config.cases, args.config.steps
            );
            return ExitCode::FAILURE;
        }
    };

    match args.mode {
        Mode::Bundle => emit_bundle(args.config.seed, &scenarios),
        Mode::Stats => report_stats(args.config, &scenarios),
        Mode::Describe(index) => describe(&scenarios, index),
    }
}

const USAGE: &str = "usage: evm-differential [--seed N] [--cases N] [--steps N] [--only N] [--stats] [--describe N]";

fn parse_args() -> Result<Args, String> {
    let mut config = RunConfig::default();
    let mut mode = Mode::Bundle;
    let mut argv = std::env::args().skip(1);

    while let Some(flag) = argv.next() {
        match flag.as_str() {
            "--seed" => config.seed = read_number(&mut argv, "--seed")?,
            "--cases" => config.cases = read_small(&mut argv, "--cases")?,
            "--steps" => config.steps = read_small(&mut argv, "--steps")?,
            "--only" => {
                let index = read_small(&mut argv, "--only")?;
                config.only = Some(index);
            }
            "--stats" => mode = Mode::Stats,
            "--describe" => {
                let index = read_small(&mut argv, "--describe")?;
                mode = Mode::Describe(index);
            }
            "--help" | "-h" => return Err(String::from("help requested")),
            other => return Err(format!("unknown flag {other}")),
        }
    }

    if config.cases == 0 {
        return Err(String::from("--cases must be at least one"));
    }
    Ok(Args { config, mode })
}

fn read_number(argv: &mut impl Iterator<Item = String>, flag: &str) -> Result<u64, String> {
    let raw = argv.next().ok_or_else(|| format!("{flag} needs a value"))?;
    raw.parse::<u64>()
        .map_err(|_| format!("{flag} needs a whole number, got {raw}"))
}

fn read_small(argv: &mut impl Iterator<Item = String>, flag: &str) -> Result<u32, String> {
    let value = read_number(argv, flag)?;
    u32::try_from(value).map_err(|_| format!("{flag} is too large"))
}

/// Writes the hex bundle that Foundry reads over the standard output.
fn emit_bundle(seed: u64, scenarios: &[Scenario]) -> ExitCode {
    let raw = encode_bundle(seed, scenarios);
    let mut text = String::with_capacity(raw.len() * 2 + 2);
    text.push_str("0x");
    for byte in &raw {
        text.push_str(HEX.get(usize::from(*byte)).copied().unwrap_or("00"));
    }

    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    if handle.write_all(text.as_bytes()).is_err() || handle.flush().is_err() {
        eprintln!("could not write the bundle");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Prints how often each interesting state was reached.
fn report_stats(config: RunConfig, scenarios: &[Scenario]) -> ExitCode {
    let total = scenarios.len();
    let mut operations = 0usize;
    let mut successes = 0usize;
    let mut with_finalize = 0usize;
    let mut with_deposit_claim = 0usize;
    let mut with_redeem_claim = 0usize;
    let mut with_abort = 0usize;
    let mut with_deposit_refund = 0usize;
    let mut with_redeem_refund = 0usize;
    let mut with_rejection = 0usize;
    let mut with_cancel = 0usize;
    let mut with_pause = 0usize;
    let mut deepest = 0usize;

    for scenario in scenarios {
        operations += scenario.actions.len();
        successes += scenario.successes();
        deepest = deepest.max(scenario.actions.len());
        if scenario.has_finalized_epoch() {
            with_finalize += 1;
        }
        if scenario.has_aborted_epoch() {
            with_abort += 1;
        }
        if scenario.counts(ActionKind::ClaimDeposit) > 0 {
            with_deposit_claim += 1;
        }
        if scenario.counts(ActionKind::ClaimRedeem) > 0 {
            with_redeem_claim += 1;
        }
        if scenario.counts(ActionKind::RefundDeposit) > 0 {
            with_deposit_refund += 1;
        }
        if scenario.counts(ActionKind::RefundRedeem) > 0 {
            with_redeem_refund += 1;
        }
        if scenario.counts(ActionKind::CancelDeposit) + scenario.counts(ActionKind::CancelRedeem)
            > 0
        {
            with_cancel += 1;
        }
        if scenario.counts(ActionKind::Pause) > 0 {
            with_pause += 1;
        }
        if scenario.rejections() > 0 {
            with_rejection += 1;
        }
    }

    println!("seed {}", config.seed);
    println!("scenarios {total}");
    println!("operations {operations}");
    println!("successes {successes}");
    println!("rejections {}", operations.saturating_sub(successes));
    println!("deepest scenario {deepest}");
    print_rate("finalized epoch", with_finalize, total);
    print_rate("deposit claim", with_deposit_claim, total);
    print_rate("redemption claim", with_redeem_claim, total);
    print_rate("cancellation", with_cancel, total);
    print_rate("pause", with_pause, total);
    print_rate("abort", with_abort, total);
    print_rate("deposit refund", with_deposit_refund, total);
    print_rate("redemption refund", with_redeem_refund, total);
    print_rate("intentional rejection", with_rejection, total);

    for family in 0..evm_differential::generator::FAMILY_COUNT {
        let count = scenarios.iter().filter(|s| s.family == family).count();
        println!("family {} {count}", family_name(family));
    }

    ExitCode::SUCCESS
}

fn print_rate(label: &str, hits: usize, total: usize) {
    let percent = hits.saturating_mul(100).checked_div(total).unwrap_or(0);
    println!("{label} {hits}/{total} ({percent}%)");
}

/// Prints one scenario in a readable form for debugging a mismatch.
fn describe(scenarios: &[Scenario], index: u32) -> ExitCode {
    let Some(scenario) = scenarios
        .iter()
        .find(|scenario| scenario.index == index)
        .or_else(|| scenarios.first())
    else {
        eprintln!("no scenario to describe");
        return ExitCode::FAILURE;
    };

    println!(
        "scenario {} seed {} family {}",
        scenario.index,
        scenario.seed,
        family_name(scenario.family)
    );
    println!(
        "start {} duration {} minDeposit {} minRedeem {}",
        scenario.start_timestamp,
        scenario.epoch_duration,
        scenario.min_deposit,
        scenario.min_redeem
    );
    println!("initial assets {:?}", scenario.initial_assets);

    for (step, entry) in scenario.actions.iter().enumerate() {
        println!(
            "{step:>3} {:<16} actor {} amount {} epoch {} at {} -> {}",
            entry.action.kind.name(),
            entry.action.actor,
            entry.action.amount,
            entry.action.epoch,
            entry.action.timestamp,
            entry.result.name()
        );
    }

    for epoch in &scenario.epochs {
        println!(
            "epoch {} outcome {} deposits {} minted {} redeemed {} assets {} dust {}/{}",
            epoch.epoch_id,
            epoch.outcome,
            epoch.deposit_assets,
            epoch.minted_shares,
            epoch.redeem_shares,
            epoch.redeem_assets,
            epoch.deposit_dust,
            epoch.redeem_dust
        );
    }

    let successes = scenario.successes();
    println!(
        "operations {} successes {successes} rejections {}",
        scenario.actions.len(),
        scenario.rejections()
    );
    ExitCode::SUCCESS
}

const HEX: [&str; 256] = build_hex_table();

const fn build_hex_table() -> [&'static str; 256] {
    // A const table keeps the hex writer allocation free.
    [
        "00", "01", "02", "03", "04", "05", "06", "07", "08", "09", "0a", "0b", "0c", "0d", "0e",
        "0f", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "1a", "1b", "1c", "1d",
        "1e", "1f", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "2a", "2b", "2c",
        "2d", "2e", "2f", "30", "31", "32", "33", "34", "35", "36", "37", "38", "39", "3a", "3b",
        "3c", "3d", "3e", "3f", "40", "41", "42", "43", "44", "45", "46", "47", "48", "49", "4a",
        "4b", "4c", "4d", "4e", "4f", "50", "51", "52", "53", "54", "55", "56", "57", "58", "59",
        "5a", "5b", "5c", "5d", "5e", "5f", "60", "61", "62", "63", "64", "65", "66", "67", "68",
        "69", "6a", "6b", "6c", "6d", "6e", "6f", "70", "71", "72", "73", "74", "75", "76", "77",
        "78", "79", "7a", "7b", "7c", "7d", "7e", "7f", "80", "81", "82", "83", "84", "85", "86",
        "87", "88", "89", "8a", "8b", "8c", "8d", "8e", "8f", "90", "91", "92", "93", "94", "95",
        "96", "97", "98", "99", "9a", "9b", "9c", "9d", "9e", "9f", "a0", "a1", "a2", "a3", "a4",
        "a5", "a6", "a7", "a8", "a9", "aa", "ab", "ac", "ad", "ae", "af", "b0", "b1", "b2", "b3",
        "b4", "b5", "b6", "b7", "b8", "b9", "ba", "bb", "bc", "bd", "be", "bf", "c0", "c1", "c2",
        "c3", "c4", "c5", "c6", "c7", "c8", "c9", "ca", "cb", "cc", "cd", "ce", "cf", "d0", "d1",
        "d2", "d3", "d4", "d5", "d6", "d7", "d8", "d9", "da", "db", "dc", "dd", "de", "df", "e0",
        "e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8", "e9", "ea", "eb", "ec", "ed", "ee", "ef",
        "f0", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "fa", "fb", "fc", "fd", "fe",
        "ff",
    ]
}
