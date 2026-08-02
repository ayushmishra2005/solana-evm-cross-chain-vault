//! Typed record of everything the simulator did.
//!
//! Records are values, not log lines. `Display` exists only to make a failing
//! test readable.

extern crate alloc;

use alloc::vec::Vec;

use protocol_types::{AssetAmount, MessageId, TransferId};

use crate::endpoint::EndpointId;
use crate::event::EventId;
use crate::fault::FaultId;
use crate::lane::Lane;
use crate::time::Tick;

/// Position of one record inside the trace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceIndex(u32);

impl TraceIndex {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl core::fmt::Display for TraceIndex {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "[{}]", self.0)
    }
}

/// What a message or transfer is about, when the record names one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    Message(MessageId),
    Transfer(TransferId),
}

/// Why a delivery attempt could not finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlockReason {
    EndpointHalted,
    LanePaused,
}

/// Why the simulator refused to apply a declared fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RejectReason {
    CorruptOffsetOutOfRange,
    CorruptMaskIsZero,
    AlreadyCorrupted,
    SplitExceedsRequest,
    SplitPieceIsZero,
    SplitHasNoPieces,
    AmountIsZero,
    AmountExceedsRequest,
    OverDeliveryNotAboveRequest,
    DuplicateCopiesIsZero,
    ConflictingGroup,
    UnknownEndpoint,
}

impl RejectReason {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::CorruptOffsetOutOfRange => 1,
            Self::CorruptMaskIsZero => 2,
            Self::AlreadyCorrupted => 12,
            Self::SplitExceedsRequest => 3,
            Self::SplitPieceIsZero => 4,
            Self::SplitHasNoPieces => 5,
            Self::AmountIsZero => 6,
            Self::AmountExceedsRequest => 7,
            Self::OverDeliveryNotAboveRequest => 8,
            Self::DuplicateCopiesIsZero => 9,
            Self::ConflictingGroup => 10,
            Self::UnknownEndpoint => 11,
        }
    }
}

/// The concrete change a fault made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultEffect {
    Delayed { to: Tick },
    Dropped,
    Duplicated { copies: u16 },
    Rerouted { to: EndpointId },
    Corrupted { offset: usize, from: u8, to: u8 },
    AmountSet { to: AssetAmount },
    SplitInto { pieces: u16 },
}

/// One thing that happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceAction {
    EventScheduled {
        event: EventId,
        lane: Lane,
        source: EndpointId,
        destination: EndpointId,
        subject: Subject,
        deliver_at: Tick,
    },
    FaultApplied {
        fault: FaultId,
        event: EventId,
        effect: FaultEffect,
    },
    FaultRejected {
        fault: FaultId,
        event: EventId,
        reason: RejectReason,
    },
    TickAdvanced {
        from: Tick,
        to: Tick,
    },
    EndpointHalted {
        endpoint: EndpointId,
    },
    EndpointResumed {
        endpoint: EndpointId,
        released: u32,
    },
    DeliveryAttempted {
        event: EventId,
        destination: EndpointId,
        attempt: u32,
    },
    DeliveryBlocked {
        event: EventId,
        destination: EndpointId,
        reason: BlockReason,
    },
    EventDelivered {
        event: EventId,
        destination: EndpointId,
        subject: Subject,
        after_deadline: bool,
    },
    EventDropped {
        event: EventId,
        fault: Option<FaultId>,
    },
    EventExpired {
        event: EventId,
        deadline: Tick,
    },
    EventRejected {
        event: EventId,
        reason: RejectReason,
    },
    DuplicateCreated {
        original: EventId,
        duplicate: EventId,
        deliver_at: Tick,
    },
    PartialDeliveryCreated {
        original: EventId,
        piece_event: EventId,
        transfer: TransferId,
        amount: AssetAmount,
        piece: u16,
    },
    EventReordered {
        event: EventId,
        from: Tick,
        to: Tick,
    },
    LanePaused {
        lane: Lane,
    },
    LaneResumed {
        lane: Lane,
        released: u32,
    },
}

impl TraceAction {
    /// Stable number of the action kind.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::EventScheduled { .. } => 1,
            Self::FaultApplied { .. } => 2,
            Self::FaultRejected { .. } => 3,
            Self::TickAdvanced { .. } => 4,
            Self::EndpointHalted { .. } => 5,
            Self::EndpointResumed { .. } => 6,
            Self::DeliveryAttempted { .. } => 7,
            Self::DeliveryBlocked { .. } => 8,
            Self::EventDelivered { .. } => 9,
            Self::EventDropped { .. } => 10,
            Self::EventExpired { .. } => 11,
            Self::EventRejected { .. } => 12,
            Self::DuplicateCreated { .. } => 13,
            Self::PartialDeliveryCreated { .. } => 14,
            Self::EventReordered { .. } => 15,
            Self::LanePaused { .. } => 16,
            Self::LaneResumed { .. } => 17,
        }
    }

    /// The event this record is about, when it names one.
    #[must_use]
    pub const fn event(&self) -> Option<EventId> {
        match self {
            Self::EventScheduled { event, .. }
            | Self::FaultApplied { event, .. }
            | Self::FaultRejected { event, .. }
            | Self::DeliveryAttempted { event, .. }
            | Self::DeliveryBlocked { event, .. }
            | Self::EventDelivered { event, .. }
            | Self::EventDropped { event, .. }
            | Self::EventExpired { event, .. }
            | Self::EventRejected { event, .. }
            | Self::EventReordered { event, .. } => Some(*event),
            Self::DuplicateCreated { original, .. }
            | Self::PartialDeliveryCreated { original, .. } => Some(*original),
            Self::TickAdvanced { .. }
            | Self::EndpointHalted { .. }
            | Self::EndpointResumed { .. }
            | Self::LanePaused { .. }
            | Self::LaneResumed { .. } => None,
        }
    }

    /// The message or transfer this record is about, when it names one.
    #[must_use]
    pub const fn subject(&self) -> Option<Subject> {
        match self {
            Self::EventScheduled { subject, .. } | Self::EventDelivered { subject, .. } => {
                Some(*subject)
            }
            Self::PartialDeliveryCreated { transfer, .. } => Some(Subject::Transfer(*transfer)),
            _ => None,
        }
    }
}

/// One trace record with its position and the tick it happened on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceRecord {
    pub index: TraceIndex,
    pub tick: Tick,
    pub action: TraceAction,
}

impl core::fmt::Display for TraceRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "{} {} ", self.index, self.tick)?;
        match self.action {
            TraceAction::EventScheduled {
                event,
                lane,
                source,
                destination,
                deliver_at,
                ..
            } => write!(
                formatter,
                "scheduled {event} {lane} {source}->{destination} at {deliver_at}"
            ),
            TraceAction::FaultApplied {
                fault,
                event,
                effect,
            } => write!(formatter, "fault {fault} on {event} {effect:?}"),
            TraceAction::FaultRejected {
                fault,
                event,
                reason,
            } => write!(formatter, "fault {fault} on {event} refused {reason:?}"),
            TraceAction::TickAdvanced { from, to } => write!(formatter, "time {from}->{to}"),
            TraceAction::EndpointHalted { endpoint } => write!(formatter, "halt {endpoint}"),
            TraceAction::EndpointResumed { endpoint, released } => {
                write!(formatter, "resume {endpoint} released {released}")
            }
            TraceAction::DeliveryAttempted {
                event,
                destination,
                attempt,
            } => write!(formatter, "attempt {attempt} {event} to {destination}"),
            TraceAction::DeliveryBlocked {
                event,
                destination,
                reason,
            } => write!(formatter, "blocked {event} at {destination} {reason:?}"),
            TraceAction::EventDelivered {
                event,
                destination,
                after_deadline,
                ..
            } => write!(
                formatter,
                "delivered {event} to {destination} late={after_deadline}"
            ),
            TraceAction::EventDropped { event, fault } => match fault {
                Some(id) => write!(formatter, "dropped {event} by {id}"),
                None => write!(formatter, "dropped {event}"),
            },
            TraceAction::EventExpired { event, deadline } => {
                write!(formatter, "expired {event} past {deadline}")
            }
            TraceAction::EventRejected { event, reason } => {
                write!(formatter, "refused {event} {reason:?}")
            }
            TraceAction::DuplicateCreated {
                original,
                duplicate,
                deliver_at,
            } => write!(
                formatter,
                "duplicate {duplicate} of {original} at {deliver_at}"
            ),
            TraceAction::PartialDeliveryCreated {
                original,
                piece_event,
                amount,
                piece,
                ..
            } => write!(
                formatter,
                "piece {piece} {piece_event} of {original} amount {}",
                amount.get()
            ),
            TraceAction::EventReordered { event, from, to } => {
                write!(formatter, "reorder {event} {from}->{to}")
            }
            TraceAction::LanePaused { lane } => write!(formatter, "pause {lane}"),
            TraceAction::LaneResumed { lane, released } => {
                write!(formatter, "resume {lane} released {released}")
            }
        }
    }
}

/// Every record in the order it was written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trace {
    records: Vec<TraceRecord>,
}

impl Trace {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[must_use]
    pub fn records(&self) -> &[TraceRecord] {
        &self.records
    }

    /// Records added after an earlier length was noted.
    #[must_use]
    pub fn since(&self, start: usize) -> &[TraceRecord] {
        self.records.get(start..).unwrap_or(&[])
    }

    pub(crate) fn push(&mut self, tick: Tick, action: TraceAction) -> TraceIndex {
        let index = TraceIndex::new(u32::try_from(self.records.len()).unwrap_or(u32::MAX));
        self.records.push(TraceRecord {
            index,
            tick,
            action,
        });
        index
    }

    /// Records whose action names one event.
    pub fn for_event(&self, event: EventId) -> impl Iterator<Item = &TraceRecord> {
        self.records
            .iter()
            .filter(move |record| record.action.event() == Some(event))
    }

    /// True when every record carries its own position in order.
    #[must_use]
    pub fn indices_are_contiguous(&self) -> bool {
        self.records
            .iter()
            .enumerate()
            .all(|(position, record)| u32::try_from(position) == Ok(record.index.get()))
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};

    use super::*;

    fn sample_actions() -> Vec<TraceAction> {
        let event = EventId::new(1);
        let endpoint = EndpointId::new(2);
        let fault = FaultId::new(3);
        alloc::vec![
            TraceAction::EventScheduled {
                event,
                lane: Lane::Control,
                source: EndpointId::new(1),
                destination: endpoint,
                subject: Subject::Message(MessageId::new([1u8; 32])),
                deliver_at: Tick::new(4),
            },
            TraceAction::FaultApplied {
                fault,
                event,
                effect: FaultEffect::Dropped,
            },
            TraceAction::FaultRejected {
                fault,
                event,
                reason: RejectReason::UnknownEndpoint,
            },
            TraceAction::TickAdvanced {
                from: Tick::ZERO,
                to: Tick::new(1),
            },
            TraceAction::EndpointHalted { endpoint },
            TraceAction::EndpointResumed {
                endpoint,
                released: 2,
            },
            TraceAction::DeliveryAttempted {
                event,
                destination: endpoint,
                attempt: 1,
            },
            TraceAction::DeliveryBlocked {
                event,
                destination: endpoint,
                reason: BlockReason::EndpointHalted,
            },
            TraceAction::EventDelivered {
                event,
                destination: endpoint,
                subject: Subject::Transfer(TransferId::new([2u8; 32])),
                after_deadline: true,
            },
            TraceAction::EventDropped {
                event,
                fault: Some(fault),
            },
            TraceAction::EventDropped { event, fault: None },
            TraceAction::EventExpired {
                event,
                deadline: Tick::new(9),
            },
            TraceAction::EventRejected {
                event,
                reason: RejectReason::SplitExceedsRequest,
            },
            TraceAction::DuplicateCreated {
                original: event,
                duplicate: EventId::new(5),
                deliver_at: Tick::new(6),
            },
            TraceAction::PartialDeliveryCreated {
                original: event,
                piece_event: EventId::new(6),
                transfer: TransferId::new([3u8; 32]),
                amount: AssetAmount::new(7),
                piece: 1,
            },
            TraceAction::EventReordered {
                event,
                from: Tick::new(1),
                to: Tick::new(2),
            },
            TraceAction::LanePaused { lane: Lane::Asset },
            TraceAction::LaneResumed {
                lane: Lane::Asset,
                released: 3,
            },
        ]
    }

    #[test]
    fn every_action_prints_a_line_of_its_own() {
        let mut seen: Vec<String> = Vec::new();
        for action in sample_actions() {
            let record = TraceRecord {
                index: TraceIndex::new(0),
                tick: Tick::ZERO,
                action,
            };
            let text = record.to_string();
            assert!(!text.is_empty());
            assert!(!seen.contains(&text), "repeated line {text}");
            seen.push(text);
        }
    }

    #[test]
    fn action_codes_stay_distinct_per_kind() {
        let mut seen: Vec<u8> = Vec::new();
        for action in sample_actions() {
            let code = action.code();
            if !seen.contains(&code) {
                seen.push(code);
            }
        }
        assert_eq!(seen.len(), 17);
    }

    #[test]
    fn records_about_one_event_can_be_read_back() {
        let mut trace = Trace::new();
        assert!(trace.is_empty());
        for action in sample_actions() {
            trace.push(Tick::new(1), action);
        }
        assert_eq!(trace.len(), 18);
        assert!(trace.indices_are_contiguous());
        assert_eq!(trace.for_event(EventId::new(1)).count(), 13);
        assert_eq!(trace.records().len(), 18);
        assert_eq!(trace.since(16).len(), 2);
        assert!(trace.since(99).is_empty());
    }

    #[test]
    fn only_scheduling_and_delivery_records_name_a_subject() {
        let with_subject = sample_actions()
            .iter()
            .filter(|action| action.subject().is_some())
            .count();
        assert_eq!(with_subject, 3);
    }

    #[test]
    fn a_trace_index_prints_its_position() {
        assert_eq!(TraceIndex::new(4).to_string(), "[4]");
        assert_eq!(TraceIndex::new(4).get(), 4);
    }
}
