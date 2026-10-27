use anchor_lang::prelude::*;

pub const CONFIG_SEED: &[u8] = b"config";
pub const TRUSTED_KEY_SEED: &[u8] = b"trusted_key";
pub const MANDATE_SEED: &[u8] = b"mandate";
pub const RECEIPT_SEED: &[u8] = b"consumed";

pub const NONCE_DOMAIN: &[u8] = b"es256-authorization-verifier:nonce:v1";
pub const MANDATE_DOMAIN: &[u8] = b"es256-authorization-verifier:mandate:v1";
pub const JTI_DOMAIN: &[u8] = b"es256-authorization-verifier:jti:v1";

pub const ISSUER: &[u8] = b"https://verdicts.saifuro.com";
pub const ALLOW_VERDICT: &[u8] = b"allow";
pub const SANDBOX_KID_PREFIX: &[u8] = b"sandbox-";
/// Every verdict expires exactly this long after issue.
pub const VERDICT_LIFETIME_SECONDS: i64 = 300;
/// How far in the future `iat` may lie relative to the cluster clock.
pub const MAX_ISSUED_AT_SKEW_SECONDS: i64 = 60;

pub const MAX_KID_LEN: usize = 32;
pub const MAX_JTI_LEN: usize = 64;
pub const MAX_MANDATE_LEN: usize = 64;
pub const MAX_POLICY_LEN: usize = 64;
pub const MAX_NONCE_LEN: usize = 64;
pub const MAX_AGENT_LEN: usize = 64;
pub const MAX_CURRENCY_LEN: usize = 16;
pub const MAX_RECIPIENT_LEN: usize = 128;
pub const MAX_CATEGORY_LEN: usize = 32;

pub const COMPRESSED_PUBLIC_KEY_LEN: usize = 33;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum Environment {
    Production,
    Sandbox,
}

impl Environment {
    pub fn seed(self) -> [u8; 1] {
        match self {
            Self::Production => [0],
            Self::Sandbox => [1],
        }
    }

    /// Sandbox and production tokens share an issuer and claim shape; the
    /// kid prefix is the only signed field that tells them apart.
    pub fn accepts_kid(self, kid: &[u8]) -> bool {
        let sandbox = kid.starts_with(SANDBOX_KID_PREFIX);
        match self {
            Self::Production => !sandbox,
            Self::Sandbox => sandbox,
        }
    }
}

#[account]
#[derive(InitSpace)]
pub struct VerifierConfig {
    pub administrator: Pubkey,
    pub consumer: Pubkey,
    pub environment: Environment,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct TrustedKey {
    pub config: Pubkey,
    #[max_len(MAX_KID_LEN)]
    pub kid: String,
    pub public_key: [u8; COMPRESSED_PUBLIC_KEY_LEN],
    /// Earliest accepted `iat` and cluster time.
    pub not_before: i64,
    /// Latest accepted `exp`.
    pub not_after: i64,
    pub revoked: bool,
    pub bump: u8,
}

/// The exact authorization context an administrator approved. It has no
/// update path; a different context needs a different mandate account.
#[account]
#[derive(InitSpace)]
pub struct Mandate {
    pub config: Pubkey,
    pub mandate_hash: [u8; 32],
    #[max_len(MAX_MANDATE_LEN)]
    pub mandate_id: String,
    /// Signed `request.amount` scaled by 10^6.
    pub amount_micros: u64,
    #[max_len(MAX_CURRENCY_LEN)]
    pub currency: String,
    #[max_len(MAX_RECIPIENT_LEN)]
    pub recipient: String,
    #[max_len(MAX_POLICY_LEN)]
    pub policy_version: String,
    #[max_len(MAX_AGENT_LEN)]
    pub agent: String,
    #[max_len(MAX_CATEGORY_LEN)]
    pub category: String,
    pub revoked: bool,
    pub bump: u8,
}

/// Permanent evidence that one nonce was consumed under one config.
#[account]
#[derive(InitSpace)]
pub struct ConsumedAuthorization {
    pub config: Pubkey,
    pub nonce_hash: [u8; 32],
    pub signing_input_hash: [u8; 32],
    pub jti_hash: [u8; 32],
    pub mandate: Pubkey,
    pub consumed_at: i64,
    pub expires_at: i64,
    pub bump: u8,
}

pub fn domain_hash(domain: &[u8], value: &[u8]) -> [u8; 32] {
    solana_sha256_hasher::hashv(&[domain, value]).to_bytes()
}

pub fn nonce_hash(nonce: &[u8]) -> [u8; 32] {
    domain_hash(NONCE_DOMAIN, nonce)
}

pub fn mandate_hash(mandate_id: &[u8]) -> [u8; 32] {
    domain_hash(MANDATE_DOMAIN, mandate_id)
}

/// The trusted-key address seed for `kid`. Anchor derives the address
/// before the handler runs, and a seed over 32 bytes aborts derivation.
/// Longer kids all map to the empty seed, which no valid kid can occupy, so
/// they reach the handler and are rejected there instead of colliding with
/// a registered prefix.
pub fn kid_seed(kid: &str) -> &[u8] {
    let bytes = kid.as_bytes();
    if bytes.len() > MAX_KID_LEN {
        &[]
    } else {
        bytes
    }
}

/// Printable ASCII without quote or backslash: the only string bytes the
/// token parser accepts, so registered values can always be matched.
pub fn is_profile_text(value: &[u8], max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .iter()
            .all(|byte| matches!(byte, 0x20..=0x7e) && *byte != b'"' && *byte != b'\\')
}
