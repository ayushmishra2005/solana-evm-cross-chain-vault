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
mod v2;

use std::path::Path;
use std::process::ExitCode;

/// Failures carry their context as text; the driver only reports them.
type Res<T> = Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "usage:
  solevm-devnet addresses              print every address the namespace uses (no network)
  solevm-devnet run                    set up a fresh namespace and run the scenario
  solevm-devnet verify <evidence>      re-check recorded evidence against the RPC (no keys)
  solevm-devnet report <evidence>      render recorded evidence as Markdown
  solevm-devnet addresses-v2           print every address a V2 namespace uses (no network)
  solevm-devnet run-v2                 run the V2 deployment-authorization scenario
  solevm-devnet verify-v2 <evidence>   re-check a V2 record against the RPC (no keys)
  solevm-devnet report-v2 <evidence>   render a V2 record as Markdown

environment:
  SOLEVM_RPC_URL         RPC endpoint (V1 default https://api.devnet.solana.com;
                         V2 requires it)
  SOLEVM_HOME            V1 key and state directory outside the repository
                         (default $HOME/.config/solevm-devnet)
  SOLEVM_V2_HOME         V2 key and state directory outside the repository
                         (default $HOME/.config/solevm-devnet-v2)
  SOLEVM_CLUSTER         V2 only, required: devnet or localnet
  SOLEVM_RUN_LABEL       namespace label; a new label gives a fresh namespace
  SOLEVM_ALLOW_LOCALNET  V1 only: set to 1 to allow a local test validator";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let outcome = match words.as_slice() {
        ["addresses"] => scenario::print_addresses(),
        ["run"] => scenario::run(),
        ["verify", path] => evidence::verify(Path::new(path)),
        ["report", path] => report::render(Path::new(path)),
        ["addresses-v2"] => v2::scenario::print_addresses(),
        ["run-v2"] => v2::scenario::run(),
        ["verify-v2", path] => v2::evidence::verify(Path::new(path)),
        ["report-v2", path] => v2::report::render(Path::new(path)),
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
