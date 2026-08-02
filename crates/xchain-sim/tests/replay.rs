//! Snapshots, branches, and the promise that a refused step changes nothing.

#![allow(clippy::unwrap_used)]

mod common;

use common::{HUB, LEG, WATCHER, amount, canonical, fault, simulator, transfer};
use xchain_sim::{
    AssetRequest, ControlRequest, EndpointId, EventId, FaultAction, FaultTarget, Lane, Operation,
    SimError, Tick,
};

fn opening_moves() -> Vec<Operation> {
    vec![
        Operation::ScheduleControl(ControlRequest::new(HUB, LEG, canonical(1), Tick::new(1))),
        Operation::ScheduleAsset(AssetRequest::new(
            transfer(0x11),
            HUB,
            LEG,
            amount(900),
            Tick::new(2),
        )),
        Operation::AdvanceTo(Tick::new(1)),
    ]
}

#[test]
fn a_snapshot_can_be_restored_over_and_over() {
    let mut sim = simulator();
    sim.apply_all(&opening_moves()).unwrap();
    let snapshot = sim.snapshot();

    for _ in 0..3 {
        let copy = snapshot.restore();
        assert_eq!(copy.state_hash(), snapshot.state_hash());
        assert_eq!(copy.now(), Tick::new(1));
    }
}

#[test]
fn running_on_a_branch_leaves_the_snapshot_alone() {
    let mut sim = simulator();
    sim.apply_all(&opening_moves()).unwrap();
    let snapshot = sim.snapshot();
    let before = snapshot.state_hash();

    let finished = snapshot.branch(&[Operation::RunUntilIdle]).unwrap();
    assert_eq!(finished.control_inbox(LEG).len(), 1);
    assert_eq!(snapshot.state_hash(), before);
    assert!(snapshot.restore().control_inbox(LEG).is_empty());
}

#[test]
fn two_plans_from_one_point_can_be_compared() {
    let mut sim = simulator();
    sim.apply_all(&opening_moves()).unwrap();
    let snapshot = sim.snapshot();

    let clean = snapshot.branch(&[Operation::RunUntilIdle]).unwrap();
    let broken = snapshot
        .branch(&[
            Operation::AddFault(fault(
                1,
                FaultTarget::Transfer(transfer(0x11)),
                FaultAction::Partial {
                    amount: amount(400),
                },
            )),
            Operation::RunUntilIdle,
        ])
        .unwrap();

    assert_eq!(clean.delivered_for_transfer(transfer(0x11)), 900);
    assert_eq!(broken.delivered_for_transfer(transfer(0x11)), 400);
    assert_ne!(clean.state_hash(), broken.state_hash());
}

#[test]
fn the_same_step_list_rebuilds_the_same_run() {
    let steps = [
        opening_moves(),
        vec![
            Operation::AddFault(fault(
                1,
                FaultTarget::Lane(Lane::Asset),
                FaultAction::Duplicate {
                    copies: 2,
                    spacing: 1,
                },
            )),
            Operation::RunUntilIdle,
            Operation::HaltEndpoint(LEG),
            Operation::ScheduleControl(ControlRequest::new(HUB, LEG, canonical(2), Tick::new(20))),
            Operation::RunUntilIdle,
            Operation::ResumeEndpoint(LEG),
            Operation::RunUntilIdle,
        ],
    ]
    .concat();

    let mut left = simulator();
    left.apply_all(&steps).unwrap();
    let mut right = simulator();
    right.apply_all(&steps).unwrap();

    assert_eq!(left.state_hash(), right.state_hash());
    assert_eq!(left.trace(), right.trace());
    assert_eq!(left.asset_inbox(LEG).len(), 3);
}

#[test]
fn a_refused_schedule_leaves_no_mark() {
    let mut sim = simulator();
    sim.apply_all(&opening_moves()).unwrap();
    let before = sim.state_hash();
    let trace_length = sim.trace().len();

    let outcome = sim.apply(&Operation::ScheduleControl(ControlRequest::new(
        HUB,
        EndpointId::new(404),
        canonical(3),
        Tick::new(5),
    )));

    assert!(matches!(outcome, Err(SimError::UnknownEndpoint(_))));
    assert_eq!(sim.state_hash(), before);
    assert_eq!(sim.trace().len(), trace_length);
}

#[test]
fn a_refused_fault_leaves_no_mark() {
    let mut sim = simulator();
    sim.apply_all(&opening_moves()).unwrap();
    let before = sim.state_hash();

    let outcome = sim.apply(&Operation::AddFault(fault(
        1,
        FaultTarget::Lane(Lane::Asset),
        FaultAction::Split {
            pieces: Vec::new(),
            spacing: 1,
        },
    )));

    assert!(matches!(outcome, Err(SimError::InvalidPartialSplit)));
    assert_eq!(sim.state_hash(), before);
    assert!(sim.plan().is_empty());
}

#[test]
fn a_refused_time_move_leaves_no_mark() {
    let mut sim = simulator();
    sim.apply_all(&opening_moves()).unwrap();
    let before = sim.state_hash();

    let outcome = sim.apply(&Operation::AdvanceTo(Tick::ZERO));
    assert!(matches!(outcome, Err(SimError::TimeMovesBackwards { .. })));
    assert_eq!(sim.state_hash(), before);
    assert_eq!(sim.now(), Tick::new(1));
}

#[test]
fn a_refused_reorder_leaves_no_mark() {
    let mut sim = simulator();
    sim.apply_all(&opening_moves()).unwrap();
    let before = sim.state_hash();

    let outcome = sim.apply(&Operation::MoveBefore {
        event: EventId::new(1),
        other: EventId::new(999),
    });
    assert!(matches!(outcome, Err(SimError::UnknownEvent(_))));
    assert_eq!(sim.state_hash(), before);
}

#[test]
fn a_step_list_stops_at_the_first_refusal() {
    let mut sim = simulator();
    let outcome = sim.apply_all(&[
        Operation::ScheduleControl(ControlRequest::new(HUB, LEG, canonical(1), Tick::new(1))),
        Operation::HaltEndpoint(EndpointId::new(404)),
        Operation::ScheduleControl(ControlRequest::new(HUB, LEG, canonical(2), Tick::new(1))),
    ]);

    assert!(matches!(outcome, Err(SimError::UnknownEndpoint(_))));
    assert_eq!(sim.event_count(), 1);
}

#[test]
fn a_best_effort_run_reports_each_step_and_keeps_going() {
    let mut sim = simulator();
    let results = sim.apply_best_effort(&[
        Operation::ScheduleControl(ControlRequest::new(HUB, LEG, canonical(1), Tick::new(1))),
        Operation::HaltEndpoint(EndpointId::new(404)),
        Operation::ScheduleControl(ControlRequest::new(
            HUB,
            WATCHER,
            canonical(2),
            Tick::new(1),
        )),
        Operation::RunUntilIdle,
    ]);

    assert_eq!(results.len(), 4);
    assert!(results.first().unwrap().is_ok());
    assert!(results.get(1).unwrap().is_err());
    assert!(results.get(2).unwrap().is_ok());
    assert_eq!(sim.control_inbox(LEG).len(), 1);
    assert_eq!(sim.control_inbox(WATCHER).len(), 1);
}

#[test]
fn a_named_event_id_makes_a_run_stable_across_rebuilds() {
    let steps = vec![
        Operation::ScheduleControl(
            ControlRequest::new(HUB, LEG, canonical(1), Tick::new(1)).with_id(EventId::new(100)),
        ),
        Operation::AddFault(fault(
            1,
            FaultTarget::Event(EventId::new(100)),
            FaultAction::Delay { ticks: 2 },
        )),
        Operation::RunUntilIdle,
    ];

    let mut left = simulator();
    left.apply_all(&steps).unwrap();
    let mut right = simulator();
    right.apply_all(&steps).unwrap();
    assert_eq!(left.state_hash(), right.state_hash());
    assert_eq!(
        left.control_inbox(LEG).first().unwrap().delivered_at,
        Tick::new(3)
    );
}

#[test]
fn reusing_an_event_id_is_refused() {
    let mut sim = simulator();
    sim.schedule_control(
        ControlRequest::new(HUB, LEG, canonical(1), Tick::new(1)).with_id(EventId::new(7)),
    )
    .unwrap();
    let outcome = sim.schedule_control(
        ControlRequest::new(HUB, LEG, canonical(2), Tick::new(1)).with_id(EventId::new(7)),
    );
    assert!(matches!(outcome, Err(SimError::DuplicateEventId(_))));
    assert_eq!(sim.event_count(), 1);
}

#[test]
fn a_named_event_id_does_not_collide_with_the_counter() {
    let mut sim = simulator();
    let named = sim
        .schedule_control(
            ControlRequest::new(HUB, LEG, canonical(1), Tick::new(1)).with_id(EventId::new(50)),
        )
        .unwrap();
    let automatic = sim
        .schedule_control(ControlRequest::new(HUB, LEG, canonical(2), Tick::new(1)))
        .unwrap();
    assert_eq!(named, EventId::new(50));
    assert_eq!(automatic, EventId::new(51));
}

#[test]
fn a_duplicate_never_reuses_an_identifier_the_caller_took() {
    let mut sim = simulator();
    let event = sim
        .schedule_control(
            ControlRequest::new(HUB, LEG, canonical(1), Tick::new(1)).with_id(EventId::new(2)),
        )
        .unwrap();
    sim.schedule_control(
        ControlRequest::new(HUB, LEG, canonical(2), Tick::new(9)).with_id(EventId::new(3)),
    )
    .unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Duplicate {
            copies: 1,
            spacing: 1,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    let mut ids: Vec<u64> = sim.events().map(|event| event.id().get()).collect();
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count);
    assert_eq!(count, 3);
}

#[test]
fn an_adding_endpoint_step_keeps_the_run_valid() {
    let mut sim = simulator();
    let extra = EndpointId::new(9);
    sim.add_endpoint(extra).unwrap();
    assert!(matches!(
        sim.add_endpoint(extra),
        Err(SimError::DuplicateEndpoint(_))
    ));

    sim.schedule_asset(AssetRequest::new(
        transfer(0x22),
        HUB,
        extra,
        amount(10),
        Tick::ZERO,
    ))
    .unwrap();
    sim.run_until_idle();
    assert_eq!(sim.asset_inbox(extra).len(), 1);
}

#[test]
fn a_simulator_needs_at_least_one_endpoint() {
    assert!(matches!(
        xchain_sim::Simulator::new(&[]),
        Err(SimError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        xchain_sim::Simulator::new(&[HUB, HUB]),
        Err(SimError::DuplicateEndpoint(_))
    ));
}
