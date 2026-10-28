//! Disposable devnet keys, read from a directory outside the repository.
//!
//! Only public keys ever leave this module.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use anchor_lang::prelude::Pubkey;
use solana_keypair::{Keypair, read_keypair_file};
use solana_signer::Signer;

use crate::Res;

pub(crate) struct Keys {
    /// Fee payer and upgrade authority of both programs.
    pub deployer: Keypair,
    pub administrator: Keypair,
    pub guardian: Keypair,
    pub verifier: Keypair,
    /// Mint authority, and the wallet that plays the bridge delivering assets.
    pub transport: Keypair,
    /// Signs every forged authorization in the negative tests.
    pub attacker: Keypair,
    pub mint: Keypair,
    pub escrow_owner: Pubkey,
}

impl Keys {
    pub(crate) fn load(dir: &Path, label: &str) -> Res<Self> {
        let keys = Self {
            deployer: read(dir, "deployer")?,
            administrator: read(dir, "administrator")?,
            guardian: read(dir, "guardian")?,
            verifier: read(dir, "transport-verifier")?,
            transport: read(dir, "asset-transport")?,
            attacker: read(dir, "attacker")?,
            mint: read_or_create(&dir.join(format!("test-mint-{label}.json")))?,
            escrow_owner: read(dir, "escrow-owner")?.pubkey(),
        };
        let leg = read(dir, "leg-program")?.pubkey();
        if leg != solevm_remote_leg::ID {
            return Err(format!(
                "leg-program.json is {leg}, but the devnet build declares {}",
                solevm_remote_leg::ID
            )
            .into());
        }
        let adapter = read(dir, "adapter-program")?.pubkey();
        if adapter != solevm_test_strategy::ID {
            return Err(format!(
                "adapter-program.json is {adapter}, but the devnet build declares {}",
                solevm_test_strategy::ID
            )
            .into());
        }
        Ok(keys)
    }
}

pub(crate) fn read(dir: &Path, name: &str) -> Res<Keypair> {
    let path = dir.join(format!("{name}.json"));
    read_keypair_file(&path).map_err(|error| format!("{}: {error}", path.display()).into())
}

pub(crate) fn read_or_create(path: &Path) -> Res<Keypair> {
    if path.exists() {
        return read_keypair_file(path)
            .map_err(|error| format!("{}: {error}", path.display()).into());
    }
    let keypair = Keypair::new();
    let encoded = serde_json::to_string(&keypair.to_bytes().to_vec())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(encoded.as_bytes())?;
    Ok(keypair)
}
