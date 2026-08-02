//! Canonical digest of the whole simulator.
//!
//! The bytes below are written by hand. Nothing here reads `Debug` output or
//! Rust memory layout, so the digest depends only on declared state.

extern crate alloc;

use alloc::vec::Vec;

use protocol_types::keccak256;

use crate::endpoint::{DeliveredAsset, DeliveredControl, Endpoint};
use crate::event::{Event, EventStatus};
use crate::fault::{Fault, FaultAction, FaultTarget};
use crate::lane::{Lane, LaneState};
use crate::simulator::{LatePolicy, Simulator};
use crate::time::Tick;

/// Keccak-256 over the canonical state bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateHash([u8; 32]);

impl StateHash {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl core::fmt::Display for StateHash {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Appends fixed width fields in the order they are written.
#[derive(Debug, Default)]
pub(crate) struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    fn new() -> Self {
        Self::default()
    }

    fn tag(&mut self, tag: &[u8; 4]) {
        self.bytes.extend_from_slice(tag);
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn bool(&mut self, value: bool) {
        self.bytes.push(u8::from(value));
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn u128(&mut self, value: u128) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn count(&mut self, value: usize) {
        self.u32(u32::try_from(value).unwrap_or(u32::MAX));
    }

    fn wide(&mut self, value: &[u8; 32]) {
        self.bytes.extend_from_slice(value);
    }

    /// Length first, so no two payloads can share one byte string.
    fn blob(&mut self, value: &[u8]) {
        self.count(value.len());
        self.bytes.extend_from_slice(value);
    }

    fn tick(&mut self, value: Tick) {
        self.u64(value.get());
    }

    /// A presence byte then a fixed width value, so absence has its own shape.
    fn maybe_tick(&mut self, value: Option<Tick>) {
        self.bool(value.is_some());
        self.u64(value.unwrap_or(Tick::ZERO).get());
    }

    fn maybe_u64(&mut self, value: Option<u64>) {
        self.bool(value.is_some());
        self.u64(value.unwrap_or(0));
    }

    fn maybe_u16(&mut self, value: Option<u16>) {
        self.bool(value.is_some());
        self.u16(value.unwrap_or(0));
    }

    fn finish(self) -> StateHash {
        StateHash::new(keccak256(&self.bytes))
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

const fn status_code(status: EventStatus) -> u8 {
    status.code()
}

const fn lane_code(lane: Lane) -> u8 {
    lane.priority()
}

const fn lane_state_code(state: LaneState) -> u8 {
    match state {
        LaneState::Running => 0,
        LaneState::Paused => 1,
    }
}

const fn policy_code(policy: LatePolicy) -> u8 {
    match policy {
        LatePolicy::DeliverWithMarker => 0,
        LatePolicy::Expire => 1,
    }
}

fn write_control_inbox(out: &mut Encoder, entries: &[DeliveredControl]) {
    out.count(entries.len());
    for entry in entries {
        out.u64(entry.event.get());
        out.u32(entry.source.get());
        out.u32(entry.intended_destination.get());
        out.blob(&entry.bytes);
        out.wide(entry.message_id.as_bytes());
        out.tick(entry.delivered_at);
        out.maybe_u64(entry.duplicate_of.map(crate::event::EventId::get));
        match entry.mutation {
            Some(mutation) => {
                out.bool(true);
                out.u64(u64::try_from(mutation.offset).unwrap_or(u64::MAX));
                out.u8(mutation.from);
                out.u8(mutation.to);
                out.wide(mutation.original_message_id.as_bytes());
            }
            None => {
                out.bool(false);
                out.u64(0);
                out.u8(0);
                out.u8(0);
                out.wide(&[0u8; 32]);
            }
        }
        out.bool(entry.after_deadline);
    }
}

fn write_asset_inbox(out: &mut Encoder, entries: &[DeliveredAsset]) {
    out.count(entries.len());
    for entry in entries {
        out.u64(entry.event.get());
        out.wide(entry.transfer.as_bytes());
        out.u32(entry.source.get());
        out.u32(entry.intended_destination.get());
        out.u128(entry.requested.get());
        out.u128(entry.delivered.get());
        out.tick(entry.delivered_at);
        out.maybe_u64(entry.duplicate_of.map(crate::event::EventId::get));
        out.maybe_u16(entry.piece);
        out.bool(entry.over_delivered);
        out.bool(entry.after_timeout);
    }
}

fn write_endpoint(out: &mut Encoder, endpoint: &Endpoint) {
    out.u32(endpoint.id().get());
    out.bool(endpoint.is_halted());
    write_control_inbox(out, endpoint.control_inbox());
    write_asset_inbox(out, endpoint.asset_inbox());
}

fn write_event(out: &mut Encoder, event: &Event) {
    out.u8(lane_code(event.lane()));
    match event {
        Event::Control(control) => {
            out.u64(control.id.get());
            out.u32(control.source.get());
            out.u32(control.destination.get());
            out.u32(control.intended_destination.get());
            out.blob(&control.bytes);
            out.wide(control.message_id.as_bytes());
            out.tick(control.deliver_at);
            out.u32(control.attempts);
            out.maybe_u64(control.duplicate_of.map(crate::event::EventId::get));
            match control.mutation {
                Some(mutation) => {
                    out.bool(true);
                    out.u64(u64::try_from(mutation.offset).unwrap_or(u64::MAX));
                    out.u8(mutation.from);
                    out.u8(mutation.to);
                }
                None => {
                    out.bool(false);
                    out.u64(0);
                    out.u8(0);
                    out.u8(0);
                }
            }
            out.maybe_tick(control.expires_at);
            out.bool(control.from_fault);
            out.u8(status_code(control.status));
        }
        Event::Asset(asset) => {
            out.u64(asset.id.get());
            out.wide(asset.transfer.as_bytes());
            out.u32(asset.source.get());
            out.u32(asset.destination.get());
            out.u32(asset.intended_destination.get());
            out.u128(asset.requested.get());
            out.u128(asset.delivered.get());
            out.tick(asset.deliver_at);
            out.u32(asset.attempts);
            out.maybe_u64(asset.duplicate_of.map(crate::event::EventId::get));
            out.maybe_u16(asset.piece);
            out.bool(asset.over_delivered);
            out.maybe_tick(asset.timeout_at);
            out.bool(asset.from_fault);
            out.u8(status_code(asset.status));
        }
    }
}

fn write_target(out: &mut Encoder, target: FaultTarget) {
    match target {
        FaultTarget::Event(id) => {
            out.u8(1);
            out.u64(id.get());
        }
        FaultTarget::Message(id) => {
            out.u8(2);
            out.wide(id.as_bytes());
        }
        FaultTarget::Transfer(id) => {
            out.u8(3);
            out.wide(id.as_bytes());
        }
        FaultTarget::Route {
            source,
            destination,
            lane,
        } => {
            out.u8(4);
            out.u32(source.get());
            out.u32(destination.get());
            out.u8(lane_code(lane));
        }
        FaultTarget::Lane(lane) => {
            out.u8(5);
            out.u8(lane_code(lane));
        }
    }
}

fn write_action(out: &mut Encoder, action: &FaultAction) {
    match action {
        FaultAction::Delay { ticks } => {
            out.u8(1);
            out.u64(*ticks);
        }
        FaultAction::Drop => out.u8(2),
        FaultAction::Duplicate { copies, spacing } => {
            out.u8(3);
            out.u16(*copies);
            out.u64(*spacing);
        }
        FaultAction::ReplaceDestination(endpoint) => {
            out.u8(4);
            out.u32(endpoint.get());
        }
        FaultAction::CorruptByte { offset, mask } => {
            out.u8(5);
            out.u64(u64::try_from(*offset).unwrap_or(u64::MAX));
            out.u8(*mask);
        }
        FaultAction::Partial { amount } => {
            out.u8(6);
            out.u128(amount.get());
        }
        FaultAction::Split { pieces, spacing } => {
            out.u8(7);
            out.count(pieces.len());
            for piece in pieces {
                out.u128(piece.get());
            }
            out.u64(*spacing);
        }
        FaultAction::OverDeliver { amount } => {
            out.u8(8);
            out.u128(amount.get());
        }
    }
}

fn write_fault(out: &mut Encoder, fault: &Fault) {
    out.u32(fault.id.get());
    write_target(out, fault.target);
    write_action(out, &fault.action);
}

/// Writes every part of the state the digest commits to.
pub(crate) fn encode_state(simulator: &Simulator, out: &mut Encoder) {
    out.tag(b"XSM1");

    out.tag(b"TIME");
    out.tick(simulator.now());

    out.tag(b"CONF");
    out.u8(policy_code(simulator.config().control_late_policy));
    out.u8(policy_code(simulator.config().asset_late_policy));

    out.tag(b"ENDP");
    out.count(simulator.endpoint_count());
    for endpoint in simulator.endpoints() {
        write_endpoint(out, endpoint);
    }

    out.tag(b"EVNT");
    out.count(simulator.event_count());
    for event in simulator.events() {
        write_event(out, event);
    }

    out.tag(b"QUEU");
    out.count(simulator.queue().len());
    for key in simulator.queue().iter() {
        out.tick(key.deliver_at);
        out.u8(key.lane_priority);
        out.u64(key.event.get());
    }

    out.tag(b"HELD");
    out.count(simulator.held().len());
    for event in simulator.held() {
        out.u64(event.get());
    }

    out.tag(b"LANE");
    for lane in Lane::ALL {
        out.u8(lane_code(lane));
        out.u8(lane_state_code(simulator.lane_state(lane)));
    }

    out.tag(b"PLAN");
    out.count(simulator.plan().len());
    for fault in simulator.plan().iter() {
        write_fault(out, fault);
    }

    out.tag(b"BOUN");
    out.count(simulator.resolved_events().len());
    for event in simulator.resolved_events() {
        out.u64(event.get());
    }

    out.tag(b"TRCE");
    out.count(simulator.trace().len());

    out.tag(b"NEXT");
    out.u64(simulator.next_event_number());
}

/// Digest of the whole simulator state.
#[must_use]
pub fn state_hash(simulator: &Simulator) -> StateHash {
    let mut encoder = Encoder::new();
    encode_state(simulator, &mut encoder);
    encoder.finish()
}

#[cfg(test)]
pub(crate) fn state_bytes(simulator: &Simulator) -> Vec<u8> {
    let mut encoder = Encoder::new();
    encode_state(simulator, &mut encoder);
    encoder.bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn the_digest_prints_as_lower_case_hex() {
        let hash = StateHash::new([0xABu8; 32]);
        let text = hash.to_string();
        assert_eq!(text.len(), 64);
        assert!(text.starts_with("abab"));
        assert_eq!(hash.as_bytes(), &[0xABu8; 32]);
        assert_eq!(hash.to_bytes(), [0xABu8; 32]);
    }

    #[test]
    fn a_length_prefix_keeps_neighbouring_blobs_apart() {
        let mut left = Encoder::new();
        left.blob(b"ab");
        left.blob(b"c");
        let mut right = Encoder::new();
        right.blob(b"a");
        right.blob(b"bc");
        assert_ne!(left.bytes(), right.bytes());
    }

    #[test]
    fn a_missing_value_is_not_the_same_as_a_zero_value() {
        let mut absent = Encoder::new();
        absent.maybe_tick(None);
        let mut present = Encoder::new();
        present.maybe_tick(Some(Tick::ZERO));
        assert_ne!(absent.bytes(), present.bytes());

        let mut no_piece = Encoder::new();
        no_piece.maybe_u16(None);
        let mut zero_piece = Encoder::new();
        zero_piece.maybe_u16(Some(0));
        assert_ne!(no_piece.bytes(), zero_piece.bytes());

        let mut no_id = Encoder::new();
        no_id.maybe_u64(None);
        let mut zero_id = Encoder::new();
        zero_id.maybe_u64(Some(0));
        assert_ne!(no_id.bytes(), zero_id.bytes());
    }

    #[test]
    fn integers_are_written_big_endian() {
        let mut encoder = Encoder::new();
        encoder.u16(1);
        encoder.u32(1);
        encoder.u64(1);
        encoder.u128(1);
        let bytes = encoder.bytes();
        assert_eq!(bytes.first(), Some(&0));
        assert_eq!(bytes.last(), Some(&1));
        assert_eq!(bytes.len(), 2 + 4 + 8 + 16);
    }

    #[test]
    fn every_fault_action_writes_its_own_discriminant() {
        use crate::endpoint::EndpointId;
        use protocol_types::AssetAmount;

        let actions = [
            FaultAction::Delay { ticks: 1 },
            FaultAction::Drop,
            FaultAction::Duplicate {
                copies: 1,
                spacing: 1,
            },
            FaultAction::ReplaceDestination(EndpointId::new(1)),
            FaultAction::CorruptByte { offset: 1, mask: 1 },
            FaultAction::Partial {
                amount: AssetAmount::new(1),
            },
            FaultAction::Split {
                pieces: alloc::vec![AssetAmount::new(1)],
                spacing: 1,
            },
            FaultAction::OverDeliver {
                amount: AssetAmount::new(1),
            },
        ];
        let mut seen: Vec<u8> = Vec::new();
        for action in &actions {
            let mut encoder = Encoder::new();
            write_action(&mut encoder, action);
            let code = encoder.bytes().first().copied().unwrap_or(0);
            assert!(!seen.contains(&code));
            seen.push(code);
        }
    }

    #[test]
    fn the_canonical_bytes_start_with_the_crate_tag_and_track_the_clock() {
        use crate::endpoint::EndpointId;

        let Ok(mut simulator) = Simulator::new(&[EndpointId::new(1), EndpointId::new(2)]) else {
            unreachable!("two distinct endpoints always build");
        };
        let before = state_bytes(&simulator);
        assert_eq!(before.get(..4), Some(b"XSM1".as_slice()));
        assert_eq!(before.get(4..8), Some(b"TIME".as_slice()));
        assert_eq!(before.get(8..16), Some([0u8; 8].as_slice()));

        let _ = simulator.advance_by(3);
        let after = state_bytes(&simulator);
        assert_eq!(after.get(8..16), Some([0, 0, 0, 0, 0, 0, 0, 3].as_slice()));
        assert_ne!(after, before);
    }

    #[test]
    fn every_fault_target_writes_its_own_discriminant() {
        use crate::endpoint::EndpointId;
        use crate::event::EventId;
        use protocol_types::{MessageId, TransferId};

        let targets = [
            FaultTarget::Event(EventId::new(1)),
            FaultTarget::Message(MessageId::ZERO),
            FaultTarget::Transfer(TransferId::ZERO),
            FaultTarget::Route {
                source: EndpointId::new(1),
                destination: EndpointId::new(2),
                lane: Lane::Control,
            },
            FaultTarget::Lane(Lane::Asset),
        ];
        let mut seen: Vec<u8> = Vec::new();
        for target in targets {
            let mut encoder = Encoder::new();
            write_target(&mut encoder, target);
            let code = encoder.bytes().first().copied().unwrap_or(0);
            assert!(!seen.contains(&code));
            seen.push(code);
        }
    }
}
