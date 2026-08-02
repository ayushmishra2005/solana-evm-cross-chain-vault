//! Scheduled control messages and asset movements.
//!
//! The two kinds are separate on purpose. A control event carries bytes the
//! simulator never interprets, and an asset event carries value with no link
//! back to any message.

extern crate alloc;

use alloc::vec::Vec;

use protocol_types::{AssetAmount, MessageId, TransferId};

use crate::endpoint::EndpointId;
use crate::lane::Lane;
use crate::time::Tick;

/// Names one scheduled delivery attempt stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(u64);

impl EventId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for EventId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl core::fmt::Display for EventId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "#{}", self.0)
    }
}

/// Where an event sits in its life.
///
/// `Blocked` is not final. A blocked event waits for a resume and then goes
/// back into the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventStatus {
    Scheduled,
    Ready,
    Delivered,
    Dropped,
    Blocked,
    Expired,
    RejectedBySimulator,
}

impl EventStatus {
    /// True when no later attempt can change this event.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Delivered | Self::Dropped | Self::Expired | Self::RejectedBySimulator
        )
    }

    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Scheduled => 0,
            Self::Ready => 1,
            Self::Delivered => 2,
            Self::Dropped => 3,
            Self::Blocked => 4,
            Self::Expired => 5,
            Self::RejectedBySimulator => 6,
        }
    }
}

/// The single byte edit a corruption fault made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteMutation {
    pub offset: usize,
    pub from: u8,
    pub to: u8,
    /// Identity of the message before the edit.
    pub original_message_id: MessageId,
}

/// A control message waiting to be delivered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlEvent {
    pub id: EventId,
    pub source: EndpointId,
    pub destination: EndpointId,
    /// Where the sender aimed before any reroute fault.
    pub intended_destination: EndpointId,
    pub bytes: Vec<u8>,
    pub message_id: MessageId,
    pub deliver_at: Tick,
    pub attempts: u32,
    pub duplicate_of: Option<EventId>,
    pub mutation: Option<ByteMutation>,
    /// Transport deadline the caller supplied, never inferred from the bytes.
    pub expires_at: Option<Tick>,
    /// True when a fault made this event instead of the caller.
    pub from_fault: bool,
    pub status: EventStatus,
}

/// An asset movement waiting to be delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssetEvent {
    pub id: EventId,
    pub transfer: TransferId,
    pub source: EndpointId,
    pub destination: EndpointId,
    pub intended_destination: EndpointId,
    pub requested: AssetAmount,
    pub delivered: AssetAmount,
    pub deliver_at: Tick,
    pub attempts: u32,
    pub duplicate_of: Option<EventId>,
    pub piece: Option<u16>,
    pub over_delivered: bool,
    pub timeout_at: Option<Tick>,
    pub from_fault: bool,
    pub status: EventStatus,
}

/// One entry of the queue, on either lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Control(ControlEvent),
    Asset(AssetEvent),
}

impl Event {
    #[must_use]
    pub const fn id(&self) -> EventId {
        match self {
            Self::Control(event) => event.id,
            Self::Asset(event) => event.id,
        }
    }

    #[must_use]
    pub const fn lane(&self) -> Lane {
        match self {
            Self::Control(_) => Lane::Control,
            Self::Asset(_) => Lane::Asset,
        }
    }

    #[must_use]
    pub const fn source(&self) -> EndpointId {
        match self {
            Self::Control(event) => event.source,
            Self::Asset(event) => event.source,
        }
    }

    #[must_use]
    pub const fn destination(&self) -> EndpointId {
        match self {
            Self::Control(event) => event.destination,
            Self::Asset(event) => event.destination,
        }
    }

    #[must_use]
    pub const fn intended_destination(&self) -> EndpointId {
        match self {
            Self::Control(event) => event.intended_destination,
            Self::Asset(event) => event.intended_destination,
        }
    }

    #[must_use]
    pub const fn deliver_at(&self) -> Tick {
        match self {
            Self::Control(event) => event.deliver_at,
            Self::Asset(event) => event.deliver_at,
        }
    }

    #[must_use]
    pub const fn status(&self) -> EventStatus {
        match self {
            Self::Control(event) => event.status,
            Self::Asset(event) => event.status,
        }
    }

    #[must_use]
    pub const fn attempts(&self) -> u32 {
        match self {
            Self::Control(event) => event.attempts,
            Self::Asset(event) => event.attempts,
        }
    }

    #[must_use]
    pub const fn duplicate_of(&self) -> Option<EventId> {
        match self {
            Self::Control(event) => event.duplicate_of,
            Self::Asset(event) => event.duplicate_of,
        }
    }

    /// True when the simulator made this event from another one.
    ///
    /// A made event never runs a fault that creates more events, so copies and
    /// pieces cannot grow without end.
    #[must_use]
    pub const fn made_by_fault(&self) -> bool {
        match self {
            Self::Control(event) => event.from_fault,
            Self::Asset(event) => event.from_fault,
        }
    }

    /// The deadline the caller attached, if any.
    #[must_use]
    pub const fn deadline(&self) -> Option<Tick> {
        match self {
            Self::Control(event) => event.expires_at,
            Self::Asset(event) => event.timeout_at,
        }
    }

    #[must_use]
    pub const fn message_id(&self) -> Option<MessageId> {
        match self {
            Self::Control(event) => Some(event.message_id),
            Self::Asset(_) => None,
        }
    }

    #[must_use]
    pub const fn transfer_id(&self) -> Option<TransferId> {
        match self {
            Self::Control(_) => None,
            Self::Asset(event) => Some(event.transfer),
        }
    }

    #[must_use]
    pub const fn control(&self) -> Option<&ControlEvent> {
        match self {
            Self::Control(event) => Some(event),
            Self::Asset(_) => None,
        }
    }

    #[must_use]
    pub const fn asset(&self) -> Option<&AssetEvent> {
        match self {
            Self::Control(_) => None,
            Self::Asset(event) => Some(event),
        }
    }

    pub(crate) fn set_status(&mut self, status: EventStatus) {
        match self {
            Self::Control(event) => event.status = status,
            Self::Asset(event) => event.status = status,
        }
    }

    pub(crate) fn set_deliver_at(&mut self, tick: Tick) {
        match self {
            Self::Control(event) => event.deliver_at = tick,
            Self::Asset(event) => event.deliver_at = tick,
        }
    }

    pub(crate) fn set_destination(&mut self, endpoint: EndpointId) {
        match self {
            Self::Control(event) => event.destination = endpoint,
            Self::Asset(event) => event.destination = endpoint,
        }
    }

    pub(crate) fn bump_attempts(&mut self) {
        match self {
            Self::Control(event) => event.attempts = event.attempts.saturating_add(1),
            Self::Asset(event) => event.attempts = event.attempts.saturating_add(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control() -> Event {
        Event::Control(ControlEvent {
            id: EventId::new(1),
            source: EndpointId::new(1),
            destination: EndpointId::new(2),
            intended_destination: EndpointId::new(2),
            bytes: alloc::vec![1, 2, 3],
            message_id: MessageId::new([9u8; 32]),
            deliver_at: Tick::new(5),
            attempts: 0,
            duplicate_of: None,
            mutation: None,
            expires_at: Some(Tick::new(20)),
            from_fault: false,
            status: EventStatus::Scheduled,
        })
    }

    fn asset() -> Event {
        Event::Asset(AssetEvent {
            id: EventId::new(2),
            transfer: TransferId::new([7u8; 32]),
            source: EndpointId::new(1),
            destination: EndpointId::new(2),
            intended_destination: EndpointId::new(2),
            requested: AssetAmount::new(100),
            delivered: AssetAmount::new(100),
            deliver_at: Tick::new(5),
            attempts: 0,
            duplicate_of: None,
            piece: None,
            over_delivered: false,
            timeout_at: None,
            from_fault: false,
            status: EventStatus::Scheduled,
        })
    }

    #[test]
    fn only_delivered_dropped_expired_and_rejected_are_final() {
        for status in [
            EventStatus::Delivered,
            EventStatus::Dropped,
            EventStatus::Expired,
            EventStatus::RejectedBySimulator,
        ] {
            assert!(status.is_terminal(), "{status:?}");
        }
        for status in [
            EventStatus::Scheduled,
            EventStatus::Ready,
            EventStatus::Blocked,
        ] {
            assert!(!status.is_terminal(), "{status:?}");
        }
    }

    #[test]
    fn every_status_has_a_distinct_code() {
        let all = [
            EventStatus::Scheduled,
            EventStatus::Ready,
            EventStatus::Delivered,
            EventStatus::Dropped,
            EventStatus::Blocked,
            EventStatus::Expired,
            EventStatus::RejectedBySimulator,
        ];
        let mut seen = alloc::vec::Vec::new();
        for status in all {
            assert!(!seen.contains(&status.code()));
            seen.push(status.code());
        }
    }

    #[test]
    fn a_control_event_reports_a_message_id_and_no_transfer_id() {
        let event = control();
        assert_eq!(event.lane(), Lane::Control);
        assert_eq!(event.message_id(), Some(MessageId::new([9u8; 32])));
        assert_eq!(event.transfer_id(), None);
        assert!(event.control().is_some());
        assert!(event.asset().is_none());
        assert_eq!(event.deadline(), Some(Tick::new(20)));
    }

    #[test]
    fn an_asset_event_reports_a_transfer_id_and_no_message_id() {
        let event = asset();
        assert_eq!(event.lane(), Lane::Asset);
        assert_eq!(event.transfer_id(), Some(TransferId::new([7u8; 32])));
        assert_eq!(event.message_id(), None);
        assert!(event.asset().is_some());
        assert!(event.control().is_none());
        assert_eq!(event.deadline(), None);
    }

    #[test]
    fn setters_reach_both_event_kinds() {
        for mut event in [control(), asset()] {
            event.set_status(EventStatus::Delivered);
            event.set_deliver_at(Tick::new(9));
            event.set_destination(EndpointId::new(8));
            event.bump_attempts();
            assert_eq!(event.status(), EventStatus::Delivered);
            assert_eq!(event.deliver_at(), Tick::new(9));
            assert_eq!(event.destination(), EndpointId::new(8));
            assert_eq!(event.intended_destination(), EndpointId::new(2));
            assert_eq!(event.attempts(), 1);
            assert_eq!(event.source(), EndpointId::new(1));
            assert_eq!(event.duplicate_of(), None);
        }
    }

    #[test]
    fn an_event_id_prints_and_converts() {
        extern crate alloc;
        use alloc::string::ToString;
        assert_eq!(EventId::from(3u64).get(), 3);
        assert_eq!(EventId::new(3).to_string(), "#3");
    }
}
