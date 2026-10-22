//! Fixed size state for the one ordered outbound report lane.

use anchor_lang::prelude::*;

use crate::errors::RemoteLegError;

/// Seed prefix of the outbound lane.
pub const OUTBOUND_LANE_SEED: &[u8] = b"outbound-lane";

/// Seed prefix of one published report.
pub const REPORT_RECORD_SEED: &[u8] = b"report-record";

/// Seed prefix of the marker that makes a recall notice publish once.
pub const RECALL_SENT_MARKER_SEED: &[u8] = b"recall-sent-marker";

/// Sequence the lane assigns to its first message.
pub const FIRST_OUTBOUND_SEQUENCE: u64 = 1;

/// Which canonical message one record stands for.
///
/// Zero stays unused, so a zeroed account never reads as a valid type.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum ReportType {
    RemoteReport = 1,
    RecallSent = 2,
    TransportSettlement = 3,
}

impl ReportType {
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }
}

/// One total order shared by every outbound message.
#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct OutboundLane {
    pub state_version: u8,
    pub bump: u8,
    pub lane_id: u32,
    pub next_sequence: u64,
    pub current_commitment: [u8; 32],
    pub last_publication_at: i64,
    pub total_reports: u64,
    pub total_recall_notifications: u64,
    pub initialized_at: i64,
}

impl OutboundLane {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Address and bump of the lane for one configuration.
    #[must_use]
    pub fn address(remote_config: &Pubkey, lane_id: u32) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                OUTBOUND_LANE_SEED,
                remote_config.as_ref(),
                &lane_id.to_le_bytes(),
            ],
            &crate::ID,
        )
    }

    /// Moves the lane on by one accepted message.
    ///
    /// Only a publication that already succeeded may call this.
    pub fn advance(
        &mut self,
        report_type: ReportType,
        next_commitment: [u8; 32],
        published_at: i64,
    ) -> Result<()> {
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(RemoteLegError::ArithmeticOverflow)?;
        self.current_commitment = next_commitment;
        self.last_publication_at = published_at;

        // The lane keeps its fixed size, so settlements are counted on the
        // transport ledger instead.
        let counter = match report_type {
            ReportType::RemoteReport => &mut self.total_reports,
            ReportType::RecallSent => &mut self.total_recall_notifications,
            ReportType::TransportSettlement => return Ok(()),
        };
        *counter = counter
            .checked_add(1)
            .ok_or(RemoteLegError::ArithmeticOverflow)?;
        Ok(())
    }
}

/// Proof that one exact message left the leg at one exact sequence.
#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct ReportRecord {
    pub state_version: u8,
    pub bump: u8,
    pub report_type: ReportType,
    pub lane_id: u32,
    pub sequence: u64,
    pub message_id: [u8; 32],
    pub body_hash: [u8; 32],
    pub previous_commitment: [u8; 32],
    pub next_commitment: [u8; 32],
    pub encoded_len: u16,
    pub published_at: i64,
    pub related_transfer_id: [u8; 32],
    pub epoch_id: u64,
}

impl ReportRecord {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Address and bump of the record for one lane sequence.
    #[must_use]
    pub fn address(remote_config: &Pubkey, lane_id: u32, sequence: u64) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                REPORT_RECORD_SEED,
                remote_config.as_ref(),
                &lane_id.to_le_bytes(),
                &sequence.to_le_bytes(),
            ],
            &crate::ID,
        )
    }
}

/// Records that one recall already produced its terminal notice.
#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct RecallSentMarker {
    pub state_version: u8,
    pub bump: u8,
    pub transfer_id: [u8; 32],
    pub report_sequence: u64,
    pub message_id: [u8; 32],
    pub published_at: i64,
}

impl RecallSentMarker {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Address and bump of the marker for one transfer.
    #[must_use]
    pub fn address(remote_config: &Pubkey, transfer_id: &[u8; 32]) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[RECALL_SENT_MARKER_SEED, remote_config.as_ref(), transfer_id],
            &crate::ID,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_outbound_account_has_a_fixed_documented_size() {
        assert_eq!(OutboundLane::INIT_SPACE, 78);
        assert_eq!(OutboundLane::LEN, 86);
        assert_eq!(ReportRecord::INIT_SPACE, 193);
        assert_eq!(ReportRecord::LEN, 201);
        assert_eq!(RecallSentMarker::INIT_SPACE, 82);
        assert_eq!(RecallSentMarker::LEN, 90);
    }

    #[test]
    fn the_report_type_discriminants_are_the_documented_numbers() {
        assert_eq!(ReportType::RemoteReport.to_u8(), 1);
        assert_eq!(ReportType::RecallSent.to_u8(), 2);
    }

    fn lane() -> OutboundLane {
        OutboundLane {
            state_version: 1,
            bump: 254,
            lane_id: 7,
            next_sequence: FIRST_OUTBOUND_SEQUENCE,
            current_commitment: [0u8; 32],
            last_publication_at: 0,
            total_reports: 0,
            total_recall_notifications: 0,
            initialized_at: 100,
        }
    }

    #[test]
    fn advancing_moves_the_sequence_the_commitment_and_one_counter() {
        let mut state = lane();
        state
            .advance(ReportType::RemoteReport, [9u8; 32], 500)
            .expect("the lane advances");

        assert_eq!(state.next_sequence, 2);
        assert_eq!(state.current_commitment, [9u8; 32]);
        assert_eq!(state.last_publication_at, 500);
        assert_eq!(state.total_reports, 1);
        assert_eq!(state.total_recall_notifications, 0);
    }

    #[test]
    fn each_message_type_counts_on_its_own() {
        let mut state = lane();
        state
            .advance(ReportType::RecallSent, [1u8; 32], 500)
            .expect("the lane advances");
        state
            .advance(ReportType::RemoteReport, [2u8; 32], 600)
            .expect("the lane advances");
        state
            .advance(ReportType::RecallSent, [3u8; 32], 700)
            .expect("the lane advances");

        assert_eq!(state.next_sequence, 4);
        assert_eq!(state.total_reports, 1);
        assert_eq!(state.total_recall_notifications, 2);
    }

    #[test]
    fn a_sequence_at_the_type_maximum_refuses_to_advance() {
        let mut state = OutboundLane {
            next_sequence: u64::MAX,
            ..lane()
        };
        assert_eq!(
            state.advance(ReportType::RemoteReport, [1u8; 32], 500),
            Err(RemoteLegError::ArithmeticOverflow.into())
        );
    }

    #[test]
    fn a_counter_at_the_type_maximum_refuses_to_advance() {
        let mut state = OutboundLane {
            total_reports: u64::MAX,
            ..lane()
        };
        assert_eq!(
            state.advance(ReportType::RemoteReport, [1u8; 32], 500),
            Err(RemoteLegError::ArithmeticOverflow.into())
        );
    }

    #[test]
    fn each_record_address_is_unique_per_sequence() {
        let config = Pubkey::new_unique();
        let first = ReportRecord::address(&config, 7, 1).0;
        let second = ReportRecord::address(&config, 7, 2).0;
        let other_lane = ReportRecord::address(&config, 8, 1).0;

        assert_ne!(first, second);
        assert_ne!(first, other_lane);
    }

    #[test]
    fn each_marker_address_is_unique_per_transfer() {
        let config = Pubkey::new_unique();
        assert_ne!(
            RecallSentMarker::address(&config, &[1u8; 32]).0,
            RecallSentMarker::address(&config, &[2u8; 32]).0
        );
    }
}
