//! Properties over sampled operation streams.
//!
//! Set PROPTEST_CASES to widen a soak run.

#![allow(clippy::unwrap_used)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{HUB, LEG, WATCHER, canonical, simulator};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use protocol_types::{AssetAmount, TransferId};
use xchain_sim::{
    AssetRequest, ControlRequest, EndpointId, Event, EventId, EventStatus, Fault, FaultAction,
    FaultId, FaultTarget, Lane, Operation, Simulator, Tick, TraceAction, seeded_plan,
};

const ENDPOINTS: [EndpointId; 3] = [HUB, LEG, WATCHER];
const MAX_TICK: u64 = 40;

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(256)
}

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: cases(),
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/properties.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    }
}

fn endpoint() -> impl Strategy<Value = EndpointId> {
    prop::sample::select(ENDPOINTS.as_slice())
}

fn lane() -> impl Strategy<Value = Lane> {
    prop_oneof![Just(Lane::Control), Just(Lane::Asset)]
}

fn tick() -> impl Strategy<Value = Tick> {
    (0..MAX_TICK).prop_map(Tick::new)
}

/// Either a canonical message or a byte string the codec would refuse.
fn payload() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        4 => (1u64..7).prop_map(canonical),
        1 => prop::collection::vec(any::<u8>(), 1..40),
    ]
}

fn transfer() -> impl Strategy<Value = TransferId> {
    (0u8..4).prop_map(|tag| TransferId::new([tag; 32]))
}

fn control_request() -> impl Strategy<Value = ControlRequest> {
    (
        endpoint(),
        endpoint(),
        payload(),
        tick(),
        prop::option::of(tick()),
    )
        .prop_map(|(source, destination, bytes, at, deadline)| {
            let request = ControlRequest::new(source, destination, bytes, at);
            match deadline {
                Some(limit) => request.expiring_at(limit),
                None => request,
            }
        })
}

fn asset_request() -> impl Strategy<Value = AssetRequest> {
    (
        transfer(),
        endpoint(),
        endpoint(),
        1u128..2_000,
        tick(),
        prop::option::of(tick()),
    )
        .prop_map(|(id, source, destination, value, at, deadline)| {
            let request = AssetRequest::new(id, source, destination, AssetAmount::new(value), at);
            match deadline {
                Some(limit) => request.timing_out_at(limit),
                None => request,
            }
        })
}

fn control_action() -> impl Strategy<Value = FaultAction> {
    prop_oneof![
        Just(FaultAction::Drop),
        (1u64..6).prop_map(|ticks| FaultAction::Delay { ticks }),
        (1u16..3, 0u64..4).prop_map(|(copies, spacing)| FaultAction::Duplicate { copies, spacing }),
        endpoint().prop_map(FaultAction::ReplaceDestination),
        (0usize..40, 1u8..=255)
            .prop_map(|(offset, mask)| FaultAction::CorruptByte { offset, mask }),
    ]
}

fn asset_action() -> impl Strategy<Value = FaultAction> {
    prop_oneof![
        Just(FaultAction::Drop),
        (1u64..6).prop_map(|ticks| FaultAction::Delay { ticks }),
        (1u16..3, 0u64..4).prop_map(|(copies, spacing)| FaultAction::Duplicate { copies, spacing }),
        endpoint().prop_map(FaultAction::ReplaceDestination),
        (1u128..800).prop_map(|value| FaultAction::Partial {
            amount: AssetAmount::new(value)
        }),
        (prop::collection::vec(1u128..500, 1..4), 0u64..3).prop_map(|(values, spacing)| {
            FaultAction::Split {
                pieces: values.into_iter().map(AssetAmount::new).collect(),
                spacing,
            }
        }),
    ]
}

fn fault_for(lane: Lane, id: u32) -> BoxedStrategy<Fault> {
    let target = prop_oneof![
        Just(FaultTarget::Lane(lane)),
        (endpoint(), endpoint()).prop_map(move |(source, destination)| FaultTarget::Route {
            source,
            destination,
            lane,
        }),
        (0u64..20).prop_map(|raw| FaultTarget::Event(EventId::new(raw))),
    ];
    let action = match lane {
        Lane::Control => control_action().boxed(),
        Lane::Asset => asset_action().boxed(),
    };
    (target, action)
        .prop_map(move |(target, action)| Fault::new(FaultId::new(id), target, action))
        .boxed()
}

fn operation(index: u32) -> BoxedStrategy<Operation> {
    prop_oneof![
        6 => control_request().prop_map(Operation::ScheduleControl),
        6 => asset_request().prop_map(Operation::ScheduleAsset),
        4 => lane().prop_flat_map(move |side| fault_for(side, index)).prop_map(Operation::AddFault),
        3 => (0u64..6).prop_map(Operation::AdvanceBy),
        2 => tick().prop_map(Operation::AdvanceTo),
        3 => Just(Operation::DeliverNext),
        3 => Just(Operation::DeliverReady),
        2 => tick().prop_map(Operation::RunUntil),
        2 => endpoint().prop_map(Operation::HaltEndpoint),
        2 => endpoint().prop_map(Operation::ResumeEndpoint),
        1 => lane().prop_map(Operation::PauseLane),
        1 => lane().prop_map(Operation::ResumeLane),
        1 => (0u64..20, 0u64..20).prop_map(|(left, right)| Operation::SwapDeliveryTicks {
            left: EventId::new(left),
            right: EventId::new(right),
        }),
        1 => (0u64..20, 0u64..20).prop_map(|(event, other)| Operation::MoveBefore {
            event: EventId::new(event),
            other: EventId::new(other),
        }),
        1 => (0u64..20, 0u64..20).prop_map(|(event, other)| Operation::MoveAfter {
            event: EventId::new(event),
            other: EventId::new(other),
        }),
    ]
    .boxed()
}

fn operations() -> impl Strategy<Value = Vec<Operation>> {
    (1usize..30)
        .prop_flat_map(|count| {
            let steps: Vec<BoxedStrategy<Operation>> = (0..count)
                .map(|index| operation(u32::try_from(index).unwrap_or(0)))
                .collect();
            steps
        })
        .prop_map(|mut steps| {
            steps.push(Operation::ResumeLane(Lane::Control));
            steps.push(Operation::ResumeLane(Lane::Asset));
            steps.push(Operation::RunUntilIdle);
            steps
        })
}

fn run(steps: &[Operation]) -> Simulator {
    let mut sim = simulator();
    let _ = sim.apply_best_effort(steps);
    sim
}

/// Delivered records grouped by the event that produced them.
fn delivered_events(sim: &Simulator) -> BTreeSet<EventId> {
    sim.endpoints()
        .flat_map(|endpoint| {
            endpoint
                .control_inbox()
                .iter()
                .map(|entry| entry.event)
                .chain(endpoint.asset_inbox().iter().map(|entry| entry.event))
        })
        .collect()
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn no_sampled_operation_stream_makes_the_simulator_give_up(steps in operations()) {
        let sim = run(&steps);
        prop_assert_eq!(sim.event_count(), sim.events().count());
        prop_assert!(sim.trace().indices_are_contiguous());
    }

    #[test]
    fn a_refused_step_leaves_the_state_exactly_as_it_was(steps in operations()) {
        let mut sim = simulator();
        for step in &steps {
            let before = sim.state_hash();
            let trace_length = sim.trace().len();
            if sim.apply(step).is_err() {
                prop_assert_eq!(sim.state_hash(), before, "{} changed state", step);
                prop_assert_eq!(sim.trace().len(), trace_length);
            }
        }
    }

    #[test]
    fn the_same_stream_reaches_the_same_state(steps in operations()) {
        let left = run(&steps);
        let right = run(&steps);
        prop_assert_eq!(left.state_hash(), right.state_hash());
        prop_assert_eq!(left.trace(), right.trace());
        prop_assert_eq!(left.queue(), right.queue());
        prop_assert_eq!(left.held(), right.held());
    }

    #[test]
    fn a_snapshot_replays_to_the_same_state(steps in operations(), cut in 0usize..8) {
        let split = cut.min(steps.len());
        let (head, tail) = steps.split_at(split);
        let mut sim = simulator();
        let _ = sim.apply_best_effort(head);
        let snapshot = sim.snapshot();

        let mut left = snapshot.restore();
        let _ = left.apply_best_effort(tail);
        let mut right = snapshot.restore();
        let _ = right.apply_best_effort(tail);

        prop_assert_eq!(left.state_hash(), right.state_hash());
        prop_assert_eq!(snapshot.state_hash(), snapshot.restore().state_hash());
    }

    #[test]
    fn every_delivered_event_is_one_the_simulator_knows(steps in operations()) {
        let sim = run(&steps);
        for event in delivered_events(&sim) {
            prop_assert!(sim.event(event).is_some(), "{event} was never scheduled");
        }
    }

    #[test]
    fn a_dropped_event_never_reaches_an_inbox(steps in operations()) {
        let sim = run(&steps);
        let delivered = delivered_events(&sim);
        for event in sim.events() {
            if event.status() == EventStatus::Dropped {
                prop_assert!(!delivered.contains(&event.id()));
            }
        }
    }

    #[test]
    fn a_copy_carries_the_same_content_as_its_source(steps in operations()) {
        let sim = run(&steps);
        for endpoint in sim.endpoints() {
            for entry in endpoint.control_inbox() {
                let Some(origin) = entry.duplicate_of else { continue };
                let Some(source) = sim.event(origin).and_then(Event::control) else { continue };
                prop_assert_eq!(entry.source, source.source);
                prop_assert_eq!(entry.intended_destination, source.intended_destination);
                // A copy is exact when it was made, then it travels on its own.
                if entry.mutation == source.mutation {
                    prop_assert_eq!(&entry.bytes, &source.bytes);
                    prop_assert_eq!(entry.message_id, source.message_id);
                }
            }
            for entry in endpoint.asset_inbox() {
                let Some(origin) = entry.duplicate_of else { continue };
                let Some(source) = sim.event(origin).and_then(Event::asset) else { continue };
                prop_assert_eq!(entry.transfer, source.transfer);
                prop_assert_eq!(entry.requested, source.requested);
                prop_assert_eq!(entry.intended_destination, source.intended_destination);
            }
        }
    }

    #[test]
    fn a_fault_batch_is_decided_once_per_event(steps in operations()) {
        let sim = run(&steps);
        let mut attempts: BTreeMap<EventId, u32> = BTreeMap::new();
        for record in sim.trace().records() {
            match record.action {
                TraceAction::DeliveryAttempted { event, .. } => {
                    *attempts.entry(event).or_default() += 1;
                }
                TraceAction::FaultApplied { event, .. }
                | TraceAction::FaultRejected { event, .. } => {
                    prop_assert_eq!(
                        attempts.get(&event).copied(),
                        Some(1),
                        "{} took fault work outside its first attempt",
                        event
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn a_corrupted_message_never_matches_its_source_byte(steps in operations()) {
        let sim = run(&steps);
        for endpoint in sim.endpoints() {
            for entry in endpoint.control_inbox() {
                let Some(mutation) = entry.mutation else { continue };
                prop_assert_ne!(mutation.from, mutation.to);
                prop_assert_ne!(mutation.original_message_id, entry.message_id);
                prop_assert_eq!(entry.bytes.get(mutation.offset), Some(&mutation.to));
            }
        }
    }

    #[test]
    fn a_delivery_only_passes_the_request_under_an_over_delivery_fault(steps in operations()) {
        let sim = run(&steps);
        for endpoint in sim.endpoints() {
            for entry in endpoint.asset_inbox() {
                prop_assert!(!entry.over_delivered, "no sampled fault over delivers");
                prop_assert!(entry.delivered <= entry.requested);
                prop_assert!(!entry.delivered.is_zero());
            }
        }
    }

    #[test]
    fn split_pieces_never_add_up_past_the_event_they_came_from(steps in operations()) {
        let sim = run(&steps);
        let mut extra: BTreeMap<EventId, u128> = BTreeMap::new();
        for record in sim.trace().records() {
            if let TraceAction::PartialDeliveryCreated { original, amount, .. } = record.action {
                *extra.entry(original).or_default() += amount.get();
            }
        }
        for (origin, pieces) in extra {
            let Some(asset) = sim.event(origin).and_then(Event::asset) else { continue };
            let total = pieces.saturating_add(asset.delivered.get());
            prop_assert!(
                total <= asset.requested.get(),
                "pieces {} above request {}",
                total,
                asset.requested.get()
            );
        }
    }

    #[test]
    fn the_clock_only_moves_forward(steps in operations()) {
        let sim = run(&steps);
        let mut last = Tick::ZERO;
        for record in sim.trace().records() {
            prop_assert!(record.tick >= last);
            last = record.tick;
        }
        prop_assert_eq!(sim.now(), last.max(sim.now()));
    }

    #[test]
    fn a_finished_event_is_never_attempted_again(steps in operations()) {
        let sim = run(&steps);
        let mut finished: BTreeSet<EventId> = BTreeSet::new();
        for record in sim.trace().records() {
            match record.action {
                TraceAction::DeliveryAttempted { event, .. } => {
                    prop_assert!(!finished.contains(&event), "{event} was attempted after finishing");
                }
                TraceAction::EventDelivered { event, .. }
                | TraceAction::EventDropped { event, .. }
                | TraceAction::EventExpired { event, .. }
                | TraceAction::EventRejected { event, .. } => {
                    finished.insert(event);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn a_halted_endpoint_takes_nothing_while_it_is_down(steps in operations()) {
        let sim = run(&steps);
        let mut halted: BTreeSet<EndpointId> = BTreeSet::new();
        for record in sim.trace().records() {
            match record.action {
                TraceAction::EndpointHalted { endpoint } => {
                    halted.insert(endpoint);
                }
                TraceAction::EndpointResumed { endpoint, .. } => {
                    halted.remove(&endpoint);
                }
                TraceAction::EventDelivered { destination, .. } => {
                    prop_assert!(!halted.contains(&destination), "{destination} took a delivery while halted");
                }
                _ => {}
            }
        }
    }

    #[test]
    fn a_paused_lane_completes_nothing_while_it_is_down(steps in operations()) {
        let sim = run(&steps);
        let mut paused: BTreeSet<Lane> = BTreeSet::new();
        for record in sim.trace().records() {
            match record.action {
                TraceAction::LanePaused { lane } => {
                    paused.insert(lane);
                }
                TraceAction::LaneResumed { lane, .. } => {
                    paused.remove(&lane);
                }
                TraceAction::EventDelivered { subject, .. } => {
                    let side = match subject {
                        xchain_sim::Subject::Message(_) => Lane::Control,
                        xchain_sim::Subject::Transfer(_) => Lane::Asset,
                    };
                    prop_assert!(!paused.contains(&side), "{side} delivered while paused");
                }
                _ => {}
            }
        }
    }

    #[test]
    fn trace_positions_have_no_gaps(steps in operations()) {
        let sim = run(&steps);
        prop_assert!(sim.trace().indices_are_contiguous());
    }

    #[test]
    fn event_and_fault_identifiers_stay_unique(steps in operations()) {
        let sim = run(&steps);
        let ids: Vec<EventId> = sim.events().map(Event::id).collect();
        let unique: BTreeSet<EventId> = ids.iter().copied().collect();
        prop_assert_eq!(ids.len(), unique.len());

        let faults: Vec<FaultId> = sim.plan().iter().map(|fault| fault.id).collect();
        let unique_faults: BTreeSet<FaultId> = faults.iter().copied().collect();
        prop_assert_eq!(faults.len(), unique_faults.len());
    }

    #[test]
    fn a_finished_run_leaves_no_event_waiting_on_a_clear_path(steps in operations()) {
        let sim = run(&steps);
        prop_assert!(sim.queue().is_empty(), "run until idle left work behind");
        for event in sim.events() {
            let waiting = matches!(event.status(), EventStatus::Scheduled | EventStatus::Ready);
            prop_assert!(!waiting, "{} is still waiting", event.id());
        }
    }

    #[test]
    fn every_held_event_is_blocked_and_out_of_the_queue(steps in operations()) {
        let sim = run(&steps);
        for id in sim.held() {
            let event = sim.event(*id);
            prop_assert_eq!(event.map(Event::status), Some(EventStatus::Blocked));
            prop_assert_eq!(sim.queue().find(*id), None);
        }
    }

    #[test]
    fn a_seeded_plan_is_the_same_every_time(seed in any::<u64>(), count in 1usize..12) {
        let targets: Vec<(EventId, Lane)> = (0..count)
            .map(|index| {
                let lane = if index % 2 == 0 { Lane::Control } else { Lane::Asset };
                (EventId::new(index.try_into().unwrap_or(0)), lane)
            })
            .collect();
        let first = seeded_plan(seed, &targets);
        prop_assert_eq!(&first, &seeded_plan(seed, &targets));
        prop_assert_eq!(first.len(), count);
    }

    #[test]
    fn a_seeded_plan_drives_a_repeatable_run(seed in any::<u64>()) {
        let build = || {
            let mut sim = simulator();
            let mut targets = Vec::new();
            for index in 0..4u64 {
                let control = sim
                    .schedule_control(ControlRequest::new(
                        HUB,
                        LEG,
                        canonical(index.saturating_add(1)),
                        Tick::new(index),
                    ))
                    .unwrap();
                targets.push((control, Lane::Control));
                let asset = sim
                    .schedule_asset(AssetRequest::new(
                        TransferId::new([u8::try_from(index).unwrap_or(0); 32]),
                        HUB,
                        LEG,
                        AssetAmount::new(1_000),
                        Tick::new(index),
                    ))
                    .unwrap();
                targets.push((asset, Lane::Asset));
            }
            let plan = seeded_plan(seed, &targets);
            for fault in plan.iter() {
                let _ = sim.add_fault(fault.clone());
            }
            sim.run_until_idle();
            sim
        };
        let left = build();
        let right = build();
        prop_assert_eq!(left.state_hash(), right.state_hash(), "plan {}", left.plan());
    }
}
