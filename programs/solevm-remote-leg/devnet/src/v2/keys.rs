//! Disposable V2 keys, read from a directory outside the repository.
//!
//! Only public keys ever leave this module. The P-256 risk key is not here:
//! the scenario generates it in memory for each run and never stores it.

use std::path::Path;

use anchor_lang::prelude::Pubkey;
use solana_keypair::{Keypair, read_keypair_file};
use solana_signer::Signer;

use super::FOREIGN_PROGRAMS;
use crate::Res;
use crate::keys::{read, read_or_create};

pub(crate) struct Keys {
    /// Fee payer and upgrade authority of the V2 leg and verifier.
    pub deployer: Keypair,
    /// Leg administrator, and the test adapter's test authority.
    pub administrator: Keypair,
    pub guardian: Keypair,
    pub verifier: Keypair,
    /// Mint authority, and the wallet that plays the bridge delivering assets.
    pub transport: Keypair,
    /// Signs every forged action in the negative tests.
    pub attacker: Keypair,
    /// Administers the deployment authority: registers risk keys.
    pub risk_administrator: Keypair,
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
            risk_administrator: read(dir, "risk-administrator")?,
            mint: read_or_create(&dir.join(format!("test-mint-{label}.json")))?,
            escrow_owner: read(dir, "escrow-owner")?.pubkey(),
        };
        for (file, declared) in [
            ("leg-program", solevm_remote_leg::ID),
            ("verifier-program", es256_authorization_verifier::ID),
        ] {
            let key = read(dir, file)?.pubkey();
            if key != declared {
                return Err(
                    format!("{file}.json is {key}, but the V2 build declares {declared}").into(),
                );
            }
            if FOREIGN_PROGRAMS.contains(&key) {
                return Err(format!("{file}.json is {key}, a V1 or production program").into());
            }
        }

        let roles = keys.roles();
        for (index, (role, key)) in roles.iter().enumerate() {
            if roles
                .iter()
                .skip(index.saturating_add(1))
                .any(|(_, other)| other == key)
            {
                return Err(format!("the {role} key is shared with another role").into());
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            let wallet = Path::new(&home).join(".config/solana/id.json");
            if let Ok(default) = read_keypair_file(&wallet)
                && let Some((role, _)) = roles.iter().find(|(_, key)| *key == default.pubkey())
            {
                return Err(format!("the {role} key is the Solana CLI default wallet").into());
            }
        }
        Ok(keys)
    }

    /// Every signer role with its public key.
    pub(crate) fn roles(&self) -> Vec<(&'static str, Pubkey)> {
        vec![
            ("deployer_and_upgrade_authority", self.deployer.pubkey()),
            (
                "administrator_and_adapter_test_authority",
                self.administrator.pubkey(),
            ),
            ("emergency_guardian", self.guardian.pubkey()),
            ("transport_verifier", self.verifier.pubkey()),
            (
                "asset_transport_and_mint_authority",
                self.transport.pubkey(),
            ),
            ("attacker", self.attacker.pubkey()),
            ("risk_administrator", self.risk_administrator.pubkey()),
            ("escrow_owner", self.escrow_owner),
        ]
    }
}
