//! Generic delivery endpoints.
//!
//! An endpoint is only a mailbox with a running state. The simulator never
//! reads what an inbox means, so no endpoint is tied to a chain.

extern crate alloc;

use alloc::vec::Vec;

use protocol_types::{AssetAmount, MessageId, TransferId};

use crate::event::{ByteMutation, EventId};
use crate::time::Tick;

/// Names one endpoint of the simulated network.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EndpointId(u32);

impl EndpointId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for EndpointId {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl core::fmt::Display for EndpointId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "e{}", self.0)
    }
}

/// Whether an endpoint currently completes deliveries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EndpointState {
    #[default]
    Active,
    Halted,
}

impl EndpointState {
    #[must_use]
    pub const fn is_halted(self) -> bool {
        matches!(self, Self::Halted)
    }
}

/// One control message that reached an endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveredControl {
    pub event: EventId,
    pub source: EndpointId,
    /// Where the sender aimed, which differs from the inbox after a reroute.
    pub intended_destination: EndpointId,
    pub bytes: Vec<u8>,
    pub message_id: MessageId,
    pub delivered_at: Tick,
    pub duplicate_of: Option<EventId>,
    pub mutation: Option<ByteMutation>,
    pub after_deadline: bool,
}

impl DeliveredControl {
    /// True when the message landed somewhere the sender did not choose.
    #[must_use]
    pub fn is_misrouted(&self, inbox_owner: EndpointId) -> bool {
        self.intended_destination != inbox_owner
    }
}

/// One asset movement that reached an endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeliveredAsset {
    pub event: EventId,
    pub transfer: TransferId,
    pub source: EndpointId,
    pub intended_destination: EndpointId,
    pub requested: AssetAmount,
    pub delivered: AssetAmount,
    pub delivered_at: Tick,
    pub duplicate_of: Option<EventId>,
    /// Index of this piece when a transfer was split.
    pub piece: Option<u16>,
    pub over_delivered: bool,
    pub after_timeout: bool,
}

impl DeliveredAsset {
    #[must_use]
    pub fn is_misrouted(&self, inbox_owner: EndpointId) -> bool {
        self.intended_destination != inbox_owner
    }
}

/// A mailbox pair plus a running state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    id: EndpointId,
    state: EndpointState,
    control_inbox: Vec<DeliveredControl>,
    asset_inbox: Vec<DeliveredAsset>,
}

impl Endpoint {
    #[must_use]
    pub fn new(id: EndpointId) -> Self {
        Self {
            id,
            state: EndpointState::Active,
            control_inbox: Vec::new(),
            asset_inbox: Vec::new(),
        }
    }

    #[must_use]
    pub const fn id(&self) -> EndpointId {
        self.id
    }

    #[must_use]
    pub const fn state(&self) -> EndpointState {
        self.state
    }

    #[must_use]
    pub const fn is_halted(&self) -> bool {
        self.state.is_halted()
    }

    #[must_use]
    pub fn control_inbox(&self) -> &[DeliveredControl] {
        &self.control_inbox
    }

    #[must_use]
    pub fn asset_inbox(&self) -> &[DeliveredAsset] {
        &self.asset_inbox
    }

    pub(crate) fn set_state(&mut self, state: EndpointState) {
        self.state = state;
    }

    pub(crate) fn push_control(&mut self, delivered: DeliveredControl) {
        self.control_inbox.push(delivered);
    }

    pub(crate) fn push_asset(&mut self, delivered: DeliveredAsset) {
        self.asset_inbox.push(delivered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_endpoint_is_active_and_empty() {
        let endpoint = Endpoint::new(EndpointId::new(4));
        assert_eq!(endpoint.id(), EndpointId::new(4));
        assert!(!endpoint.is_halted());
        assert!(endpoint.control_inbox().is_empty());
        assert!(endpoint.asset_inbox().is_empty());
    }

    #[test]
    fn halting_changes_the_reported_state() {
        let mut endpoint = Endpoint::new(EndpointId::new(1));
        endpoint.set_state(EndpointState::Halted);
        assert!(endpoint.is_halted());
        assert_eq!(endpoint.state(), EndpointState::Halted);
    }

    #[test]
    fn an_endpoint_id_prints_and_converts() {
        extern crate alloc;
        use alloc::string::ToString;
        assert_eq!(EndpointId::from(7u32).get(), 7);
        assert_eq!(EndpointId::new(7).to_string(), "e7");
    }

    #[test]
    fn a_delivery_aimed_elsewhere_reads_as_misrouted() {
        let asset = DeliveredAsset {
            event: EventId::new(1),
            transfer: TransferId::new([1u8; 32]),
            source: EndpointId::new(1),
            intended_destination: EndpointId::new(2),
            requested: AssetAmount::new(10),
            delivered: AssetAmount::new(10),
            delivered_at: Tick::ZERO,
            duplicate_of: None,
            piece: None,
            over_delivered: false,
            after_timeout: false,
        };
        assert!(asset.is_misrouted(EndpointId::new(3)));
        assert!(!asset.is_misrouted(EndpointId::new(2)));
    }
}
