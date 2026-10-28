//! Every address one V2 namespace uses, derived from public inputs.

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use es256_authorization_verifier::{
    DEPLOYMENT_AUTHORITY_SEED, DEPLOYMENT_RECEIPT_SEED, RISK_KEY_SEED,
};
use solana_signer::Signer;
use solevm_remote_leg::{DEPLOY_AUTHORIZATION_SEED, RecallSentMarker, RemoteConfig};

use super::keys::Keys;
use crate::namespace::Namespace;

/// Prefix of every V2 namespace tag, so no V2 label collides with a V1 one.
pub(crate) const PREFIX: &str = "solevm/devnet-v2";

#[derive(Clone, Debug)]
pub(crate) struct V2Namespace {
    /// Leg, adapter and token addresses, derived as V1 derives them.
    pub base: Namespace,
    pub deployment_authority: Pubkey,
    pub authorization_policy: Pubkey,
    pub adapter_caller: Pubkey,
    pub verifier_program_data: Pubkey,
}

impl V2Namespace {
    pub(crate) fn derive(label: &str, keys: &Keys) -> Self {
        let base = Namespace::derive_in(
            PREFIX,
            label,
            keys.mint.pubkey(),
            keys.escrow_owner,
            keys.transport.pubkey(),
        );
        let verifier = es256_authorization_verifier::ID;
        let deployment_authority = Pubkey::find_program_address(
            &[
                DEPLOYMENT_AUTHORITY_SEED,
                solevm_remote_leg::ID.as_ref(),
                base.config.as_ref(),
            ],
            &verifier,
        )
        .0;
        Self {
            authorization_policy: Pubkey::find_program_address(
                &[DEPLOY_AUTHORIZATION_SEED, base.config.as_ref()],
                &solevm_remote_leg::ID,
            )
            .0,
            adapter_caller: RemoteConfig::adapter_caller(&base.config).0,
            verifier_program_data: Pubkey::find_program_address(
                &[verifier.as_ref()],
                &bpf_loader_upgradeable::ID,
            )
            .0,
            deployment_authority,
            base,
        }
    }

    pub(crate) fn risk_key(&self, public_key: &[u8; 33]) -> Pubkey {
        let (parity, x) = public_key.split_at(1);
        Pubkey::find_program_address(
            &[RISK_KEY_SEED, self.deployment_authority.as_ref(), parity, x],
            &es256_authorization_verifier::ID,
        )
        .0
    }

    pub(crate) fn receipt(&self, nonce: &[u8; 16]) -> Pubkey {
        Pubkey::find_program_address(
            &[
                DEPLOYMENT_RECEIPT_SEED,
                self.deployment_authority.as_ref(),
                nonce,
            ],
            &es256_authorization_verifier::ID,
        )
        .0
    }

    pub(crate) fn recall_marker(&self, transfer_id: &[u8; 32]) -> Pubkey {
        RecallSentMarker::address(&self.base.config, transfer_id).0
    }

    /// The nonce of the approval named `name`, unique to this namespace.
    pub(crate) fn nonce(&self, name: &str) -> [u8; 16] {
        let mut nonce = [0u8; 16];
        let tag = self.base.tag(&format!("nonce/{name}"));
        nonce.copy_from_slice(tag.get(..16).unwrap_or(&[0u8; 16]));
        nonce
    }

    /// Every named address the namespace uses, for the evidence and listing.
    pub(crate) fn addresses(&self) -> Vec<(&'static str, Pubkey)> {
        let ns = &self.base;
        vec![
            ("leg_program", solevm_remote_leg::ID),
            ("leg_program_data", ns.program_data),
            ("verifier_program", es256_authorization_verifier::ID),
            ("verifier_program_data", self.verifier_program_data),
            ("adapter_program", solevm_test_strategy::ID),
            ("test_mint", ns.mint),
            ("remote_config", ns.config),
            ("custody_authority", ns.custody_authority),
            ("custody_token_account", ns.custody),
            ("outbound_escrow", ns.escrow),
            ("risk_config", ns.risk_config),
            ("control_lane", ns.control_lane),
            ("strategy_config", ns.strategy_config),
            ("remote_position", ns.position),
            ("outbound_lane", ns.outbound_lane),
            ("adapter_state", ns.adapter_state),
            ("adapter_authority", ns.adapter_authority),
            ("adapter_token_vault", ns.adapter_vault),
            ("adapter_caller", self.adapter_caller),
            ("transport_source_token_account", ns.transport_source),
            ("deployment_authority", self.deployment_authority),
            ("authorization_policy", self.authorization_policy),
        ]
    }
}

#[cfg(test)]
mod tests {
    use solevm_remote_leg::REMOTE_CONFIG_SEED;

    use super::*;

    fn derive(prefix: &'static str, label: &str) -> Namespace {
        Namespace::derive_in(
            prefix,
            label,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        )
    }

    /// The V1 tags still produce the recorded V1 config.
    #[test]
    fn v1_derivation_matches_the_recorded_devnet_config() {
        let ns = derive("solevm/devnet", "devnet-1");
        let v1_leg = Pubkey::from_str_const("27zReYgZvFTaUSWwob4qirN2Yxekcm32mnQDpMzeTyAE");
        let config = Pubkey::find_program_address(
            &[REMOTE_CONFIG_SEED, &ns.deployment_id, &ns.vault_id],
            &v1_leg,
        )
        .0;
        assert_eq!(
            config.to_string(),
            "BNcFskRtpkHrTbSzAzxbhTCVLjyViewdwrQZeA2MAZYi"
        );
    }

    #[test]
    fn v2_tags_never_collide_with_v1_tags() {
        let v1 = derive("solevm/devnet", "devnet-1");
        let v2 = derive(PREFIX, "devnet-1");
        assert_ne!(v1.deployment_id, v2.deployment_id);
        assert_ne!(v1.vault_id, v2.vault_id);
        assert_ne!(v1.transfer_id("allocation"), v2.transfer_id("allocation"));
    }
}
