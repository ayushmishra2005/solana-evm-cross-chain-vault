//! Fixed size control plane state: risk limits and replay lanes.

use anchor_lang::prelude::*;

use crate::errors::RemoteLegError;

/// Seed prefix of the risk configuration account.
pub const RISK_CONFIG_SEED: &[u8] = b"risk-config";

/// Seed prefix of a replay lane.
pub const REPLAY_LANE_SEED: &[u8] = b"replay-lane";

/// Seed prefix of a consumed message record.
pub const CONSUMED_MESSAGE_SEED: &[u8] = b"consumed-message";

/// Largest basis point value the protocol accepts.
pub const MAX_BASIS_POINTS: u16 = 10_000;

/// Bytes held back for later fields of the same layout version.
pub const RISK_CONFIG_RESERVED: usize = 32;

/// Inbound message classes that keep separate replay state.
///
/// The discriminant is the seed byte, so it must never be reordered.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum MessageClass {
    Allocate = 0,
    Recall = 1,
    ConfigUpdate = 2,
}

impl MessageClass {
    /// Every class, in discriminant order.
    pub const ALL: [Self; 3] = [Self::Allocate, Self::Recall, Self::ConfigUpdate];

    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// The wire message type this class accepts.
    #[must_use]
    pub const fn message_type(self) -> protocol_types::MessageType {
        match self {
            Self::Allocate => protocol_types::MessageType::Allocate,
            Self::Recall => protocol_types::MessageType::Recall,
            Self::ConfigUpdate => protocol_types::MessageType::ConfigUpdate,
        }
    }
}

/// Risk limits the canonical vault controls through config messages.
#[account]
#[derive(InitSpace, Debug)]
pub struct RiskConfig {
    pub state_version: u8,
    pub bump: u8,
    pub max_remote_allocation_bps: u16,
    pub max_upward_deviation_bps: u16,
    pub max_downward_deviation_bps: u16,
    pub max_report_age: u64,
    pub config_version: u64,
    pub config_commitment: [u8; 32],
    pub initialized_at: i64,
    pub last_update_at: i64,
    pub reserved: [u8; RISK_CONFIG_RESERVED],
}

impl RiskConfig {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Seeds of the risk configuration for one remote leg.
    #[must_use]
    pub fn seeds(remote_config: &Pubkey) -> [&[u8]; 2] {
        [RISK_CONFIG_SEED, remote_config.as_ref()]
    }

    /// Rejects any risk parameter outside its documented range.
    pub fn check_parameters(
        max_remote_allocation_bps: u16,
        max_upward_deviation_bps: u16,
        max_downward_deviation_bps: u16,
        max_report_age: u64,
        config_commitment: &[u8; 32],
    ) -> Result<()> {
        for rate in [
            max_remote_allocation_bps,
            max_upward_deviation_bps,
            max_downward_deviation_bps,
        ] {
            require_gte!(MAX_BASIS_POINTS, rate, RemoteLegError::InvalidBasisPoints);
        }
        require_neq!(max_report_age, 0, RemoteLegError::InvalidReportAge);
        require!(
            config_commitment != &[0u8; 32],
            RemoteLegError::InvalidConfigCommitment
        );
        Ok(())
    }
}

/// Replay state of one inbound message class.
#[account]
#[derive(InitSpace, Debug)]
pub struct ReplayLane {
    pub state_version: u8,
    pub bump: u8,
    pub message_class: MessageClass,
    pub lane_id: u32,
    pub minimum_acceptable_sequence: u64,
    pub highest_consumed_sequence: u64,
    pub message_commitment: [u8; 32],
    pub mandatory_watermark_lag: u64,
    pub last_accepted_at: i64,
}

impl ReplayLane {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Sequence every lane starts accepting from.
    pub const FIRST_SEQUENCE: u64 = 1;

    /// Seeds of one lane, given the class byte and lane id bytes.
    #[must_use]
    pub fn seeds<'a>(
        remote_config: &'a Pubkey,
        message_class: &'a [u8; 1],
        lane_id: &'a [u8; 4],
    ) -> [&'a [u8]; 4] {
        [
            REPLAY_LANE_SEED,
            remote_config.as_ref(),
            message_class,
            lane_id,
        ]
    }
}

/// Proof that one exact message was already applied.
#[account]
#[derive(InitSpace, Debug)]
pub struct ConsumedMessage {
    pub state_version: u8,
    pub bump: u8,
    pub message_class: MessageClass,
    pub lane_id: u32,
    pub sequence: u64,
    pub message_id: [u8; 32],
}

impl ConsumedMessage {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Seeds of one record, given the class, lane and sequence bytes.
    #[must_use]
    pub fn seeds<'a>(
        remote_config: &'a Pubkey,
        message_class: &'a [u8; 1],
        lane_id: &'a [u8; 4],
        sequence: &'a [u8; 8],
    ) -> [&'a [u8]; 5] {
        [
            CONSUMED_MESSAGE_SEED,
            remote_config.as_ref(),
            message_class,
            lane_id,
            sequence,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_account_has_a_fixed_documented_size() {
        assert_eq!(RiskConfig::INIT_SPACE, 104);
        assert_eq!(RiskConfig::LEN, 112);
        assert_eq!(ReplayLane::INIT_SPACE, 71);
        assert_eq!(ReplayLane::LEN, 79);
        assert_eq!(ConsumedMessage::INIT_SPACE, 47);
        assert_eq!(ConsumedMessage::LEN, 55);
    }

    #[test]
    fn the_message_class_byte_never_changes() {
        assert_eq!(MessageClass::Allocate.to_u8(), 0);
        assert_eq!(MessageClass::Recall.to_u8(), 1);
        assert_eq!(MessageClass::ConfigUpdate.to_u8(), 2);
    }

    #[test]
    fn every_message_class_maps_to_its_wire_type() {
        assert_eq!(
            MessageClass::Allocate.message_type(),
            protocol_types::MessageType::Allocate
        );
        assert_eq!(
            MessageClass::Recall.message_type(),
            protocol_types::MessageType::Recall
        );
        assert_eq!(
            MessageClass::ConfigUpdate.message_type(),
            protocol_types::MessageType::ConfigUpdate
        );
    }

    #[test]
    fn a_message_class_serializes_as_one_byte() {
        for class in MessageClass::ALL {
            let mut bytes = Vec::new();
            class.serialize(&mut bytes).expect("class serializes");
            assert_eq!(bytes, vec![class.to_u8()]);
        }
    }

    #[test]
    fn the_three_message_classes_are_distinct() {
        let mut seen = MessageClass::ALL.map(MessageClass::to_u8);
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2]);
    }

    #[test]
    fn the_lane_seeds_keep_their_documented_order() {
        let config = Pubkey::new_unique();
        let class = [MessageClass::Recall.to_u8()];
        let lane = 7u32.to_le_bytes();
        let seeds = ReplayLane::seeds(&config, &class, &lane);
        assert_eq!(seeds[0], REPLAY_LANE_SEED);
        assert_eq!(seeds[1], config.as_ref());
        assert_eq!(seeds[2], &class[..]);
        assert_eq!(seeds[3], &lane[..]);
    }

    #[test]
    fn the_consumed_record_seeds_keep_their_documented_order() {
        let config = Pubkey::new_unique();
        let class = [MessageClass::ConfigUpdate.to_u8()];
        let lane = 7u32.to_le_bytes();
        let sequence = 42u64.to_le_bytes();
        let seeds = ConsumedMessage::seeds(&config, &class, &lane, &sequence);
        assert_eq!(seeds[0], CONSUMED_MESSAGE_SEED);
        assert_eq!(seeds[1], config.as_ref());
        assert_eq!(seeds[2], &class[..]);
        assert_eq!(seeds[3], &lane[..]);
        assert_eq!(seeds[4], &sequence[..]);
    }

    #[test]
    fn the_same_sequence_in_two_classes_gives_two_addresses() {
        let config = Pubkey::new_unique();
        let lane = 1u32.to_le_bytes();
        let sequence = 5u64.to_le_bytes();
        let allocate = [MessageClass::Allocate.to_u8()];
        let recall = [MessageClass::Recall.to_u8()];
        let first = Pubkey::find_program_address(
            &ConsumedMessage::seeds(&config, &allocate, &lane, &sequence),
            &crate::ID,
        );
        let second = Pubkey::find_program_address(
            &ConsumedMessage::seeds(&config, &recall, &lane, &sequence),
            &crate::ID,
        );
        assert_ne!(first.0, second.0);
    }

    #[test]
    fn the_same_lane_in_two_classes_gives_two_addresses() {
        let config = Pubkey::new_unique();
        let lane = 1u32.to_le_bytes();
        let allocate = [MessageClass::Allocate.to_u8()];
        let config_update = [MessageClass::ConfigUpdate.to_u8()];
        let first =
            Pubkey::find_program_address(&ReplayLane::seeds(&config, &allocate, &lane), &crate::ID);
        let second = Pubkey::find_program_address(
            &ReplayLane::seeds(&config, &config_update, &lane),
            &crate::ID,
        );
        assert_ne!(first.0, second.0);
    }

    #[test]
    fn valid_risk_parameters_are_accepted() {
        assert!(RiskConfig::check_parameters(10_000, 10_000, 10_000, 1, &[1u8; 32]).is_ok());
    }

    #[test]
    fn basis_points_above_ten_thousand_are_rejected() {
        for index in 0..3 {
            let mut rates = [1_000u16; 3];
            rates[index] = 10_001;
            let error = RiskConfig::check_parameters(rates[0], rates[1], rates[2], 1, &[1u8; 32])
                .expect_err("out of range basis points should reject");
            assert_eq!(error, Error::from(RemoteLegError::InvalidBasisPoints));
        }
    }

    #[test]
    fn a_zero_report_age_is_rejected() {
        let error = RiskConfig::check_parameters(1, 1, 1, 0, &[1u8; 32])
            .expect_err("zero report age should reject");
        assert_eq!(error, Error::from(RemoteLegError::InvalidReportAge));
    }

    #[test]
    fn a_zero_config_commitment_is_rejected() {
        let error = RiskConfig::check_parameters(1, 1, 1, 1, &[0u8; 32])
            .expect_err("zero commitment should reject");
        assert_eq!(error, Error::from(RemoteLegError::InvalidConfigCommitment));
    }
}
