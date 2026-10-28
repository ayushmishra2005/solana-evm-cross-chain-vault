//! The SolEVM strategy-deployment authorization profile (V2).
//!
//! A risk authority approves one deployment of attributed custody by signing
//! one fixed-width message. The native secp256r1 program verifies the
//! signature over exactly these bytes. This program checks the fields it owns
//! (profile, network, policy generation, time, nonce, key) and records the
//! rest in a receipt. The remote leg recomputes the deployment context from
//! its own accounts and compares it with the receipt in the same transaction.
//!
//! Message layout, all integers little-endian:
//!
//! | Offset | Bytes | Field                 |
//! |-------:|------:|-----------------------|
//! |      0 |     8 | magic `SOLEVMAZ`      |
//! |      8 |     1 | profile version (2)   |
//! |      9 |     1 | operation (1, deploy) |
//! |     10 |    32 | deployment authority  |
//! |     42 |    32 | network domain        |
//! |     74 |     8 | policy generation     |
//! |     82 |    32 | deployment context    |
//! |    114 |     8 | amount (raw units)    |
//! |    122 |    16 | nonce                 |
//! |    138 |     8 | issued at             |
//! |    146 |     8 | expires at            |

use anchor_lang::prelude::*;
use solana_instructions_sysvar::{load_current_index_checked, load_instruction_at_checked};

use crate::errors::VerifierError;
use crate::state::{DEPLOYMENT_NONCE_LEN, DEPLOYMENT_PROFILE_VERSION, DeploymentAuthority};

type ParseResult<T> = core::result::Result<T, VerifierError>;

pub const MAGIC: [u8; 8] = *b"SOLEVMAZ";
pub const OPERATION_DEPLOY_TO_STRATEGY: u8 = 1;
pub const MESSAGE_LEN: usize = 154;
/// Longest accepted gap between `issued_at` and `expires_at`.
pub const MAX_LIFETIME_SECONDS: i64 = 300;

/// Domain of the deployment-context commitment.
pub const CONTEXT_DOMAIN: &[u8] = b"solevm-remote-leg:deploy-context:v2";

/// Anchor selector of the remote leg's `deploy_to_strategy`.
pub const DEPLOY_TO_STRATEGY_DISCRIMINATOR: [u8; 8] = [215, 49, 61, 222, 180, 60, 9, 118];
/// `deploy_to_strategy` takes the selector and one `u64` amount.
pub const DEPLOY_DATA_LEN: usize = 16;
/// Account positions in `deploy_to_strategy` that bind it to this consume.
pub const DEPLOY_ACCOUNT_COUNT: usize = 15;
pub const DEPLOY_REMOTE_CONFIG_INDEX: usize = 0;
pub const DEPLOY_RECEIPT_INDEX: usize = 13;

/// One signed deployment approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeploymentAuthorization {
    pub authority: Pubkey,
    pub network_domain: [u8; 32],
    pub policy_generation: u64,
    pub deployment_context: [u8; 32],
    pub amount: u64,
    pub nonce: [u8; DEPLOYMENT_NONCE_LEN],
    pub issued_at: i64,
    pub expires_at: i64,
}

impl DeploymentAuthorization {
    /// Accepts exactly one encoding: the layout above, with no trailing
    /// bytes, this profile's magic and version, and the deploy operation.
    pub fn decode(bytes: &[u8]) -> ParseResult<Self> {
        if bytes.len() != MESSAGE_LEN {
            return Err(VerifierError::InvalidAuthorizationMessage);
        }
        let mut reader = Reader(bytes);
        if reader.take::<8>()? != MAGIC || reader.take::<1>()? != [DEPLOYMENT_PROFILE_VERSION] {
            return Err(VerifierError::InvalidAuthorizationMessage);
        }
        if reader.take::<1>()? != [OPERATION_DEPLOY_TO_STRATEGY] {
            return Err(VerifierError::UnsupportedOperation);
        }
        Ok(Self {
            authority: Pubkey::new_from_array(reader.take()?),
            network_domain: reader.take()?,
            policy_generation: u64::from_le_bytes(reader.take()?),
            deployment_context: reader.take()?,
            amount: u64::from_le_bytes(reader.take()?),
            nonce: reader.take()?,
            issued_at: i64::from_le_bytes(reader.take()?),
            expires_at: i64::from_le_bytes(reader.take()?),
        })
    }

    /// The canonical bytes a risk authority signs.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MESSAGE_LEN);
        bytes.extend_from_slice(&MAGIC);
        bytes.push(DEPLOYMENT_PROFILE_VERSION);
        bytes.push(OPERATION_DEPLOY_TO_STRATEGY);
        bytes.extend_from_slice(self.authority.as_ref());
        bytes.extend_from_slice(&self.network_domain);
        bytes.extend_from_slice(&self.policy_generation.to_le_bytes());
        bytes.extend_from_slice(&self.deployment_context);
        bytes.extend_from_slice(&self.amount.to_le_bytes());
        bytes.extend_from_slice(&self.nonce);
        bytes.extend_from_slice(&self.issued_at.to_le_bytes());
        bytes.extend_from_slice(&self.expires_at.to_le_bytes());
        bytes
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> ParseResult<[u8; N]> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(VerifierError::InvalidAuthorizationMessage)?;
        self.0 = rest;
        Ok(*head)
    }
}

/// The remote-leg state one approval is scoped to. The risk authority hashes
/// what it evaluated; the remote leg hashes what it holds at execution. Every
/// field is fixed width, so the concatenation has one reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeploymentContext {
    pub remote_leg_program: Pubkey,
    pub remote_config: Pubkey,
    pub deployment_id: [u8; 32],
    pub vault_id: [u8; 32],
    /// Advanced by every canonical config update.
    pub config_version: u64,
    pub strategy_config: Pubkey,
    pub adapter_program: Pubkey,
    pub adapter_state: Pubkey,
    pub adapter_authority: Pubkey,
    pub adapter_token_vault: Pubkey,
    pub max_remote_principal: u64,
    pub asset_mint: Pubkey,
    pub token_program: Pubkey,
    pub custody_authority: Pubkey,
    pub custody_token_account: Pubkey,
    /// Exposure before this deployment.
    pub deployed_principal: u64,
}

impl DeploymentContext {
    /// SHA-256 over the domain, this verifier's id and every field in order.
    pub fn commitment(&self) -> [u8; 32] {
        solana_sha256_hasher::hashv(&[
            CONTEXT_DOMAIN,
            crate::ID.as_ref(),
            self.remote_leg_program.as_ref(),
            self.remote_config.as_ref(),
            &self.deployment_id,
            &self.vault_id,
            &self.config_version.to_le_bytes(),
            self.strategy_config.as_ref(),
            self.adapter_program.as_ref(),
            self.adapter_state.as_ref(),
            self.adapter_authority.as_ref(),
            self.adapter_token_vault.as_ref(),
            &self.max_remote_principal.to_le_bytes(),
            self.asset_mint.as_ref(),
            self.token_program.as_ref(),
            self.custody_authority.as_ref(),
            self.custody_token_account.as_ref(),
            &self.deployed_principal.to_le_bytes(),
        ])
        .to_bytes()
    }
}

/// Requires the instruction right after this one to be the remote leg's
/// `deploy_to_strategy` for this namespace, spending this receipt for this
/// exact amount. The remote leg makes the reciprocal check on its side.
pub fn check_deployment_follows(
    instructions_sysvar: &AccountInfo,
    authority: &DeploymentAuthority,
    receipt: &Pubkey,
    amount: u64,
) -> Result<()> {
    let current = usize::from(load_current_index_checked(instructions_sysvar)?);
    let next = current
        .checked_add(1)
        .and_then(|index| load_instruction_at_checked(index, instructions_sysvar).ok())
        .ok_or(VerifierError::MissingDeploymentInstruction)?;

    let mut expected_data = [0u8; DEPLOY_DATA_LEN];
    let (selector, argument) = expected_data.split_at_mut(DEPLOY_TO_STRATEGY_DISCRIMINATOR.len());
    selector.copy_from_slice(&DEPLOY_TO_STRATEGY_DISCRIMINATOR);
    argument.copy_from_slice(&amount.to_le_bytes());

    let named = |index: usize| next.accounts.get(index).map(|meta| meta.pubkey);
    require!(
        next.program_id == authority.remote_leg_program
            && next.data == expected_data
            && next.accounts.len() == DEPLOY_ACCOUNT_COUNT
            && named(DEPLOY_REMOTE_CONFIG_INDEX) == Some(authority.remote_config)
            && named(DEPLOY_RECEIPT_INDEX) == Some(*receipt),
        VerifierError::InvalidDeploymentInstruction
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn sample() -> DeploymentAuthorization {
        DeploymentAuthorization {
            authority: Pubkey::new_from_array([0xA1; 32]),
            network_domain: [0xA2; 32],
            policy_generation: 0x0102_0304_0506_0708,
            deployment_context: [0xA3; 32],
            amount: 40_000_000_000,
            nonce: [0xA4; DEPLOYMENT_NONCE_LEN],
            issued_at: 1_800_000_000,
            expires_at: 1_800_000_300,
        }
    }

    #[test]
    fn encoding_has_the_documented_layout_and_round_trips() {
        let message = sample();
        let bytes = message.encode();
        assert_eq!(bytes.len(), MESSAGE_LEN);
        assert_eq!(&bytes[0..8], b"SOLEVMAZ");
        assert_eq!(bytes[8], 2);
        assert_eq!(bytes[9], 1);
        assert_eq!(&bytes[10..42], &[0xA1; 32]);
        assert_eq!(&bytes[42..74], &[0xA2; 32]);
        assert_eq!(&bytes[74..82], &0x0102_0304_0506_0708u64.to_le_bytes());
        assert_eq!(&bytes[82..114], &[0xA3; 32]);
        assert_eq!(&bytes[114..122], &40_000_000_000u64.to_le_bytes());
        assert_eq!(&bytes[122..138], &[0xA4; 16]);
        assert_eq!(&bytes[138..146], &1_800_000_000i64.to_le_bytes());
        assert_eq!(&bytes[146..154], &1_800_000_300i64.to_le_bytes());
        assert_eq!(DeploymentAuthorization::decode(&bytes).unwrap(), message);
    }

    fn rejects_as(bytes: &[u8], expected: VerifierError) -> bool {
        DeploymentAuthorization::decode(bytes).is_err_and(|error| error as u32 == expected as u32)
    }

    #[test]
    fn every_truncation_and_extension_is_rejected() {
        let bytes = sample().encode();
        for length in 0..MESSAGE_LEN {
            assert!(
                rejects_as(&bytes[..length], VerifierError::InvalidAuthorizationMessage),
                "{length}"
            );
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(rejects_as(
            &longer,
            VerifierError::InvalidAuthorizationMessage
        ));
    }

    #[test]
    fn every_change_to_the_profile_prefix_is_rejected() {
        let canonical = sample().encode();
        for index in 0..10 {
            for value in 0..=255u8 {
                if value == canonical[index] {
                    continue;
                }
                let mut bytes = canonical.clone();
                bytes[index] = value;
                let expected = if index == 9 {
                    VerifierError::UnsupportedOperation
                } else {
                    VerifierError::InvalidAuthorizationMessage
                };
                assert!(rejects_as(&bytes, expected), "byte {index} = {value:#04x}");
            }
        }
    }

    #[test]
    fn every_field_byte_reaches_the_decoded_message() {
        let canonical = sample().encode();
        for index in 10..MESSAGE_LEN {
            let mut bytes = canonical.clone();
            bytes[index] ^= 0x01;
            assert_ne!(
                DeploymentAuthorization::decode(&bytes).unwrap(),
                sample(),
                "byte {index} is ignored"
            );
        }
    }

    fn context() -> DeploymentContext {
        let key = |byte: u8| Pubkey::new_from_array([byte; 32]);
        DeploymentContext {
            remote_leg_program: key(1),
            remote_config: key(2),
            deployment_id: [3; 32],
            vault_id: [4; 32],
            config_version: 5,
            strategy_config: key(6),
            adapter_program: key(7),
            adapter_state: key(8),
            adapter_authority: key(9),
            adapter_token_vault: key(10),
            max_remote_principal: 11,
            asset_mint: key(12),
            token_program: key(13),
            custody_authority: key(14),
            custody_token_account: key(15),
            deployed_principal: 16,
        }
    }

    #[test]
    fn every_context_field_changes_the_commitment() {
        let base = context();
        let other = Pubkey::new_from_array([0xEE; 32]);
        let mutations: [fn(&mut DeploymentContext, Pubkey); 16] = [
            |c, k| c.remote_leg_program = k,
            |c, k| c.remote_config = k,
            |c, _| c.deployment_id[0] ^= 1,
            |c, _| c.vault_id[31] ^= 1,
            |c, _| c.config_version ^= 1,
            |c, k| c.strategy_config = k,
            |c, k| c.adapter_program = k,
            |c, k| c.adapter_state = k,
            |c, k| c.adapter_authority = k,
            |c, k| c.adapter_token_vault = k,
            |c, _| c.max_remote_principal ^= 1,
            |c, k| c.asset_mint = k,
            |c, k| c.token_program = k,
            |c, k| c.custody_authority = k,
            |c, k| c.custody_token_account = k,
            |c, _| c.deployed_principal ^= 1,
        ];
        let mut seen = vec![base.commitment()];
        for mutate in mutations {
            let mut changed = base;
            mutate(&mut changed, other);
            let commitment = changed.commitment();
            assert!(!seen.contains(&commitment));
            seen.push(commitment);
        }
    }

    #[test]
    fn the_deploy_selector_is_the_anchor_hash_of_its_name() {
        let digest = solana_sha256_hasher::hash(b"global:deploy_to_strategy").to_bytes();
        assert_eq!(digest[..8], DEPLOY_TO_STRATEGY_DISCRIMINATOR);
    }
}
