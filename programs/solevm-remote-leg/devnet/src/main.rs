//! Drives the deployed remote leg through allocation, reporting, close,
//! recovery and the frozen guardian path on devnet, and records evidence that
//! anyone can re-check against a public RPC without any keys.

#[cfg(not(solevm_devnet))]
compile_error!(
    "build from programs/solevm-remote-leg/devnet so `--cfg solevm_devnet` applies \
     and the program ids match the devnet binaries"
);

mod chain;
mod evidence;
mod keys;
mod messages;
mod namespace;
mod report;
mod scenario;
mod settings;
mod time;

use std::path::Path;
use std::process::ExitCode;

/// Failures carry their context as text; the driver only reports them.
type Res<T> = Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "usage:
  solevm-devnet addresses           print every address the namespace uses (no network)
  solevm-devnet run                 set up a fresh namespace and run the scenario
  solevm-devnet verify <evidence>   re-check recorded evidence against the RPC (no keys)
  solevm-devnet report <evidence>   render recorded evidence as Markdown

environment:
  SOLEVM_RPC_URL         RPC endpoint (default https://api.devnet.solana.com)
  SOLEVM_HOME            key and state directory outside the repository
                         (default $HOME/.config/solevm-devnet)
  SOLEVM_RUN_LABEL       namespace label; a new label gives a fresh namespace
  SOLEVM_ALLOW_LOCALNET  set to 1 to allow a local test validator";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let outcome = match words.as_slice() {
        ["addresses"] => scenario::print_addresses(),
        ["run"] => scenario::run(),
        ["verify", path] => evidence::verify(Path::new(path)),
        ["report", path] => report::render(Path::new(path)),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
