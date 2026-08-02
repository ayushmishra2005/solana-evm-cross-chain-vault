//! Repeatability of order, trace, and digest.

#![allow(clippy::unwrap_used)]

mod common;

use common::{HUB, LEG, WATCHER, amount, canonical, fault, simulator, transfer};
use xchain_sim::{
    AssetRequest, ControlRequest, EndpointId, EventId, FaultAction, FaultTarget, Lane, Operation,
    Simulator, Tick, TraceAction, seeded_plan,
};

fn mixed_run() -> Vec<Operation> {
    vec![
        Operation::ScheduleControl(ControlRequest::new(HUB, LEG, canonical(1), Tick::new(2))),
        Operation::ScheduleAsset(AssetRequest::new(
            transfer(0x11),
            HUB,
            LEG,
            amount(1_000),
            Tick::new(2),
        )),
        Operation::ScheduleControl(ControlRequest::new(
            HUB,
            WATCHER,
            canonical(2),
            Tick::new(4),
        )),
        Operation::AddFault(fault(
            1,
            FaultTarget::Lane(Lane::Control),
            FaultAction::Delay { ticks: 1 },
        )),
        Operation::AdvanceTo(Tick::new(2)),
        Operation::DeliverReady,
        Operation::HaltEndpoint(WATCHER),
        Operation::RunUntil(Tick::new(6)),
        Operation::ResumeEndpoint(WATCHER),
        Operation::PauseLane(Lane::Asset),
        Operation::RunUntilIdle,
        Operation::ResumeLane(Lane::Asset),
        Operation::RunUntilIdle,
    ]
}

fn run(operations: &[Operation]) -> Simulator {
    let mut sim = simulator();
    sim.apply_all(operations).unwrap();
    sim
}

#[test]
fn the_same_operations_write_the_same_trace() {
    let left = run(&mixed_run());
    let right = run(&mixed_run());
    assert_eq!(left.trace(), right.trace());
    assert!(!left.trace().is_empty());
}

#[test]
fn the_same_operations_reach_the_same_digest() {
    let left = run(&mixed_run());
    let right = run(&mixed_run());
    assert_eq!(left.state_hash(), right.state_hash());
}

#[test]
fn the_same_operations_fill_the_same_inboxes() {
    let left = run(&mixed_run());
    let right = run(&mixed_run());
    assert_eq!(left.control_inbox(LEG), right.control_inbox(LEG));
    assert_eq!(left.asset_inbox(LEG), right.asset_inbox(LEG));
    assert_eq!(left.control_inbox(WATCHER), right.control_inbox(WATCHER));
}

#[test]
fn a_seed_always_lays_out_the_same_plan() {
    let targets = [
        (EventId::new(1), Lane::Control),
        (EventId::new(2), Lane::Asset),
        (EventId::new(3), Lane::Control),
    ];
    assert_eq!(seeded_plan(4_242, &targets), seeded_plan(4_242, &targets));
}

#[test]
fn a_seeded_plan_can_be_printed_for_a_failure_report() {
    let targets = [(EventId::new(1), Lane::Control)];
    let text = format!("{}", seeded_plan(9, &targets));
    assert!(text.starts_with("plan["));
    assert!(text.ends_with(']'));
}

#[test]
fn two_different_plans_reach_two_different_digests() {
    let base = vec![
        Operation::ScheduleControl(ControlRequest::new(HUB, LEG, canonical(1), Tick::new(1))),
        Operation::ScheduleAsset(AssetRequest::new(
            transfer(0x11),
            HUB,
            LEG,
            amount(500),
            Tick::new(1),
        )),
    ];
    let mut start = simulator();
    start.apply_all(&base).unwrap();
    let snapshot = start.snapshot();

    let dropped = snapshot
        .branch(&[
            Operation::AddFault(fault(
                1,
                FaultTarget::Lane(Lane::Control),
                FaultAction::Drop,
            )),
            Operation::RunUntilIdle,
        ])
        .unwrap();
    let delayed = snapshot
        .branch(&[
            Operation::AddFault(fault(
                1,
                FaultTarget::Lane(Lane::Control),
                FaultAction::Delay { ticks: 3 },
            )),
            Operation::RunUntilIdle,
        ])
        .unwrap();

    assert_ne!(dropped.state_hash(), delayed.state_hash());
    assert!(dropped.control_inbox(LEG).is_empty());
    assert_eq!(delayed.control_inbox(LEG).len(), 1);
}

#[test]
fn a_cloned_snapshot_replays_to_the_same_digest() {
    let mut sim = simulator();
    sim.apply_all(&mixed_run()[..4]).unwrap();
    let snapshot = sim.snapshot();
    let tail = &mixed_run()[4..];

    let left = snapshot.branch(tail).unwrap();
    let right = snapshot.branch(tail).unwrap();
    assert_eq!(left.state_hash(), right.state_hash());
    assert_eq!(left.trace(), right.trace());
}

#[test]
fn queue_order_does_not_follow_the_order_events_were_added() {
    let forward = vec![
        Operation::ScheduleControl(
            ControlRequest::new(HUB, LEG, canonical(1), Tick::new(9)).with_id(EventId::new(10)),
        ),
        Operation::ScheduleControl(
            ControlRequest::new(HUB, LEG, canonical(2), Tick::new(1)).with_id(EventId::new(20)),
        ),
        Operation::RunUntilIdle,
    ];
    let backward = vec![
        Operation::ScheduleControl(
            ControlRequest::new(HUB, LEG, canonical(2), Tick::new(1)).with_id(EventId::new(20)),
        ),
        Operation::ScheduleControl(
            ControlRequest::new(HUB, LEG, canonical(1), Tick::new(9)).with_id(EventId::new(10)),
        ),
        Operation::RunUntilIdle,
    ];

    let arrival = |sim: &Simulator| -> Vec<EventId> {
        sim.control_inbox(LEG)
            .iter()
            .map(|entry| entry.event)
            .collect()
    };
    assert_eq!(arrival(&run(&forward)), arrival(&run(&backward)));
    assert_eq!(
        arrival(&run(&forward)),
        vec![EventId::new(20), EventId::new(10)]
    );
}

#[test]
fn the_digest_ignores_how_a_value_prints() {
    let mut left = simulator();
    let mut right = simulator();
    for sim in [&mut left, &mut right] {
        sim.schedule_control(ControlRequest::new(HUB, LEG, canonical(1), Tick::new(1)))
            .unwrap();
        sim.run_until_idle();
    }
    assert_eq!(left.state_hash(), right.state_hash());

    // A digest that leaned on Debug text would move with the endpoint list.
    let mut wider = Simulator::new(&[HUB, LEG, WATCHER, EndpointId::new(4)]).unwrap();
    wider
        .schedule_control(ControlRequest::new(HUB, LEG, canonical(1), Tick::new(1)))
        .unwrap();
    wider.run_until_idle();
    assert_ne!(left.state_hash(), wider.state_hash());
}

#[test]
fn the_lower_event_id_always_wins_a_shared_tick() {
    for (first, second) in [
        (EventId::new(5), EventId::new(6)),
        (EventId::new(6), EventId::new(5)),
    ] {
        let mut sim = simulator();
        sim.schedule_control(
            ControlRequest::new(HUB, LEG, canonical(1), Tick::new(3)).with_id(first),
        )
        .unwrap();
        sim.schedule_control(
            ControlRequest::new(HUB, LEG, canonical(2), Tick::new(3)).with_id(second),
        )
        .unwrap();
        sim.run_until_idle();

        let arrived: Vec<_> = sim
            .control_inbox(LEG)
            .iter()
            .map(|entry| entry.event)
            .collect();
        assert_eq!(arrived, vec![EventId::new(5), EventId::new(6)]);
    }
}

#[test]
fn a_digest_moves_when_the_clock_moves() {
    let mut sim = simulator();
    let before = sim.state_hash();
    sim.advance_by(1).unwrap();
    assert_ne!(sim.state_hash(), before);
}

#[test]
fn a_digest_moves_when_an_endpoint_halts() {
    let mut sim = simulator();
    let before = sim.state_hash();
    sim.halt_endpoint(LEG).unwrap();
    assert_ne!(sim.state_hash(), before);
}

#[test]
fn a_digest_moves_when_a_lane_pauses() {
    let mut sim = simulator();
    let before = sim.state_hash();
    sim.pause_lane(Lane::Asset);
    assert_ne!(sim.state_hash(), before);
}

#[test]
fn a_digest_moves_when_an_inbox_gains_an_entry() {
    let mut sim = simulator();
    sim.schedule_control(ControlRequest::new(HUB, LEG, canonical(1), Tick::ZERO))
        .unwrap();
    let before = sim.state_hash();
    sim.deliver_ready();
    assert_ne!(sim.state_hash(), before);
}

#[test]
fn a_digest_moves_when_a_fault_joins_the_plan() {
    let mut sim = simulator();
    let before = sim.state_hash();
    sim.add_fault(fault(
        1,
        FaultTarget::Lane(Lane::Control),
        FaultAction::Drop,
    ))
    .unwrap();
    assert_ne!(sim.state_hash(), before);
}

#[test]
fn trace_positions_run_without_a_gap() {
    let sim = run(&mixed_run());
    assert!(sim.trace().indices_are_contiguous());
    let positions: Vec<u32> = sim
        .trace()
        .records()
        .iter()
        .map(|record| record.index.get())
        .collect();
    assert_eq!(positions.first(), Some(&0));
    assert_eq!(positions.len(), sim.trace().len());
}

#[test]
fn the_clock_never_steps_backwards() {
    let sim = run(&mixed_run());
    let mut last = Tick::ZERO;
    for record in sim.trace().records() {
        assert!(record.tick >= last, "trace tick went backwards");
        last = record.tick;
    }
    assert!(matches!(
        run(&mixed_run()).advance_to(Tick::ZERO),
        Err(xchain_sim::SimError::TimeMovesBackwards { .. })
    ));
}

#[test]
fn every_delivered_event_was_scheduled_first() {
    let sim = run(&mixed_run());
    let scheduled: Vec<EventId> = sim
        .trace()
        .records()
        .iter()
        .filter_map(|record| match record.action {
            TraceAction::EventScheduled { event, .. } => Some(event),
            TraceAction::DuplicateCreated { duplicate, .. } => Some(duplicate),
            TraceAction::PartialDeliveryCreated { piece_event, .. } => Some(piece_event),
            _ => None,
        })
        .collect();
    for record in sim.trace().records() {
        if let TraceAction::EventDelivered { event, .. } = record.action {
            assert!(scheduled.contains(&event), "{event} was never scheduled");
        }
    }
}
