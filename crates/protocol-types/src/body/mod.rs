//! Fixed width message bodies.

mod allocate;
mod config_update;
mod recall;
mod recall_sent;
mod remote_report;
mod resolve;
mod transport_settlement;

pub use allocate::AllocateBody;
pub use config_update::ConfigUpdateBody;
pub use recall::RecallBody;
pub use recall_sent::RecallSentBody;
pub use remote_report::{ProbeStatus, RemoteReportBody};
pub use resolve::{ResolveAction, ResolveBody};
pub use transport_settlement::{SettlementKind, TransportSettlementBody};

use crate::error::{DecodeError, EncodeError, ValidationError};
use crate::identifier::{Sequence, Timestamp};
use crate::message::MessageType;

/// Envelope times a body is checked against.
///
/// Both are carried so a body event can never sit after its observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EnvelopeTimes {
    pub observed_at: Timestamp,
    pub published_at: Timestamp,
}

/// The body of one message, tagged by its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    Allocate(AllocateBody),
    Recall(RecallBody),
    RemoteReport(RemoteReportBody),
    RecallSent(RecallSentBody),
    ConfigUpdate(ConfigUpdateBody),
    Resolve(ResolveBody),
    TransportSettlement(TransportSettlementBody),
}

impl Body {
    #[must_use]
    pub const fn message_type(&self) -> MessageType {
        match self {
            Self::Allocate(_) => MessageType::Allocate,
            Self::Recall(_) => MessageType::Recall,
            Self::RemoteReport(_) => MessageType::RemoteReport,
            Self::RecallSent(_) => MessageType::RecallSent,
            Self::ConfigUpdate(_) => MessageType::ConfigUpdate,
            Self::Resolve(_) => MessageType::Resolve,
            Self::TransportSettlement(_) => MessageType::TransportSettlement,
        }
    }

    /// Bytes this body always occupies.
    #[must_use]
    pub const fn encoded_len(&self) -> usize {
        self.message_type().body_len()
    }

    pub(crate) fn encode_into(&self, out: &mut [u8]) -> Result<(), EncodeError> {
        match self {
            Self::Allocate(body) => body.encode_into(out),
            Self::Recall(body) => body.encode_into(out),
            Self::RemoteReport(body) => body.encode_into(out),
            Self::RecallSent(body) => body.encode_into(out),
            Self::ConfigUpdate(body) => body.encode_into(out),
            Self::Resolve(body) => body.encode_into(out),
            Self::TransportSettlement(body) => body.encode_into(out),
        }
    }

    pub(crate) fn decode(kind: MessageType, bytes: &[u8]) -> Result<Self, DecodeError> {
        match kind {
            MessageType::Allocate => AllocateBody::decode(bytes).map(Self::Allocate),
            MessageType::Recall => RecallBody::decode(bytes).map(Self::Recall),
            MessageType::RemoteReport => RemoteReportBody::decode(bytes).map(Self::RemoteReport),
            MessageType::RecallSent => RecallSentBody::decode(bytes).map(Self::RecallSent),
            MessageType::ConfigUpdate => ConfigUpdateBody::decode(bytes).map(Self::ConfigUpdate),
            MessageType::Resolve => ResolveBody::decode(bytes).map(Self::Resolve),
            MessageType::TransportSettlement => {
                TransportSettlementBody::decode(bytes).map(Self::TransportSettlement)
            }
        }
    }

    /// The sequence is the one in the header, so a body can name earlier messages.
    pub(crate) fn validate(
        &self,
        times: EnvelopeTimes,
        sequence: Sequence,
    ) -> Result<(), ValidationError> {
        match self {
            Self::Allocate(body) => body.validate(times.published_at),
            Self::Recall(body) => body.validate(times.published_at),
            Self::RemoteReport(body) => body.validate(times),
            Self::RecallSent(body) => body.validate(times),
            Self::ConfigUpdate(body) => body.validate(times.published_at),
            Self::Resolve(body) => body.validate(sequence),
            Self::TransportSettlement(body) => body.validate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout;

    #[test]
    fn each_body_reports_the_length_of_its_own_kind() {
        let cases = [
            (
                Body::Allocate(AllocateBody::sample()),
                layout::ALLOCATE_BODY_LEN,
            ),
            (Body::Recall(RecallBody::sample()), layout::RECALL_BODY_LEN),
            (
                Body::RemoteReport(RemoteReportBody::sample()),
                layout::REMOTE_REPORT_BODY_LEN,
            ),
            (
                Body::RecallSent(RecallSentBody::sample()),
                layout::RECALL_SENT_BODY_LEN,
            ),
            (
                Body::ConfigUpdate(ConfigUpdateBody::sample()),
                layout::CONFIG_UPDATE_BODY_LEN,
            ),
            (
                Body::Resolve(ResolveBody::sample()),
                layout::RESOLVE_BODY_LEN,
            ),
            (
                Body::TransportSettlement(TransportSettlementBody::sample()),
                layout::TRANSPORT_SETTLEMENT_BODY_LEN,
            ),
        ];
        for (body, expected) in cases {
            assert_eq!(body.encoded_len(), expected);
            assert_eq!(body.message_type().body_len(), expected);
        }
    }

    #[test]
    fn decoding_a_shorter_kind_from_a_longer_body_still_reads_a_body() {
        let mut bytes = [0u8; layout::MAX_BODY_LEN];
        assert_eq!(RemoteReportBody::sample().encode_into(&mut bytes), Ok(()));
        let decoded = Body::decode(MessageType::Recall, &bytes);
        assert!(matches!(decoded, Ok(Body::Recall(_))));
    }

    #[test]
    fn decoding_a_longer_kind_from_a_shorter_body_rejects() {
        let mut bytes = [0u8; layout::RECALL_BODY_LEN];
        assert_eq!(RecallBody::sample().encode_into(&mut bytes), Ok(()));
        assert!(Body::decode(MessageType::RemoteReport, &bytes).is_err());
    }
}
