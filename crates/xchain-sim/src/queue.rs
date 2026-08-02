//! The ordered set of pending deliveries.
//!
//! Order is fixed by the key below and never by map iteration order, so the
//! same schedule always drains in the same sequence.

extern crate alloc;

use alloc::collections::BTreeSet;

use crate::event::EventId;
use crate::lane::Lane;
use crate::time::Tick;

/// Sort key of one pending delivery.
///
/// The field order is the tie-breaker: tick first, then lane, then event id.
/// Event ids never repeat, so two entries can never compare equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueueKey {
    pub deliver_at: Tick,
    pub lane_priority: u8,
    pub event: EventId,
}

impl QueueKey {
    #[must_use]
    pub const fn new(deliver_at: Tick, lane: Lane, event: EventId) -> Self {
        Self {
            deliver_at,
            lane_priority: lane.priority(),
            event,
        }
    }
}

/// A deterministic set of pending deliveries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventQueue {
    entries: BTreeSet<QueueKey>,
}

impl EventQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn insert(&mut self, key: QueueKey) -> bool {
        self.entries.insert(key)
    }

    pub(crate) fn remove(&mut self, key: &QueueKey) -> bool {
        self.entries.remove(key)
    }

    /// The entry that should be attempted next.
    #[must_use]
    pub fn peek(&self) -> Option<QueueKey> {
        self.entries.first().copied()
    }

    pub(crate) fn pop(&mut self) -> Option<QueueKey> {
        self.entries.pop_first()
    }

    /// Pending entries in delivery order.
    pub fn iter(&self) -> impl Iterator<Item = &QueueKey> {
        self.entries.iter()
    }

    /// The key of one event, when it is still pending.
    #[must_use]
    pub fn find(&self, event: EventId) -> Option<QueueKey> {
        self.entries.iter().find(|key| key.event == event).copied()
    }

    /// The earliest tick that still holds work.
    #[must_use]
    pub fn next_tick(&self) -> Option<Tick> {
        self.peek().map(|key| key.deliver_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(tick: u64, lane: Lane, event: u64) -> QueueKey {
        QueueKey::new(Tick::new(tick), lane, EventId::new(event))
    }

    #[test]
    fn the_earliest_tick_comes_out_first() {
        let mut queue = EventQueue::new();
        assert!(queue.insert(key(9, Lane::Control, 1)));
        assert!(queue.insert(key(2, Lane::Control, 2)));
        assert!(queue.insert(key(5, Lane::Control, 3)));
        assert_eq!(queue.pop().map(|k| k.event), Some(EventId::new(2)));
        assert_eq!(queue.pop().map(|k| k.event), Some(EventId::new(3)));
        assert_eq!(queue.pop().map(|k| k.event), Some(EventId::new(1)));
        assert!(queue.is_empty());
    }

    #[test]
    fn control_comes_before_asset_at_the_same_tick() {
        let mut queue = EventQueue::new();
        queue.insert(key(4, Lane::Asset, 1));
        queue.insert(key(4, Lane::Control, 2));
        assert_eq!(queue.pop().map(|k| k.event), Some(EventId::new(2)));
        assert_eq!(queue.pop().map(|k| k.event), Some(EventId::new(1)));
    }

    #[test]
    fn the_lower_event_id_wins_a_full_tie() {
        let mut queue = EventQueue::new();
        queue.insert(key(4, Lane::Control, 30));
        queue.insert(key(4, Lane::Control, 7));
        queue.insert(key(4, Lane::Control, 19));
        let order: alloc::vec::Vec<u64> = core::iter::from_fn(|| queue.pop())
            .map(|k| k.event.get())
            .collect();
        assert_eq!(order, alloc::vec![7, 19, 30]);
    }

    #[test]
    fn insertion_order_does_not_change_drain_order() {
        let forward = [
            key(3, Lane::Control, 1),
            key(3, Lane::Asset, 2),
            key(1, Lane::Asset, 3),
        ];
        let mut backward = forward;
        backward.reverse();

        let drain = |source: &[QueueKey]| {
            let mut queue = EventQueue::new();
            for entry in source {
                queue.insert(*entry);
            }
            core::iter::from_fn(|| queue.pop()).collect::<alloc::vec::Vec<_>>()
        };
        assert_eq!(drain(&forward), drain(&backward));
    }

    #[test]
    fn a_pending_entry_can_be_found_and_removed() {
        let mut queue = EventQueue::new();
        let entry = key(6, Lane::Asset, 11);
        queue.insert(entry);
        assert_eq!(queue.find(EventId::new(11)), Some(entry));
        assert_eq!(queue.find(EventId::new(12)), None);
        assert_eq!(queue.next_tick(), Some(Tick::new(6)));
        assert_eq!(queue.len(), 1);
        assert!(queue.remove(&entry));
        assert!(!queue.remove(&entry));
        assert_eq!(queue.next_tick(), None);
    }

    #[test]
    fn iteration_follows_delivery_order() {
        let mut queue = EventQueue::new();
        queue.insert(key(8, Lane::Control, 1));
        queue.insert(key(2, Lane::Control, 2));
        let seen: alloc::vec::Vec<u64> = queue.iter().map(|k| k.event.get()).collect();
        assert_eq!(seen, alloc::vec![2, 1]);
    }
}
