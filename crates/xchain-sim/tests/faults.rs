//! Fault composition, ordering, and refusal of setups that cannot combine.

#![allow(clippy::unwrap_used)]

mod common;

use common::{
    HUB, LEG, WATCHER, amount, asset_at, control_at, fault, first_body_offset, simulator, transfer,
};
use xchain_sim::{
    EventStatus, FaultAction, FaultStage, FaultTarget, Lane, RejectReason, SimError, Tick,
    TraceAction,
};

/// Order in which the trace shows fault work for one event.
fn effect_stages(sim: &xchain_sim::Simulator, event: xchain_sim::EventId) -> Vec<FaultStage> {
    sim.trace()
        .records()
        .iter()
        .filter_map(|record| match record.action {
            TraceAction::FaultApplied {
                fault, event: id, ..
            } if id == event => sim.plan().get(fault).map(|f| f.action.stage()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_delayed_message_still_produces_its_copy() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Delay { ticks: 4 },
    ))
    .unwrap();
    sim.add_fault(fault(
        2,
        FaultTarget::Event(event),
        FaultAction::Duplicate {
            copies: 1,
            spacing: 0,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    let inbox = sim.control_inbox(LEG);
    assert_eq!(inbox.len(), 2);
    let copy = inbox.first().unwrap();
    let original = inbox.get(1).unwrap();
    assert_eq!(copy.duplicate_of, Some(event));
    assert_eq!(copy.delivered_at, Tick::new(1));
    assert_eq!(original.event, event);
    assert_eq!(original.delivered_at, Tick::new(5));
}

#[test]
fn a_copy_carries_the_corruption_that_was_applied_before_it() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::CorruptByte {
            offset: first_body_offset(),
            mask: 0x0F,
        },
    ))
    .unwrap();
    sim.add_fault(fault(
        2,
        FaultTarget::Event(event),
        FaultAction::Duplicate {
            copies: 1,
            spacing: 1,
        },
    ))
    .unwrap();
    sim.add_fault(fault(
        3,
        FaultTarget::Event(event),
        FaultAction::Delay { ticks: 6 },
    ))
    .unwrap();
    sim.run_until_idle();

    assert_eq!(
        effect_stages(&sim, event),
        vec![FaultStage::Content, FaultStage::Fanout, FaultStage::Delay]
    );
    let inbox = sim.control_inbox(LEG);
    assert_eq!(inbox.len(), 2);
    assert_eq!(inbox.first().unwrap().bytes, inbox.get(1).unwrap().bytes);
    assert!(inbox.iter().all(|entry| entry.mutation.is_some()));
}

#[test]
fn a_reroute_is_decided_before_a_drop_removes_the_original() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::ReplaceDestination(WATCHER),
    ))
    .unwrap();
    sim.add_fault(fault(2, FaultTarget::Event(event), FaultAction::Drop))
        .unwrap();
    sim.run_until_idle();

    assert_eq!(
        effect_stages(&sim, event),
        vec![FaultStage::Reroute, FaultStage::Drop]
    );
    assert!(sim.control_inbox(LEG).is_empty());
    assert!(sim.control_inbox(WATCHER).is_empty());
    assert_eq!(
        sim.event(event).map(xchain_sim::Event::status),
        Some(EventStatus::Dropped)
    );
    assert_eq!(sim.event(event).unwrap().destination(), WATCHER);
}

#[test]
fn a_copy_made_before_a_drop_survives_the_drop() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Duplicate {
            copies: 1,
            spacing: 2,
        },
    ))
    .unwrap();
    sim.add_fault(fault(2, FaultTarget::Event(event), FaultAction::Drop))
        .unwrap();
    sim.run_until_idle();

    let inbox = sim.control_inbox(LEG);
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox.first().unwrap().duplicate_of, Some(event));
    assert_eq!(
        sim.event(event).map(xchain_sim::Event::status),
        Some(EventStatus::Dropped)
    );
}

#[test]
fn a_split_is_made_before_the_original_piece_is_delayed() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x11, 900)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Split {
            pieces: vec![amount(400), amount(500)],
            spacing: 1,
        },
    ))
    .unwrap();
    sim.add_fault(fault(
        2,
        FaultTarget::Event(event),
        FaultAction::Delay { ticks: 10 },
    ))
    .unwrap();
    sim.run_until_idle();

    assert_eq!(
        effect_stages(&sim, event),
        vec![FaultStage::Content, FaultStage::Delay]
    );
    let inbox = sim.asset_inbox(LEG);
    assert_eq!(inbox.len(), 2);
    assert_eq!(inbox.first().unwrap().piece, Some(1));
    assert_eq!(inbox.first().unwrap().delivered_at, Tick::new(2));
    assert_eq!(inbox.get(1).unwrap().piece, Some(0));
    assert_eq!(inbox.get(1).unwrap().delivered_at, Tick::new(11));
    assert_eq!(sim.delivered_for_transfer(transfer(0x11)), 900);
}

#[test]
fn two_splits_on_one_event_are_refused_when_the_fault_is_added() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x11, 900)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Split {
            pieces: vec![amount(400)],
            spacing: 1,
        },
    ))
    .unwrap();
    let outcome = sim.add_fault(fault(
        2,
        FaultTarget::Event(event),
        FaultAction::Split {
            pieces: vec![amount(500)],
            spacing: 1,
        },
    ));
    assert!(matches!(outcome, Err(SimError::ConflictingFaults { .. })));
    assert_eq!(sim.plan().len(), 1);
}

#[test]
fn a_split_and_a_partial_on_one_event_are_refused() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x11, 900)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Partial {
            amount: amount(100),
        },
    ))
    .unwrap();
    let outcome = sim.add_fault(fault(
        2,
        FaultTarget::Event(event),
        FaultAction::Split {
            pieces: vec![amount(500)],
            spacing: 1,
        },
    ));
    assert!(matches!(outcome, Err(SimError::ConflictingFaults { .. })));
}

#[test]
fn two_content_faults_that_meet_only_at_delivery_reject_the_event() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x11, 900)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Partial {
            amount: amount(100),
        },
    ))
    .unwrap();
    sim.add_fault(fault(
        2,
        FaultTarget::Lane(Lane::Asset),
        FaultAction::Partial {
            amount: amount(200),
        },
    ))
    .unwrap();
    sim.run_until_idle();

    assert!(sim.asset_inbox(LEG).is_empty());
    assert_eq!(
        sim.event(event).map(xchain_sim::Event::status),
        Some(EventStatus::RejectedBySimulator)
    );
    assert!(sim.trace().records().iter().any(|record| matches!(
        record.action,
        TraceAction::FaultRejected {
            reason: RejectReason::ConflictingGroup,
            ..
        }
    )));
}

#[test]
fn the_same_fault_identifier_cannot_be_added_twice() {
    let mut sim = simulator();
    sim.add_fault(fault(
        1,
        FaultTarget::Lane(Lane::Control),
        FaultAction::Drop,
    ))
    .unwrap();
    let outcome = sim.add_fault(fault(1, FaultTarget::Lane(Lane::Asset), FaultAction::Drop));
    assert!(matches!(outcome, Err(SimError::DuplicateFaultId(_))));
    assert_eq!(sim.plan().len(), 1);
}

#[test]
fn a_fault_naming_an_unknown_event_is_refused() {
    let mut sim = simulator();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Event(xchain_sim::EventId::new(404)),
        FaultAction::Drop,
    ));
    assert!(matches!(outcome, Err(SimError::UnknownEvent(_))));
}

#[test]
fn a_fault_naming_a_finished_event_is_refused() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.run_until_idle();
    let outcome = sim.add_fault(fault(1, FaultTarget::Event(event), FaultAction::Drop));
    assert!(matches!(outcome, Err(SimError::EventAlreadyTerminal(_))));
}

#[test]
fn a_reroute_to_an_unknown_endpoint_is_refused() {
    let mut sim = simulator();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Lane(Lane::Control),
        FaultAction::ReplaceDestination(xchain_sim::EndpointId::new(77)),
    ));
    assert!(matches!(outcome, Err(SimError::UnknownEndpoint(_))));
}

#[test]
fn a_copy_fault_asking_for_no_copies_is_refused() {
    let mut sim = simulator();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Lane(Lane::Control),
        FaultAction::Duplicate {
            copies: 0,
            spacing: 1,
        },
    ));
    assert!(matches!(outcome, Err(SimError::InvalidConfiguration(_))));
}

#[test]
fn a_corruption_mask_that_changes_nothing_is_refused() {
    let mut sim = simulator();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Lane(Lane::Control),
        FaultAction::CorruptByte { offset: 0, mask: 0 },
    ));
    assert!(matches!(outcome, Err(SimError::InvalidConfiguration(_))));
}

#[test]
fn a_route_fault_reaches_only_that_route() {
    let mut sim = simulator();
    sim.schedule_control(control_at(1, 1)).unwrap();
    sim.schedule_control(xchain_sim::ControlRequest::new(
        HUB,
        WATCHER,
        common::canonical(2),
        Tick::new(1),
    ))
    .unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Route {
            source: HUB,
            destination: LEG,
            lane: Lane::Control,
        },
        FaultAction::Drop,
    ))
    .unwrap();
    sim.run_until_idle();

    assert!(sim.control_inbox(LEG).is_empty());
    assert_eq!(sim.control_inbox(WATCHER).len(), 1);
}

#[test]
fn a_route_fault_naming_an_unknown_endpoint_is_refused() {
    let mut sim = simulator();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Route {
            source: HUB,
            destination: xchain_sim::EndpointId::new(88),
            lane: Lane::Control,
        },
        FaultAction::Drop,
    ));
    assert!(matches!(outcome, Err(SimError::UnknownEndpoint(_))));
}

#[test]
fn a_lane_wide_copy_fault_does_not_copy_its_own_copies() {
    let mut sim = simulator();
    sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Lane(Lane::Control),
        FaultAction::Duplicate {
            copies: 1,
            spacing: 1,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    assert_eq!(sim.control_inbox(LEG).len(), 2);
    assert_eq!(sim.event_count(), 2);
}

#[test]
fn a_lane_wide_split_does_not_split_the_pieces_it_made() {
    let mut sim = simulator();
    sim.schedule_asset(asset_at(1, 0x11, 900)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Lane(Lane::Asset),
        FaultAction::Split {
            pieces: vec![amount(300), amount(300)],
            spacing: 1,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    assert_eq!(sim.asset_inbox(LEG).len(), 2);
    assert_eq!(sim.delivered_for_transfer(transfer(0x11)), 600);
}

#[test]
fn one_fault_applies_to_one_event_only_once() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Lane(Lane::Control),
        FaultAction::Delay { ticks: 3 },
    ))
    .unwrap();
    sim.run_until_idle();

    assert_eq!(
        sim.control_inbox(LEG).first().unwrap().delivered_at,
        Tick::new(4)
    );
    assert_eq!(effect_stages(&sim, event), vec![FaultStage::Delay]);
}

#[test]
fn a_held_event_still_gets_its_fault_applied_only_once() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Delay { ticks: 2 },
    ))
    .unwrap();
    sim.halt_endpoint(LEG).unwrap();
    sim.run_until_idle();
    sim.resume_endpoint(LEG).unwrap();
    sim.run_until_idle();

    assert_eq!(effect_stages(&sim, event), vec![FaultStage::Delay]);
    assert_eq!(
        sim.control_inbox(LEG).first().unwrap().delivered_at,
        Tick::new(3)
    );
}

#[test]
fn a_split_that_would_exceed_the_request_rejects_the_event_at_delivery() {
    let mut sim = simulator();
    let small = sim.schedule_asset(asset_at(1, 0x11, 50)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Lane(Lane::Asset),
        FaultAction::Split {
            pieces: vec![amount(80), amount(80)],
            spacing: 1,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    assert!(sim.asset_inbox(LEG).is_empty());
    assert_eq!(
        sim.event(small).map(xchain_sim::Event::status),
        Some(EventStatus::RejectedBySimulator)
    );
    assert!(sim.trace().records().iter().any(|record| matches!(
        record.action,
        TraceAction::FaultRejected {
            reason: RejectReason::SplitExceedsRequest,
            ..
        }
    )));
}

#[test]
fn fault_work_for_one_event_always_appears_in_stage_order() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    // Identifiers run against the stage order on purpose.
    sim.add_fault(fault(
        9,
        FaultTarget::Event(event),
        FaultAction::ReplaceDestination(WATCHER),
    ))
    .unwrap();
    sim.add_fault(fault(
        7,
        FaultTarget::Event(event),
        FaultAction::CorruptByte {
            offset: first_body_offset(),
            mask: 0x22,
        },
    ))
    .unwrap();
    sim.add_fault(fault(
        5,
        FaultTarget::Event(event),
        FaultAction::Duplicate {
            copies: 1,
            spacing: 1,
        },
    ))
    .unwrap();
    sim.add_fault(fault(
        3,
        FaultTarget::Event(event),
        FaultAction::Delay { ticks: 2 },
    ))
    .unwrap();
    sim.run_until_idle();

    assert_eq!(
        effect_stages(&sim, event),
        vec![
            FaultStage::Reroute,
            FaultStage::Content,
            FaultStage::Fanout,
            FaultStage::Delay
        ]
    );
    assert_eq!(sim.control_inbox(WATCHER).len(), 2);
}

#[test]
fn a_second_corruption_of_one_message_is_refused() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::CorruptByte {
            offset: first_body_offset(),
            mask: 0x0F,
        },
    ))
    .unwrap();
    sim.add_fault(fault(
        2,
        FaultTarget::Event(event),
        FaultAction::Duplicate {
            copies: 1,
            spacing: 1,
        },
    ))
    .unwrap();
    sim.run_until(Tick::new(1)).unwrap();

    // The copy inherited the edit, so a second edit on it is refused.
    let copy = sim
        .events()
        .find(|candidate| candidate.duplicate_of() == Some(event))
        .map(xchain_sim::Event::id)
        .unwrap();
    sim.add_fault(fault(
        3,
        FaultTarget::Event(copy),
        FaultAction::CorruptByte {
            offset: first_body_offset(),
            mask: 0x0F,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    assert_eq!(sim.control_inbox(LEG).len(), 1);
    assert_eq!(
        sim.event(copy).map(xchain_sim::Event::status),
        Some(EventStatus::RejectedBySimulator)
    );
    assert!(sim.trace().records().iter().any(|record| matches!(
        record.action,
        TraceAction::FaultRejected {
            reason: RejectReason::AlreadyCorrupted,
            ..
        }
    )));
}

#[test]
fn a_fault_added_after_the_first_attempt_is_refused() {
    let mut sim = simulator();
    let event = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Delay { ticks: 5 },
    ))
    .unwrap();
    sim.run_until(Tick::new(1)).unwrap();

    let outcome = sim.add_fault(fault(2, FaultTarget::Event(event), FaultAction::Drop));
    assert!(matches!(outcome, Err(SimError::FaultsAlreadyBound(_))));

    sim.run_until_idle();
    assert_eq!(sim.control_inbox(LEG).len(), 1);
}

#[test]
fn a_lane_fault_added_after_an_event_was_attempted_leaves_that_event_alone() {
    let mut sim = simulator();
    let delayed = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(delayed),
        FaultAction::Delay { ticks: 5 },
    ))
    .unwrap();
    sim.run_until(Tick::new(1)).unwrap();

    sim.add_fault(fault(
        2,
        FaultTarget::Lane(Lane::Control),
        FaultAction::Drop,
    ))
    .unwrap();
    sim.schedule_control(control_at(2, 2)).unwrap();
    sim.run_until_idle();

    let arrived: Vec<_> = sim
        .control_inbox(LEG)
        .iter()
        .map(|entry| entry.event)
        .collect();
    assert_eq!(arrived, vec![delayed]);
}

#[test]
fn an_overdue_event_is_delivered_where_the_clock_stands() {
    let mut sim = simulator();
    sim.schedule_control(control_at(2, 1)).unwrap();
    sim.advance_to(Tick::new(30)).unwrap();
    sim.run_until_idle();

    assert_eq!(
        sim.control_inbox(LEG).first().unwrap().delivered_at,
        Tick::new(30)
    );
    assert!(sim.queue().is_empty());
}

#[test]
fn a_message_fault_reaches_every_event_carrying_those_bytes() {
    let mut sim = simulator();
    let first = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.schedule_control(control_at(2, 1)).unwrap();
    sim.schedule_control(control_at(3, 2)).unwrap();
    let identity = sim.event(first).unwrap().message_id().unwrap();
    sim.add_fault(fault(1, FaultTarget::Message(identity), FaultAction::Drop))
        .unwrap();
    sim.run_until_idle();

    assert_eq!(sim.control_inbox(LEG).len(), 1);
}
