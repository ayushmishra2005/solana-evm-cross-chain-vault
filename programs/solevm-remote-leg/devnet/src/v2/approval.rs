//! Risk approvals from a P-256 key that lives only in this process.
//!
//! The key stands in for a risk service that does not exist yet. It is
//! generated per run, never written anywhere, and only its public half is
//! registered and recorded. The message bytes come from the verifier's own
//! encoder and the precompile instruction from the shared adapter, so the
//! driver re-implements neither.

use es256_authorization_verifier::deployment::DeploymentAuthorization;
use es256_jws_adapter::{SyntheticSigner, normalize_signature, secp256r1_instruction};
use solana_instruction::Instruction;

use crate::Res;
use crate::evidence::sha256_hex;

pub(crate) struct RiskKey {
    signer: SyntheticSigner,
    public_key: [u8; 33],
}

impl RiskKey {
    pub(crate) fn generate() -> Res<Self> {
        let signer = SyntheticSigner::generate().map_err(|error| format!("P-256: {error:?}"))?;
        let public_key = signer
            .public_key()
            .map_err(|error| format!("P-256: {error:?}"))?;
        Ok(Self { signer, public_key })
    }

    pub(crate) fn public_key(&self) -> [u8; 33] {
        self.public_key
    }

    /// Signs `message` with a low-S signature, as the precompile requires.
    pub(crate) fn approve(&self, message: DeploymentAuthorization) -> Res<Approval> {
        let signing_input = message.encode();
        let raw = self
            .signer
            .sign(&signing_input)
            .map_err(|error| format!("P-256 sign: {error:?}"))?;
        let signature = normalize_signature(&raw).map_err(|error| format!("low-S: {error:?}"))?;
        Ok(Approval {
            message,
            signing_input,
            signature,
            public_key: self.public_key,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Approval {
    pub message: DeploymentAuthorization,
    pub signing_input: Vec<u8>,
    pub signature: [u8; 64],
    pub public_key: [u8; 33],
}

impl Approval {
    pub(crate) fn precompile(&self) -> Res<Instruction> {
        secp256r1_instruction(&self.public_key, &self.signature, &self.signing_input)
            .map_err(|error| format!("secp256r1 instruction: {error:?}").into())
    }

    /// The precompile presenting this signature over the same message with
    /// a different amount, as a relayer inflating an approval would.
    pub(crate) fn with_amount_altered(&self, amount: u64) -> Res<Instruction> {
        let altered = DeploymentAuthorization {
            amount,
            ..self.message
        };
        secp256r1_instruction(&self.public_key, &self.signature, &altered.encode())
            .map_err(|error| format!("secp256r1 instruction: {error:?}").into())
    }

    pub(crate) fn nonce(&self) -> [u8; 16] {
        self.message.nonce
    }

    pub(crate) fn message_sha256(&self) -> String {
        sha256_hex(&self.signing_input)
    }
}

#[cfg(test)]
mod tests {
    use anchor_lang::prelude::Pubkey;
    use es256_jws_adapter::is_low_s;

    use super::*;

    fn message(amount: u64) -> DeploymentAuthorization {
        DeploymentAuthorization {
            authority: Pubkey::new_unique(),
            network_domain: [7; 32],
            policy_generation: 1,
            deployment_context: [9; 32],
            amount,
            nonce: [3; 16],
            issued_at: 1_000,
            expires_at: 1_300,
        }
    }

    #[test]
    fn approval_signs_the_verifier_encoding_with_low_s() -> Res<()> {
        let key = RiskKey::generate()?;
        let approval = key.approve(message(150_000))?;
        assert_eq!(approval.signing_input, approval.message.encode());
        assert_eq!(approval.signing_input.len(), 154);
        assert!(is_low_s(&approval.signature).map_err(|error| format!("{error:?}"))?);
        assert_eq!(approval.public_key, key.public_key());

        let precompile = approval.precompile()?;
        assert!(
            precompile
                .data
                .windows(approval.signing_input.len())
                .any(|window| window == approval.signing_input.as_slice())
        );
        Ok(())
    }

    #[test]
    fn altered_amount_keeps_the_signature_and_changes_only_the_message() -> Res<()> {
        let approval = RiskKey::generate()?.approve(message(150_000))?;
        let genuine = approval.precompile()?;
        let altered = approval.with_amount_altered(150_001)?;
        assert_eq!(genuine.program_id, altered.program_id);
        assert_eq!(genuine.data.len(), altered.data.len());
        let differing = genuine
            .data
            .iter()
            .zip(&altered.data)
            .filter(|(left, right)| left != right)
            .count();
        assert_eq!(differing, 1, "150000 and 150001 differ in one byte");
        Ok(())
    }
}
