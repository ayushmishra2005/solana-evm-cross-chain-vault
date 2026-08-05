//! Fixed size state owned by the remote leg.

use anchor_lang::prelude::*;

/// Seed prefix of the configuration account.
pub const REMOTE_CONFIG_SEED: &[u8] = b"remote-config";

/// Seed prefix of the custody authority.
pub const CUSTODY_AUTHORITY_SEED: &[u8] = b"custody-authority";

/// Layout version written into every account this program owns.
pub const STATE_VERSION: u8 = 1;

/// Lowest configuration version an initialized leg may carry.
pub const MIN_CONFIG_VERSION: u64 = 1;

/// Decimals the supported asset must use.
pub const REQUIRED_MINT_DECIMALS: u8 = 6;

/// Bytes held back for later fields of the same layout version.
pub const REMOTE_CONFIG_RESERVED: usize = 64;

/// Immutable deployment settings plus the one mutable freeze flag.
#[account]
#[derive(InitSpace, Debug)]
pub struct RemoteConfig {
    pub state_version: u8,
    pub bump: u8,
    pub custody_authority_bump: u8,
    pub frozen: bool,
    pub administrator: Pubkey,
    pub emergency_guardian: Pubkey,
    pub transport_verifier: Pubkey,
    pub asset_mint: Pubkey,
    pub token_program: Pubkey,
    pub custody_authority: Pubkey,
    pub custody_token_account: Pubkey,
    pub outbound_escrow: Pubkey,
    pub source_chain_id: u32,
    pub destination_chain_id: u32,
    pub source_application_id: [u8; 32],
    pub local_application_id: [u8; 32],
    pub deployment_id: [u8; 32],
    pub vault_id: [u8; 32],
    pub control_lane_id: u32,
    pub report_lane_id: u32,
    pub config_version: u64,
    pub initialized_at: i64,
    pub reserved: [u8; REMOTE_CONFIG_RESERVED],
}

impl RemoteConfig {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Seeds of the configuration account for one deployment and vault.
    #[must_use]
    pub fn seeds<'a>(deployment_id: &'a [u8; 32], vault_id: &'a [u8; 32]) -> [&'a [u8]; 3] {
        [REMOTE_CONFIG_SEED, deployment_id, vault_id]
    }

    /// Address and bump of the custody authority for this configuration.
    #[must_use]
    pub fn custody_authority(config: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[CUSTODY_AUTHORITY_SEED, config.as_ref()], &crate::ID)
    }

    /// True when the signer may act as administrator or guardian.
    #[must_use]
    pub fn is_emergency_authority(&self, signer: &Pubkey) -> bool {
        signer == &self.administrator || signer == &self.emergency_guardian
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_account_has_a_fixed_documented_size() {
        assert_eq!(RemoteConfig::INIT_SPACE, 484);
        assert_eq!(RemoteConfig::LEN, 492);
    }

    #[test]
    fn the_account_discriminator_is_eight_bytes() {
        assert_eq!(RemoteConfig::DISCRIMINATOR.len(), 8);
    }

    #[test]
    fn every_configuration_field_keeps_its_documented_offset() {
        let config = sample_config();
        let mut bytes = Vec::new();
        config.serialize(&mut bytes).expect("config encodes");
        assert_eq!(bytes.len(), RemoteConfig::INIT_SPACE);

        #[track_caller]
        fn field(bytes: &[u8], offset: usize, expected: &[u8]) {
            assert_eq!(
                bytes.get(offset..offset + expected.len()),
                Some(expected),
                "field at offset {offset} moved"
            );
        }

        field(&bytes, 0, &[STATE_VERSION]);
        field(&bytes, 1, &[1]);
        field(&bytes, 2, &[2]);
        field(&bytes, 3, &[0]);
        field(&bytes, 4, &[0xA1; 32]);
        field(&bytes, 36, &[0xA2; 32]);
        field(&bytes, 68, &[0xA3; 32]);
        field(&bytes, 100, &[0xA4; 32]);
        field(&bytes, 132, &[0xA5; 32]);
        field(&bytes, 164, &[0xA6; 32]);
        field(&bytes, 196, &[0xA7; 32]);
        field(&bytes, 228, &[0xA8; 32]);
        field(&bytes, 260, &1u32.to_le_bytes());
        field(&bytes, 264, &2u32.to_le_bytes());
        field(&bytes, 268, &[1u8; 32]);
        field(&bytes, 300, &[2u8; 32]);
        field(&bytes, 332, &[3u8; 32]);
        field(&bytes, 364, &[4u8; 32]);
        field(&bytes, 396, &1u32.to_le_bytes());
        field(&bytes, 400, &2u32.to_le_bytes());
        field(&bytes, 404, &MIN_CONFIG_VERSION.to_le_bytes());
        field(&bytes, 412, &7i64.to_le_bytes());
        field(&bytes, 420, &[0u8; REMOTE_CONFIG_RESERVED]);
        assert_eq!(420 + REMOTE_CONFIG_RESERVED, RemoteConfig::INIT_SPACE);
    }

    #[test]
    fn the_reserved_bytes_stay_reserved() {
        let config = sample_config();
        assert_eq!(config.reserved, [0u8; REMOTE_CONFIG_RESERVED]);
        assert_eq!(REMOTE_CONFIG_RESERVED, 64);
    }

    fn sample_config() -> RemoteConfig {
        RemoteConfig {
            state_version: STATE_VERSION,
            bump: 1,
            custody_authority_bump: 2,
            frozen: false,
            administrator: Pubkey::new_from_array([0xA1; 32]),
            emergency_guardian: Pubkey::new_from_array([0xA2; 32]),
            transport_verifier: Pubkey::new_from_array([0xA3; 32]),
            asset_mint: Pubkey::new_from_array([0xA4; 32]),
            token_program: Pubkey::new_from_array([0xA5; 32]),
            custody_authority: Pubkey::new_from_array([0xA6; 32]),
            custody_token_account: Pubkey::new_from_array([0xA7; 32]),
            outbound_escrow: Pubkey::new_from_array([0xA8; 32]),
            source_chain_id: 1,
            destination_chain_id: 2,
            source_application_id: [1u8; 32],
            local_application_id: [2u8; 32],
            deployment_id: [3u8; 32],
            vault_id: [4u8; 32],
            control_lane_id: 1,
            report_lane_id: 2,
            config_version: MIN_CONFIG_VERSION,
            initialized_at: 7,
            reserved: [0u8; REMOTE_CONFIG_RESERVED],
        }
    }

    #[test]
    fn the_configuration_seeds_keep_their_documented_order() {
        let deployment = [7u8; 32];
        let vault = [9u8; 32];
        let seeds = RemoteConfig::seeds(&deployment, &vault);
        assert_eq!(seeds[0], REMOTE_CONFIG_SEED);
        assert_eq!(seeds[1], &deployment[..]);
        assert_eq!(seeds[2], &vault[..]);
    }

    #[test]
    fn only_the_administrator_and_the_guardian_are_emergency_authorities() {
        let administrator = Pubkey::new_unique();
        let emergency_guardian = Pubkey::new_unique();
        let config = RemoteConfig {
            state_version: STATE_VERSION,
            bump: 1,
            custody_authority_bump: 2,
            frozen: false,
            administrator,
            emergency_guardian,
            transport_verifier: Pubkey::new_unique(),
            asset_mint: Pubkey::new_unique(),
            token_program: Pubkey::new_unique(),
            custody_authority: Pubkey::new_unique(),
            custody_token_account: Pubkey::new_unique(),
            outbound_escrow: Pubkey::new_unique(),
            source_chain_id: 1,
            destination_chain_id: 2,
            source_application_id: [1u8; 32],
            local_application_id: [2u8; 32],
            deployment_id: [3u8; 32],
            vault_id: [4u8; 32],
            control_lane_id: 1,
            report_lane_id: 2,
            config_version: MIN_CONFIG_VERSION,
            initialized_at: 0,
            reserved: [0u8; REMOTE_CONFIG_RESERVED],
        };

        assert!(config.is_emergency_authority(&administrator));
        assert!(config.is_emergency_authority(&emergency_guardian));
        assert!(!config.is_emergency_authority(&Pubkey::new_unique()));
    }
}
