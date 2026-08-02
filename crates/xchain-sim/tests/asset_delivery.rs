//! Asset lane delivery behaviour, and its independence from control messages.

#![allow(clippy::unwrap_used)]

mod common;

use common::{
    HUB, LEG, WATCHER, amount, asset_at, canonical, control_at, fault, simulator, simulator_with,
    transfer,
};
use xchain_sim::{
    AssetRequest, ControlRequest, EventStatus, FaultAction, FaultTarget, Lane, LatePolicy,
    SimError, SimulatorConfig, Tick,
};

#[test]
fn a_transfer_arrives_whole_at_its_tick() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(3, 0x11, 1_000)).unwrap();
    sim.run_until_idle();

    let inbox = sim.asset_inbox(LEG);
    assert_eq!(inbox.len(), 1);
    let delivered = inbox.first().unwrap();
    assert_eq!(delivered.event, event);
    assert_eq!(delivered.requested, amount(1_000));
    assert_eq!(delivered.delivered, amount(1_000));
    assert_eq!(delivered.delivered_at, Tick::new(3));
    assert_eq!(delivered.piece, None);
    assert!(!delivered.over_delivered);
    assert_eq!(sim.delivered_for_transfer(transfer(0x11)), 1_000);
}

#[test]
fn a_partial_fault_delivers_less_than_the_request() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x11, 1_000)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Partial {
            amount: amount(400),
        },
    ))
    .unwrap();
    sim.run_until_idle();

    let delivered = sim.asset_inbox(LEG).first().unwrap();
    assert_eq!(delivered.requested, amount(1_000));
    assert_eq!(delivered.delivered, amount(400));
    assert_eq!(sim.delivered_for_transfer(transfer(0x11)), 400);
}

#[test]
fn a_split_spreads_one_transfer_over_several_deliveries() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x22, 900)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Split {
            pieces: vec![amount(500), amount(300), amount(100)],
            spacing: 2,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    let inbox = sim.asset_inbox(LEG);
    assert_eq!(inbox.len(), 3);
    let amounts: Vec<u128> = inbox.iter().map(|entry| entry.delivered.get()).collect();
    assert_eq!(amounts, vec![500, 300, 100]);
    let pieces: Vec<Option<u16>> = inbox.iter().map(|entry| entry.piece).collect();
    assert_eq!(pieces, vec![Some(0), Some(1), Some(2)]);
    let ticks: Vec<u64> = inbox.iter().map(|entry| entry.delivered_at.get()).collect();
    assert_eq!(ticks, vec![1, 3, 5]);
    assert_eq!(sim.delivered_for_transfer(transfer(0x22)), 900);
}

#[test]
fn every_piece_of_a_split_keeps_the_same_transfer_and_its_own_event_id() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x22, 900)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Split {
            pieces: vec![amount(600), amount(300)],
            spacing: 1,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    let inbox = sim.asset_inbox(LEG);
    assert!(inbox.iter().all(|entry| entry.transfer == transfer(0x22)));
    let mut ids: Vec<_> = inbox.iter().map(|entry| entry.event).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 2);
}

#[test]
fn split_pieces_never_add_up_to_more_than_the_request() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x22, 500)).unwrap();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Split {
            pieces: vec![amount(400), amount(300)],
            spacing: 1,
        },
    ));
    assert!(matches!(
        outcome,
        Err(SimError::PartialSumExceedsRequest {
            requested: 500,
            offered: 700
        })
    ));
}

#[test]
fn a_split_may_hold_back_part_of_the_request() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x22, 1_000)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Split {
            pieces: vec![amount(300), amount(200)],
            spacing: 0,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    assert_eq!(sim.delivered_for_transfer(transfer(0x22)), 500);
    assert!(
        sim.asset_inbox(LEG)
            .iter()
            .all(|entry| entry.requested == amount(1_000))
    );
}

#[test]
fn a_repeated_transfer_attempt_stays_visible() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x33, 250)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Duplicate {
            copies: 1,
            spacing: 3,
        },
    ))
    .unwrap();
    sim.run_until_idle();

    let inbox = sim.asset_inbox(LEG);
    assert_eq!(inbox.len(), 2);
    let copy = inbox.get(1).unwrap();
    assert_eq!(copy.duplicate_of, Some(event));
    assert_eq!(copy.delivered, amount(250));
    assert_eq!(copy.transfer, transfer(0x33));
    assert_eq!(sim.delivered_for_transfer(transfer(0x33)), 500);
}

#[test]
fn a_dropped_transfer_never_reaches_an_inbox() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x33, 250)).unwrap();
    sim.add_fault(fault(1, FaultTarget::Event(event), FaultAction::Drop))
        .unwrap();
    sim.run_until_idle();

    assert!(sim.asset_inbox(LEG).is_empty());
    assert_eq!(sim.delivered_for_transfer(transfer(0x33)), 0);
}

#[test]
fn a_delay_moves_the_transfer_to_a_later_tick() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(2, 0x33, 250)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Delay { ticks: 5 },
    ))
    .unwrap();
    sim.run_until_idle();

    assert_eq!(
        sim.asset_inbox(LEG).first().unwrap().delivered_at,
        Tick::new(7)
    );
}

#[test]
fn a_rerouted_transfer_shows_the_destination_the_sender_chose() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x44, 700)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::ReplaceDestination(WATCHER),
    ))
    .unwrap();
    sim.run_until_idle();

    assert!(sim.asset_inbox(LEG).is_empty());
    let delivered = sim.asset_inbox(WATCHER).first().unwrap();
    assert_eq!(delivered.intended_destination, LEG);
    assert!(delivered.is_misrouted(WATCHER));
    assert_eq!(sim.delivered_for_transfer(transfer(0x44)), 700);
}

#[test]
fn a_halted_destination_holds_the_transfer() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x44, 700)).unwrap();
    sim.halt_endpoint(LEG).unwrap();
    sim.run_until_idle();

    assert!(sim.asset_inbox(LEG).is_empty());
    assert_eq!(
        sim.event(event).map(xchain_sim::Event::status),
        Some(EventStatus::Blocked)
    );
}

#[test]
fn resuming_an_endpoint_lets_a_held_transfer_through() {
    let mut sim = simulator();
    sim.schedule_asset(asset_at(1, 0x44, 700)).unwrap();
    sim.halt_endpoint(LEG).unwrap();
    sim.run_until_idle();
    sim.advance_to(Tick::new(8)).unwrap();
    sim.resume_endpoint(LEG).unwrap();
    sim.run_until_idle();

    assert_eq!(
        sim.asset_inbox(LEG).first().unwrap().delivered_at,
        Tick::new(8)
    );
}

#[test]
fn pausing_the_asset_lane_stops_every_transfer() {
    let mut sim = simulator();
    sim.schedule_asset(asset_at(1, 0x44, 700)).unwrap();
    sim.schedule_asset(AssetRequest::new(
        transfer(0x55),
        HUB,
        WATCHER,
        amount(10),
        Tick::new(1),
    ))
    .unwrap();
    sim.pause_lane(Lane::Asset);
    sim.run_until_idle();

    assert!(sim.asset_inbox(LEG).is_empty());
    assert!(sim.asset_inbox(WATCHER).is_empty());
    assert_eq!(sim.held().len(), 2);

    sim.resume_lane(Lane::Asset);
    sim.run_until_idle();
    assert_eq!(sim.asset_inbox(LEG).len(), 1);
    assert_eq!(sim.asset_inbox(WATCHER).len(), 1);
}

#[test]
fn pausing_one_lane_leaves_the_other_running() {
    let mut sim = simulator();
    sim.schedule_control(control_at(1, 1)).unwrap();
    sim.schedule_asset(asset_at(1, 0x44, 700)).unwrap();
    sim.pause_lane(Lane::Asset);
    sim.run_until_idle();

    assert_eq!(sim.control_inbox(LEG).len(), 1);
    assert!(sim.asset_inbox(LEG).is_empty());
}

#[test]
fn value_may_arrive_before_the_message_that_describes_it() {
    let mut sim = simulator();
    sim.schedule_asset(asset_at(1, 0x11, 1_000)).unwrap();
    sim.schedule_control(control_at(5, 1)).unwrap();
    sim.run_until_idle();

    assert_eq!(
        sim.asset_inbox(LEG).first().unwrap().delivered_at,
        Tick::new(1)
    );
    assert_eq!(
        sim.control_inbox(LEG).first().unwrap().delivered_at,
        Tick::new(5)
    );
}

#[test]
fn a_message_may_arrive_before_the_value_it_describes() {
    let mut sim = simulator();
    sim.schedule_control(control_at(1, 1)).unwrap();
    sim.schedule_asset(asset_at(5, 0x11, 1_000)).unwrap();
    sim.run_until_idle();

    assert_eq!(
        sim.control_inbox(LEG).first().unwrap().delivered_at,
        Tick::new(1)
    );
    assert_eq!(
        sim.asset_inbox(LEG).first().unwrap().delivered_at,
        Tick::new(5)
    );
}

#[test]
fn value_may_arrive_with_no_message_at_all() {
    let mut sim = simulator();
    sim.schedule_asset(asset_at(1, 0x11, 1_000)).unwrap();
    sim.run_until_idle();

    assert_eq!(sim.asset_inbox(LEG).len(), 1);
    assert!(sim.control_inbox(LEG).is_empty());
}

#[test]
fn a_message_may_arrive_with_no_value_at_all() {
    let mut sim = simulator();
    sim.schedule_control(control_at(1, 1)).unwrap();
    sim.run_until_idle();

    assert_eq!(sim.control_inbox(LEG).len(), 1);
    assert!(sim.asset_inbox(LEG).is_empty());
}

#[test]
fn dropping_a_message_leaves_its_value_untouched() {
    let mut sim = simulator();
    let message = sim.schedule_control(control_at(1, 1)).unwrap();
    sim.schedule_asset(asset_at(1, 0x11, 1_000)).unwrap();
    sim.add_fault(fault(1, FaultTarget::Event(message), FaultAction::Drop))
        .unwrap();
    sim.run_until_idle();

    assert!(sim.control_inbox(LEG).is_empty());
    assert_eq!(sim.delivered_for_transfer(transfer(0x11)), 1_000);
}

#[test]
fn a_transfer_past_its_timeout_is_marked_late_under_the_default_policy() {
    let mut sim = simulator();
    sim.schedule_asset(
        AssetRequest::new(transfer(0x66), HUB, LEG, amount(80), Tick::new(9))
            .timing_out_at(Tick::new(4)),
    )
    .unwrap();
    sim.run_until_idle();

    let delivered = sim.asset_inbox(LEG).first().unwrap();
    assert!(delivered.after_timeout);
    assert_eq!(delivered.delivered, amount(80));
}

#[test]
fn a_transfer_past_its_timeout_is_stopped_under_the_expiring_policy() {
    let mut sim = simulator_with(SimulatorConfig {
        control_late_policy: LatePolicy::DeliverWithMarker,
        asset_late_policy: LatePolicy::Expire,
    });
    let event = sim
        .schedule_asset(
            AssetRequest::new(transfer(0x66), HUB, LEG, amount(80), Tick::new(9))
                .timing_out_at(Tick::new(4)),
        )
        .unwrap();
    sim.run_until_idle();

    assert!(sim.asset_inbox(LEG).is_empty());
    assert_eq!(
        sim.event(event).map(xchain_sim::Event::status),
        Some(EventStatus::Expired)
    );
}

#[test]
fn ordinary_scheduling_never_delivers_more_than_the_request() {
    let mut sim = simulator();
    sim.schedule_asset(asset_at(1, 0x77, 100)).unwrap();
    sim.run_until_idle();

    assert_eq!(sim.delivered_for_transfer(transfer(0x77)), 100);
    assert!(
        sim.asset_inbox(LEG)
            .iter()
            .all(|entry| entry.delivered <= entry.requested)
    );
}

#[test]
fn a_partial_amount_above_the_request_is_refused() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x77, 100)).unwrap();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Partial {
            amount: amount(101),
        },
    ));
    assert!(matches!(
        outcome,
        Err(SimError::PartialSumExceedsRequest {
            requested: 100,
            offered: 101
        })
    ));
}

#[test]
fn the_over_delivery_fault_is_the_only_way_past_the_request() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x77, 100)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::OverDeliver {
            amount: amount(150),
        },
    ))
    .unwrap();
    sim.run_until_idle();

    let delivered = sim.asset_inbox(LEG).first().unwrap();
    assert!(delivered.over_delivered);
    assert_eq!(delivered.delivered, amount(150));
    assert_eq!(delivered.requested, amount(100));
    assert_eq!(sim.delivered_for_transfer(transfer(0x77)), 150);
}

#[test]
fn an_over_delivery_fault_that_does_not_exceed_the_request_is_refused_at_delivery() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x77, 100)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::OverDeliver {
            amount: amount(100),
        },
    ))
    .unwrap();
    sim.run_until_idle();

    assert!(sim.asset_inbox(LEG).is_empty());
    assert_eq!(
        sim.event(event).map(xchain_sim::Event::status),
        Some(EventStatus::RejectedBySimulator)
    );
}

#[test]
fn a_transfer_of_nothing_is_refused() {
    let mut sim = simulator();
    let outcome = sim.schedule_asset(asset_at(1, 0x77, 0));
    assert!(matches!(outcome, Err(SimError::ZeroAssetAmount)));
    assert_eq!(sim.event_count(), 0);
}

#[test]
fn a_partial_fault_of_nothing_is_refused() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x77, 100)).unwrap();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Partial { amount: amount(0) },
    ));
    assert!(matches!(outcome, Err(SimError::ZeroAssetAmount)));
}

#[test]
fn a_split_with_a_zero_piece_is_refused() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x77, 100)).unwrap();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Split {
            pieces: vec![amount(50), amount(0)],
            spacing: 1,
        },
    ));
    assert!(matches!(outcome, Err(SimError::InvalidPartialSplit)));
}

#[test]
fn a_split_with_no_pieces_is_refused() {
    let mut sim = simulator();
    let event = sim.schedule_asset(asset_at(1, 0x77, 100)).unwrap();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Event(event),
        FaultAction::Split {
            pieces: Vec::new(),
            spacing: 1,
        },
    ));
    assert!(matches!(outcome, Err(SimError::InvalidPartialSplit)));
}

#[test]
fn a_control_only_fault_cannot_target_the_asset_lane() {
    let mut sim = simulator();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Lane(Lane::Asset),
        FaultAction::CorruptByte { offset: 0, mask: 1 },
    ));
    assert!(matches!(outcome, Err(SimError::InvalidConfiguration(_))));
}

#[test]
fn an_asset_only_fault_cannot_target_the_control_lane() {
    let mut sim = simulator();
    let outcome = sim.add_fault(fault(
        1,
        FaultTarget::Lane(Lane::Control),
        FaultAction::Partial { amount: amount(1) },
    ));
    assert!(matches!(outcome, Err(SimError::InvalidConfiguration(_))));
}

#[test]
fn a_transfer_fault_reaches_every_event_of_that_transfer() {
    let mut sim = simulator();
    sim.schedule_asset(asset_at(1, 0x88, 100)).unwrap();
    sim.schedule_asset(asset_at(2, 0x88, 200)).unwrap();
    sim.schedule_asset(asset_at(3, 0x99, 300)).unwrap();
    sim.add_fault(fault(
        1,
        FaultTarget::Transfer(transfer(0x88)),
        FaultAction::Drop,
    ))
    .unwrap();
    sim.run_until_idle();

    assert_eq!(sim.delivered_for_transfer(transfer(0x88)), 0);
    assert_eq!(sim.delivered_for_transfer(transfer(0x99)), 300);
}

#[test]
fn a_control_message_and_a_transfer_at_one_tick_keep_a_fixed_order() {
    let mut sim = simulator();
    sim.schedule_asset(asset_at(2, 0x11, 5)).unwrap();
    sim.schedule_control(
        ControlRequest::new(HUB, LEG, canonical(1), Tick::new(2))
            .with_id(xchain_sim::EventId::new(50)),
    )
    .unwrap();
    sim.run_until_idle();

    let control_index = sim
        .trace()
        .records()
        .iter()
        .position(|record| {
            matches!(
                record.action,
                xchain_sim::TraceAction::EventDelivered {
                    subject: xchain_sim::Subject::Message(_),
                    ..
                }
            )
        })
        .unwrap();
    let asset_index = sim
        .trace()
        .records()
        .iter()
        .position(|record| {
            matches!(
                record.action,
                xchain_sim::TraceAction::EventDelivered {
                    subject: xchain_sim::Subject::Transfer(_),
                    ..
                }
            )
        })
        .unwrap();
    assert!(
        control_index < asset_index,
        "control should sort first at a shared tick"
    );
}
