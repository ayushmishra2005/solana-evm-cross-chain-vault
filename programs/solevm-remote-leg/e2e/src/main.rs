//! Foundry FFI entry for the live Solana + xchain-sim driver.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects
)]

use std::io::{self, Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let input = match read_input() {
        Ok(input) => input,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    match solevm_e2e::run_script(&input) {
        Ok(output) => {
            if let Err(error) = io::stdout().write_all(output.as_bytes()) {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn read_input() -> Result<String, String> {
    if let Some(argument) = std::env::args().nth(1) {
        if argument == "-" {
            return read_stdin();
        }
        return Ok(argument);
    }
    read_stdin()
}

fn read_stdin() -> Result<String, String> {
    let mut buffer = String::new();
    io::stdin()
        .read_to_string(&mut buffer)
        .map_err(|error| error.to_string())?;
    if buffer.trim().is_empty() {
        return Err(String::from("missing script"));
    }
    Ok(buffer)
}
