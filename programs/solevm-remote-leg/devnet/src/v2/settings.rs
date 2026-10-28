//! V2 run settings, read from the environment. Nothing falls back to a
//! public endpoint: the RPC URL and the expected cluster are both required.

use std::env;
use std::path::{Path, PathBuf};

use crate::Res;
use crate::chain::{Chain, DEVNET_GENESIS, MAINNET_GENESIS, TESTNET_GENESIS};
use crate::settings::refuse_inside_repository;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cluster {
    Devnet,
    Localnet,
}

impl Cluster {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Devnet => "devnet",
            Self::Localnet => "localnet",
        }
    }

    /// Requires the endpoint's genesis to be this cluster's, and returns it.
    pub(crate) fn confirm(self, chain: &Chain) -> Res<String> {
        let genesis = chain.genesis_hash()?;
        if genesis == MAINNET_GENESIS || genesis == TESTNET_GENESIS {
            return Err(format!(
                "refusing {}: genesis {genesis} is mainnet-beta or testnet",
                chain.url()
            )
            .into());
        }
        let matches = match self {
            Self::Devnet => genesis == DEVNET_GENESIS,
            Self::Localnet => genesis != DEVNET_GENESIS,
        };
        if !matches {
            return Err(format!(
                "{} has genesis {genesis}, which is not {}",
                chain.url(),
                self.name()
            )
            .into());
        }
        Ok(genesis)
    }
}

pub(crate) struct Settings {
    pub rpc_url: String,
    pub cluster: Cluster,
    pub home: PathBuf,
    pub label: String,
    pub leg_binary: PathBuf,
    pub verifier_binary: PathBuf,
    pub adapter_binary: PathBuf,
    pub source_commit: String,
    pub source_tree: String,
    pub source_dirty: bool,
    /// The devnet feature set a local validator was started to match.
    pub feature_mirror: Option<PathBuf>,
    /// SOLEVM_V2_DEVNET_APPROVED=1, the operator's second step for devnet.
    pub devnet_approved: bool,
}

impl Settings {
    pub(crate) fn from_env() -> Res<Self> {
        let user_home = PathBuf::from(env::var_os("HOME").ok_or("HOME is not set")?);
        let home = env::var_os("SOLEVM_V2_HOME").map_or_else(
            || user_home.join(".config").join("solevm-devnet-v2"),
            PathBuf::from,
        );
        refuse_inside_repository(&home)?;
        let v1_home = env::var_os("SOLEVM_HOME").map(|home| (PathBuf::from(home), "SOLEVM_HOME"));
        for (other, name) in [
            (
                user_home.join(".config").join("solevm-devnet"),
                "the V1 home",
            ),
            (
                user_home.join(".config").join("solana"),
                "the Solana CLI config",
            ),
        ]
        .into_iter()
        .chain(v1_home)
        {
            if resolve(&home).starts_with(resolve(&other)) {
                return Err(format!(
                    "SOLEVM_V2_HOME ({}) must not be inside {name}",
                    home.display()
                )
                .into());
            }
        }

        let rpc_url = env::var("SOLEVM_RPC_URL").map_err(|_| "set SOLEVM_RPC_URL explicitly")?;
        let cluster = match env::var("SOLEVM_CLUSTER").as_deref() {
            Ok("devnet") => Cluster::Devnet,
            Ok("localnet") => {
                if !(rpc_url.starts_with("http://127.0.0.1:")
                    || rpc_url.starts_with("http://localhost:"))
                {
                    return Err("a localnet endpoint must be on 127.0.0.1 or localhost".into());
                }
                Cluster::Localnet
            }
            _ => return Err("set SOLEVM_CLUSTER to devnet or localnet".into()),
        };

        let label = env::var("SOLEVM_RUN_LABEL").unwrap_or_else(|_| String::from("run-1"));
        let valid = !label.is_empty()
            && label.len() <= 32
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid {
            return Err("SOLEVM_RUN_LABEL must be 1-32 characters of a-z, 0-9 and '-'".into());
        }

        let deploy = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target")
            .join("deploy-devnet-v2");
        let binary = |variable: &str, name: &str| {
            env::var_os(variable).map_or_else(|| deploy.join(name), PathBuf::from)
        };
        let feature_mirror = env::var_os("SOLEVM_FEATURE_MIRROR").map(PathBuf::from);
        if feature_mirror.is_some() && cluster != Cluster::Localnet {
            return Err("SOLEVM_FEATURE_MIRROR describes a local validator; unset it".into());
        }
        Ok(Self {
            feature_mirror,
            rpc_url,
            cluster,
            home,
            label,
            leg_binary: binary("SOLEVM_LEG_SO", "solevm_remote_leg.so"),
            verifier_binary: binary("SOLEVM_VERIFIER_SO", "es256_authorization_verifier.so"),
            adapter_binary: binary("SOLEVM_ADAPTER_SO", "solevm_test_strategy.so"),
            source_commit: env::var("SOLEVM_SOURCE_COMMIT")
                .unwrap_or_else(|_| String::from("unknown")),
            source_tree: env::var("SOLEVM_SOURCE_TREE").unwrap_or_else(|_| String::from("unknown")),
            source_dirty: env::var("SOLEVM_SOURCE_DIRTY").is_ok_and(|value| value == "1"),
            devnet_approved: env::var("SOLEVM_V2_DEVNET_APPROVED").is_ok_and(|value| value == "1"),
        })
    }

    /// Devnet writes need the same deliberate second step as the scripts, so
    /// running the driver directly does not skip it. Localnet does not.
    pub(crate) fn require_write_approval(&self) -> Res<()> {
        if self.cluster == Cluster::Devnet && !self.devnet_approved {
            return Err("running the scenario on devnet needs SOLEVM_V2_DEVNET_APPROVED=1".into());
        }
        Ok(())
    }

    pub(crate) fn key_dir(&self) -> PathBuf {
        self.home.join("keys")
    }

    /// Deploy records and evidence, kept apart per cluster.
    pub(crate) fn state_dir(&self) -> PathBuf {
        self.home.join("state").join(self.cluster.name())
    }

    /// Where this run's record is written.
    pub(crate) fn evidence_path(&self) -> PathBuf {
        self.state_dir()
            .join(format!("evidence-v2-{}.json", self.label))
    }
}

/// `path` with its closest existing ancestor resolved, so a home that does
/// not exist yet still compares by its physical location.
fn resolve(path: &Path) -> PathBuf {
    let mut probe = path.to_path_buf();
    let mut rest = Vec::new();
    loop {
        if let Ok(resolved) = probe.canonicalize() {
            return rest.iter().rev().fold(resolved, |out, part| out.join(part));
        }
        match (probe.file_name().map(ToOwned::to_owned), probe.pop()) {
            (Some(name), true) => rest.push(name),
            _ => return path.to_path_buf(),
        }
    }
}
