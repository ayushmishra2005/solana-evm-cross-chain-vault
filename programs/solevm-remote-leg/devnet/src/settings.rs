//! Run settings, read from the environment.

use std::env;
use std::path::{Path, PathBuf};

use crate::Res;

pub(crate) const DEFAULT_RPC_URL: &str = "https://api.devnet.solana.com";

pub(crate) struct Settings {
    pub rpc_url: String,
    pub home: PathBuf,
    pub label: String,
    pub allow_localnet: bool,
    pub leg_binary: PathBuf,
    pub adapter_binary: PathBuf,
    pub source_commit: String,
    pub source_tree: String,
    pub source_dirty: bool,
}

impl Settings {
    pub(crate) fn from_env() -> Res<Self> {
        let home = match env::var_os("SOLEVM_HOME") {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from(env::var_os("HOME").ok_or("HOME is not set")?)
                .join(".config")
                .join("solevm-devnet"),
        };
        refuse_inside_repository(&home)?;

        let label = env::var("SOLEVM_RUN_LABEL").unwrap_or_else(|_| String::from("run-1"));
        let valid = !label.is_empty()
            && label.len() <= 32
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid {
            return Err("SOLEVM_RUN_LABEL must be 1-32 characters of a-z, 0-9 and '-'".into());
        }

        let deploy = program_root().join("target").join("deploy-devnet");
        Ok(Self {
            rpc_url: env::var("SOLEVM_RPC_URL").unwrap_or_else(|_| String::from(DEFAULT_RPC_URL)),
            home,
            label,
            allow_localnet: env::var("SOLEVM_ALLOW_LOCALNET").is_ok_and(|value| value == "1"),
            leg_binary: env::var_os("SOLEVM_LEG_SO")
                .map_or_else(|| deploy.join("solevm_remote_leg.so"), PathBuf::from),
            adapter_binary: env::var_os("SOLEVM_ADAPTER_SO")
                .map_or_else(|| deploy.join("solevm_test_strategy.so"), PathBuf::from),
            source_commit: env::var("SOLEVM_SOURCE_COMMIT")
                .unwrap_or_else(|_| String::from("unknown")),
            source_tree: env::var("SOLEVM_SOURCE_TREE").unwrap_or_else(|_| String::from("unknown")),
            source_dirty: env::var("SOLEVM_SOURCE_DIRTY").is_ok_and(|value| value == "1"),
        })
    }

    pub(crate) fn key_dir(&self) -> PathBuf {
        self.home.join("keys")
    }

    /// Deploy records and evidence drafts, kept apart per cluster.
    pub(crate) fn state_dir(&self, cluster: &str) -> PathBuf {
        self.home.join("state").join(cluster)
    }
}

/// The remote leg program directory, which holds the devnet build output.
fn program_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Keys and evidence drafts must never land in the working tree.
fn refuse_inside_repository(home: &Path) -> Res<()> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let mut probe = home.to_path_buf();
    // The home may not exist yet, so resolve the closest existing ancestor.
    loop {
        if let Ok(resolved) = probe.canonicalize() {
            if resolved.starts_with(&repository) {
                return Err(format!(
                    "SOLEVM_HOME ({}) must be outside the repository",
                    home.display()
                )
                .into());
            }
            return Ok(());
        }
        if !probe.pop() {
            return Ok(());
        }
    }
}
