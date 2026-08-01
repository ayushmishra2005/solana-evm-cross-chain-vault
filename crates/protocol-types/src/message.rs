use crate::body::Body;
use crate::error::DecodeError;
use crate::identifier::{
    ApplicationId, ChainId, Commitment, DeploymentId, Flags, LaneId, ProtocolVersion,
    SchemaVersion, Sequence, Timestamp, VaultId,
};
use crate::layout;

/// Kind of control message, with a wire discriminant that never changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MessageType {
    Allocate,
    Recall,
    RemoteReport,
    RecallSent,
    ConfigUpdate,
}

impl MessageType {
    /// Every kind, in discriminant order.
    pub const ALL: [Self; 5] = [
        Self::Allocate,
        Self::Recall,
        Self::RemoteReport,
        Self::RecallSent,
        Self::ConfigUpdate,
    ];

    #[must_use]
    pub const fn to_u16(self) -> u16 {
        match self {
            Self::Allocate => 1,
            Self::Recall => 2,
            Self::RemoteReport => 3,
            Self::RecallSent => 4,
            Self::ConfigUpdate => 5,
        }
    }

    /// Rejects zero, reserved and future discriminants.
    pub const fn from_u16(value: u16) -> Result<Self, DecodeError> {
        match value {
            1 => Ok(Self::Allocate),
            2 => Ok(Self::Recall),
            3 => Ok(Self::RemoteReport),
            4 => Ok(Self::RecallSent),
            5 => Ok(Self::ConfigUpdate),
            other => Err(DecodeError::UnknownMessageType(other)),
        }
    }

    /// Bytes the body of this kind always occupies.
    #[must_use]
    pub const fn body_len(self) -> usize {
        match self {
            Self::Allocate => layout::ALLOCATE_BODY_LEN,
            Self::Recall => layout::RECALL_BODY_LEN,
            Self::RemoteReport => layout::REMOTE_REPORT_BODY_LEN,
            Self::RecallSent => layout::RECALL_SENT_BODY_LEN,
            Self::ConfigUpdate => layout::CONFIG_UPDATE_BODY_LEN,
        }
    }

    /// Bytes a whole message of this kind always occupies.
    #[must_use]
    pub const fn message_len(self) -> usize {
        match self {
            Self::Allocate => layout::ALLOCATE_MESSAGE_LEN,
            Self::Recall => layout::RECALL_MESSAGE_LEN,
            Self::RemoteReport => layout::REMOTE_REPORT_MESSAGE_LEN,
            Self::RecallSent => layout::RECALL_SENT_MESSAGE_LEN,
            Self::ConfigUpdate => layout::CONFIG_UPDATE_MESSAGE_LEN,
        }
    }
}

/// Fields every message carries, whatever its kind.
///
/// The kind, body length and body hash are not stored here. They follow from
/// the body, and the codec writes and checks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub protocol_version: ProtocolVersion,
    pub schema_version: SchemaVersion,
    pub flags: Flags,
    pub source_chain: ChainId,
    pub destination_chain: ChainId,
    pub source_application: ApplicationId,
    pub destination_application: ApplicationId,
    pub deployment_id: DeploymentId,
    pub vault_id: VaultId,
    pub lane_id: LaneId,
    pub sequence: Sequence,
    pub previous_commitment: Commitment,
    pub observed_at: Timestamp,
    pub published_at: Timestamp,
    pub expires_at: Timestamp,
}

/// One canonical message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub header: Header,
    pub body: Body,
}

impl Message {
    #[must_use]
    pub const fn message_type(&self) -> MessageType {
        self.body.message_type()
    }

    /// Bytes this message occupies once encoded.
    #[must_use]
    pub const fn encoded_len(&self) -> usize {
        self.message_type().message_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_discriminant_survives_the_round_trip() {
        for kind in MessageType::ALL {
            assert_eq!(MessageType::from_u16(kind.to_u16()), Ok(kind));
        }
    }

    #[test]
    fn the_discriminants_are_the_documented_numbers() {
        assert_eq!(MessageType::Allocate.to_u16(), 1);
        assert_eq!(MessageType::Recall.to_u16(), 2);
        assert_eq!(MessageType::RemoteReport.to_u16(), 3);
        assert_eq!(MessageType::RecallSent.to_u16(), 4);
        assert_eq!(MessageType::ConfigUpdate.to_u16(), 5);
    }

    #[test]
    fn zero_and_reserved_discriminants_reject() {
        for value in [0u16, 6, 7, 255, 256, u16::MAX] {
            assert_eq!(
                MessageType::from_u16(value),
                Err(DecodeError::UnknownMessageType(value))
            );
        }
    }

    #[test]
    fn each_message_length_is_the_header_plus_its_body() {
        for kind in MessageType::ALL {
            assert_eq!(kind.message_len(), layout::HEADER_LEN + kind.body_len());
        }
    }

    #[test]
    fn every_body_length_is_distinct() {
        let mut lengths = MessageType::ALL.map(MessageType::body_len);
        lengths.sort_unstable();
        for pair in lengths.windows(2) {
            assert_ne!(pair.first(), pair.get(1));
        }
    }
}
