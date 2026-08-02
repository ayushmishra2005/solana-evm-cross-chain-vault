//! Control lane delivery behaviour.

#![allow(clippy::unwrap_used)]

mod common;

use common::{
    HUB, LEG, WATCHER, body_hash_bytes, canonical, control_at, fault, first_body_offset, simulator,
    simulator_with,
};
use protocol_types::{DecodeError, MessageType};
use xchain_sim::{
    ControlRequest, EventStatus, FaultAction, FaultTarget, Lane, LatePolicy, SimError,
    SimulatorConfig, Tick, inspect,
};

#[test]
fn a_message_reaches_the_destination_inbox_at_its_tick() {
    let mut sim = simulator();
    let bytes = canonical(1);
    let event = sim
        .schedule_control(ControlRequest::new(HUB, LEG, bytes.clone(), Tick::new(4)))
        .unwrap();

    sim.run_until_idle();

    let inbox = sim.control_inbox(LEG);
    assert_eq!(inbox.len(), 1);
    let delivered = inbox.first().unwrap();
    assert_eq!(delivered.event, event);
    assert_eq!(delivered.bytes, bytes);
    assert_eq!(delivered.delivered_at, Tick::new(4));
    assert_eq!(delivered.source, HUB);
    assert!(!delivered.after_deadline);
    assert_eq!(
        sim.event(event).map(xchain_sim::Event::status),
        Some(EventStatus::Delivered)
    );
}

#[test]
fn the_transport_identity_matches_the_canonical_message_id() {
    let mut sim = simulator();
    let bytes = canonical(1);
    let expected = protocol_types::decode_message(&bytes)
        .unwrap()
        .message_id()
        .unwrap();
    sim.schedule_control(ControlRequest::new(HUB, LEG, bytes, Tick::ZERO))
        .unwrap();
    sim.deliver_ready();

    let delivered = sim.control_inbox(LEG).first().unwrap();
    assert_eq!(delivered.message_id, expected);
}

#[test]
fn a_message_scheduled_ahead_of_now_stays_pending() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(5, 1)).unwrap();

    assert_eq!(sim.deliver_next(), None);
    assert_eq!(sim.deliver_ready(), 0);
    assert!(sim.control_inbox(LEG).is_empty());
    assert_eq!(sim.queue().len(), 1);
    assert_eq!(
        sim.event(event).map(xchain_sim::Event::status),
        Some(EventStatus::Scheduled)
    );
}

#[test]
fn scheduling_into_the_past_is_refused() {
    let mut sim = simulator();
    sim.advance_to(Tick::new(9)).unwrap();
    let outcome = sim.schedule_control(control_at(2, 1));
    assert!(matches!(outcome, Err(SimError::DeliveryTickInPast { .. })));
    assert_eq!(sim.event_count(), 0);
}

#[test]
fn two_messages_on_one_tick_arrive_in_event_id_order() {
    let mut sim = simulator();
    let first = sim.schedule_control(control_at(3, 1)).unwrap();
    let second = sim.schedule_control(control_at(3, 2)).unwrap();
    sim.run_until_idle();

    let arrived: Vec<_> = sim
        .control_inbox(LEG)
        .iter()
        .map(|entry| entry.event)
        .collect();
    assert_eq!(arrived, vec![first, second]);
}

#[test]
fn an_exact_copy_keeps_the_bytes_and_the_message_id() {
    let mut sim = simulator();
    let original = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(original),
        FaultAction::Duplicate {
            copies: 1,
            spacing: 2,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    let inbox = sim.control_inbox(LEG);
    assert_eq!(inbox.len(), 2);
    let first = inbox.first().unwrap();
    let copy = inbox.get(1).unwrap();
    assert_eq!(copy.bytes, first.bytes);
    assert_eq!(copy.message_id, first.message_id);
    assert_eq!(copy.source, first.source);
    assert_eq!(copy.intended_destination, first.intended_destination);
}

#[test]
fn a_copy_carries_a_new_event_id_and_points_at_the_original() {
    let mut sim = simulator();
    let original = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(original),
        FaultAction::Duplicate {
            copies: 2,
            spacing: 1,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    let inbox = sim.control_inbox(LEG);
    assert_eq!(inbox.len(), 3);
    assert_eq!(inbox.first().unwrap().duplicate_of, None);
    for copy in inbox.iter().skip(1) {
        assert_ne!(copy.event, original);
        assert_eq!(copy.duplicate_of, Some(original));
    }
    let ids: Vec<_> = inbox.iter().map(|entry| entry.event).collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len());
}

#[test]
fn the_simulator_never_removes_a_repeated_message_on_its_own() {
    let mut sim = simulator();
    let first = sim.schedule_control(control_at(1, 1)).unwrap();
    let second = sim.schedule_control(control_at(2, 1)).unwrap();
    sim.run_until_idle();

    let message = sim.event(first).unwrap().message_id().unwrap();
    assert_eq!(sim.event(second).unwrap().message_id(), Some(message));
    assert_eq!(sim.deliveries_of_message(message).count(), 2);
}

#[test]
fn a_dropped_message_never_reaches_an_inbox() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(1, FaultTarget::Event(event), FaultAction::Drop))
        .unwrap();
    sim.run_until_idle();

    assert!(sim.control_inbox(LEG).is_empty());
    assert_eq!(
        sim.event(event).map(xchain_sim::Event::status),
        Some(EventStatus::Dropped)
    );
}

#[test]
fn a_delay_moves_the_delivery_to_a_later_tick() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(2, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Delay { ticks: 7 },
    ))
    .unwrap();
    sim.run_until_idle();

    let delivered = sim.control_inbox(LEG).first().unwrap();
    assert_eq!(delivered.delivered_at, Tick::new(9));
    assert_eq!(sim.event(event).unwrap().attempts(), 2);
}

#[test]
fn swapping_ticks_changes_arrival_order_but_not_the_sequence_field() {
    let mut sim = simulator();
    let early = sim.schedule_control(control_at(1, 11)).unwrap();
    let late = sim.schedule_control(control_at(9, 22)).unwrap();
    sim.swap_delivery_ticks(early, late).unwrap();
    sim.run_until_idle();

    let arrived: Vec<_> = sim
        .control_inbox(LEG)
        .iter()
        .map(|entry| entry.event)
        .collect();
    assert_eq!(arrived, vec![late, early]);

    let sequences: Vec<u64> = sim
        .control_inbox(LEG)
        .iter()
        .map(|entry| inspect::decode(&entry.bytes).unwrap().header.sequence.get())
        .collect();
    assert_eq!(sequences, vec![22, 11]);
}

#[test]
fn moving_one_event_before_another_reorders_only_the_transport() {
    let mut sim = simulator();
    let first = sim.schedule_control(control_at(4, 1)).unwrap();
    let second = sim.schedule_control(control_at(8, 2)).unwrap();
    let bytes_before = sim.event(second).unwrap().control().unwrap().bytes.clone();

    sim.move_before(second, first).unwrap();
    sim.run_until_idle();

    let arrived: Vec<_> = sim
        .control_inbox(LEG)
        .iter()
        .map(|entry| entry.event)
        .collect();
    assert_eq!(arrived, vec![second, first]);
    assert_eq!(sim.control_inbox(LEG).first().unwrap().bytes, bytes_before);
}

#[test]
fn moving_one_event_after_another_pushes_it_back() {
    let mut sim = simulator();
    let first = sim.schedule_control(control_at(4, 1)).unwrap();
    let second = sim.schedule_control(control_at(5, 2)).unwrap();
    sim.move_after(first, second).unwrap();
    sim.run_until_idle();

    let arrived: Vec<_> = sim
        .control_inbox(LEG)
        .iter()
        .map(|entry| entry.event)
        .collect();
    assert_eq!(arrived, vec![second, first]);
}

#[test]
fn reordering_a_finished_event_is_refused() {
    let mut sim = simulator();
    let first = sim.schedule_control(control_at(1, 1)).unwrap();
    let second = sim.schedule_control(control_at(4, 2)).unwrap();
    sim.run_until(Tick::new(1)).unwrap();

    assert!(matches!(
        sim.move_before(first, second),
        Err(SimError::EventAlreadyTerminal(_))
    ));
    assert!(matches!(
        sim.swap_delivery_ticks(first, second),
        Err(SimError::EventAlreadyTerminal(_))
    ));
}

#[test]
fn corruption_changes_exactly_one_byte() {
    let mut sim = simulator();
    let original = canonical(1);
    let event = sim
        .schedule_control(ControlRequest::new(
            HUB,
            LEG,
            original.clone(),
            Tick::new(1),
        ))
        .unwrap();
    let offset = first_body_offset();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::CorruptByte { offset, mask: 0x80 },
    ))
    .unwrap();
    sim.run_until_idle();

    let delivered = sim.control_inbox(LEG).first().unwrap();
    assert_eq!(delivered.bytes.len(), original.len());
    let changed: Vec<usize> = original
        .iter()
        .zip(delivered.bytes.iter())
        .enumerate()
        .filter(|(_, (left, right))| left != right)
        .map(|(position, _)| position)
        .collect();
    assert_eq!(changed, vec![offset]);

    let mutation = delivered.mutation.unwrap();
    assert_eq!(mutation.offset, offset);
    assert_eq!(mutation.to, mutation.from ^ 0x80);
    assert_ne!(mutation.original_message_id, delivered.message_id);
    assert_eq!(delivered.duplicate_of, None);
}

#[test]
fn corruption_leaves_the_stored_body_hash_alone() {
    let mut sim = simulator();
    let original = canonical(1);
    let event = sim
        .schedule_control(ControlRequest::new(
            HUB,
            LEG,
            original.clone(),
            Tick::new(1),
        ))
        .unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::CorruptByte {
            offset: first_body_offset(),
            mask: 0x01,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    let delivered = sim.control_inbox(LEG).first().unwrap();
    assert_eq!(
        body_hash_bytes(&delivered.bytes),
        body_hash_bytes(&original)
    );
}

#[test]
fn a_corrupted_message_fails_to_decode_when_inspected() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::CorruptByte {
            offset: first_body_offset(),
            mask: 0x01,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    let delivered = sim.control_inbox(LEG).first().unwrap();
    assert!(matches!(
        inspect::decode(&delivered.bytes),
        Err(SimError::Decode(DecodeError::BodyHashMismatch))
    ));
}

#[test]
fn a_corrupt_offset_past_the_message_is_refused_when_the_event_is_named() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::CorruptByte {
            offset: 100_000,
            mask: 1,
        },
    ));
    assert!(matches!(outcome, Err(SimError::InvalidConfiguration(_))));
    assert!(sim.plan().is_empty());
}

#[test]
fn a_rerouted_message_shows_the_destination_the_sender_chose() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::ReplaceDestination(WATCHER),
    ))
    .unwrap();
    sim.run_until_idle();

    assert!(sim.control_inbox(LEG).is_empty());
    let delivered = sim.control_inbox(WATCHER).first().unwrap();
    assert_eq!(delivered.intended_destination, LEG);
    assert!(delivered.is_misrouted(WATCHER));
}

#[test]
fn a_halted_destination_holds_the_message() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.halt_endpoint(LEG).unwrap();
    sim.run_until_idle();

    assert!(sim.control_inbox(LEG).is_empty());
    assert_eq!(
        sim.event(event).map(xchain_sim::Event::status),
        Some(EventStatus::Blocked)
    );
    assert!(sim.held().contains(&event));
    assert!(sim.queue().is_empty());
}

#[test]
fn resuming_an_endpoint_lets_a_held_message_through() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.halt_endpoint(LEG).unwrap();
    sim.run_until_idle();
    sim.advance_to(Tick::new(6)).unwrap();
    sim.resume_endpoint(LEG).unwrap();
    sim.run_until_idle();

    let delivered = sim.control_inbox(LEG).first().unwrap();
    assert_eq!(delivered.event, event);
    assert_eq!(delivered.delivered_at, Tick::new(6));
    assert!(sim.held().is_empty());
}

#[test]
fn halting_one_endpoint_leaves_the_others_working() {
    let mut sim = simulator();
    sim.schedule_control(control_at(1, 1)).unwrap();
    sim.schedule_control(ControlRequest::new(
        HUB,
        WATCHER,
        canonical(2),
        Tick::new(1),
    ))
    .unwrap();
    sim.halt_endpoint(LEG).unwrap();
    sim.run_until_idle();

    assert!(sim.control_inbox(LEG).is_empty());
    assert_eq!(sim.control_inbox(WATCHER).len(), 1);
}

#[test]
fn pausing_the_control_lane_stops_every_control_delivery() {
    let mut sim = simulator();
    sim.schedule_control(control_at(1, 1)).unwrap();
    sim.schedule_control(ControlRequest::new(
        HUB,
        WATCHER,
        canonical(2),
        Tick::new(1),
    ))
    .unwrap();
    sim.pause_lane(Lane::Control);
    sim.run_until_idle();

    assert!(sim.control_inbox(LEG).is_empty());
    assert!(sim.control_inbox(WATCHER).is_empty());
    assert_eq!(sim.held().len(), 2);
}

#[test]
fn resuming_the_control_lane_restores_delivery() {
    let mut sim = simulator();
    sim.schedule_control(control_at(1, 1)).unwrap();
    sim.pause_lane(Lane::Control);
    sim.run_until_idle();
    sim.resume_lane(Lane::Control);
    sim.run_until_idle();

    assert_eq!(sim.control_inbox(LEG).len(), 1);
    assert!(sim.held().is_empty());
}

#[test]
fn a_message_past_its_deadline_is_marked_late_under_the_default_policy() {
    let mut sim = simulator();
    sim.schedule_control(
        ControlRequest::new(HUB, LEG, canonical(1), Tick::new(10)).expiring_at(Tick::new(4)),
    )
    .unwrap();
    sim.run_until_idle();

    let delivered = sim.control_inbox(LEG).first().unwrap();
    assert!(delivered.after_deadline);
    assert_eq!(delivered.delivered_at, Tick::new(10));
}

#[test]
fn a_message_past_its_deadline_is_stopped_under_the_expiring_policy() {
    let mut sim = simulator_with(SimulatorConfig {
        control_late_policy: LatePolicy::Expire,
        asset_late_policy: LatePolicy::DeliverWithMarker,
    });
    let event = sim
        .schedule_control(
            ControlRequest::new(HUB, LEG, canonical(1), Tick::new(10)).expiring_at(Tick::new(4)),
        )
        .unwrap();
    sim.run_until_idle();

    assert!(sim.control_inbox(LEG).is_empty());
    assert_eq!(
        sim.event(event).map(xchain_sim::Event::status),
        Some(EventStatus::Expired)
    );
}

#[test]
fn a_message_inside_its_deadline_is_never_marked_late() {
    let mut sim = simulator_with(SimulatorConfig::expiring());
    sim.schedule_control(
        ControlRequest::new(HUB, LEG, canonical(1), Tick::new(3)).expiring_at(Tick::new(3)),
    )
    .unwrap();
    sim.run_until_idle();

    assert!(!sim.control_inbox(LEG).first().unwrap().after_deadline);
}

#[test]
fn the_expiry_helper_reads_the_stamp_the_message_carries() {
    let bytes = canonical(1);
    let stamp = inspect::expiration(&bytes).unwrap();
    assert_eq!(stamp.get(), common::EXPIRES_AT);
}

#[test]
fn trailing_and_malformed_bytes_travel_untouched() {
    let mut sim = simulator();
    let mut trailing = canonical(1);
    trailing.push(0xFF);
    let garbage = vec![0x00, 0x01, 0x02, 0x03];

    sim.schedule_control(ControlRequest::new(HUB, LEG, trailing.clone(), Tick::ZERO))
        .unwrap();
    sim.schedule_control(ControlRequest::new(HUB, LEG, garbage.clone(), Tick::ZERO))
        .unwrap();
    sim.deliver_ready();

    let inbox = sim.control_inbox(LEG);
    assert_eq!(inbox.first().unwrap().bytes, trailing);
    assert_eq!(inbox.get(1).unwrap().bytes, garbage);
    assert!(matches!(
        inspect::decode(&trailing),
        Err(SimError::Decode(DecodeError::TrailingBytes { .. }))
    ));
    assert!(inspect::decode(&garbage).is_err());
}

#[test]
fn a_message_with_no_bytes_is_refused() {
    let mut sim = simulator();
    let outcome = sim.schedule_control(ControlRequest::new(HUB, LEG, Vec::new(), Tick::ZERO));
    assert!(matches!(outcome, Err(SimError::InvalidConfiguration(_))));
}

#[test]
fn a_message_to_an_unknown_endpoint_is_refused() {
    let mut sim = simulator();
    let unknown = xchain_sim::EndpointId::new(99);
    let outcome = sim.schedule_control(ControlRequest::new(HUB, unknown, canonical(1), Tick::ZERO));
    assert!(matches!(outcome, Err(SimError::UnknownEndpoint(_))));
}

#[test]
fn a_message_addressed_to_its_own_sender_is_refused() {
    let mut sim = simulator();
    let outcome = sim.schedule_control(ControlRequest::new(HUB, HUB, canonical(1), Tick::ZERO));
    assert!(matches!(outcome, Err(SimError::InvalidConfiguration(_))));
}

#[test]
fn every_message_kind_travels_without_a_byte_change() {
    for (index, kind) in [
        MessageType::Allocate,
        MessageType::Recall,
        MessageType::RemoteReport,
    ]
    .into_iter()
    .enumerate()
    {
        let mut sim = simulator();
        let bytes = common::canonical_of(kind, 1);
        sim.schedule_control(ControlRequest::new(HUB, LEG, bytes.clone(), Tick::ZERO))
            .unwrap();
        sim.deliver_ready();
        assert_eq!(
            sim.control_inbox(LEG).first().unwrap().bytes,
            bytes,
            "kind {index} changed in flight"
        );
    }
}
