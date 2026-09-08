//! Deterministic transport-fault campaign against live Solana and exact EVM bytes.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

use solevm_e2e::ops::WireOp;
use solevm_e2e::world::{Observation, World};

const ALLOCATE_HEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../contracts/solevm-vault/testdata/control-lane/evm-allocate.hex"
));
const AMOUNT: u64 = 1_000_000;
const ALLOCATE_ID: &str = "0x7777777777777777777777777777777777777777777777777777777777777777";
const CASES: u64 = 256;

fn allocate_bytes() -> String {
    ALLOCATE_HEX.trim().to_string()
}

fn run_case(seed: u64) -> Observation {
    let mut world = World::new();
    world
        .apply(&WireOp::ScheduleControl {
            bytes: allocate_bytes(),
            at: 1,
        })
        .expect("control schedules");
    let control = world.last_event_id();
    world
        .apply(&WireOp::ScheduleAsset {
            transfer: String::from(ALLOCATE_ID),
            amount: AMOUNT,
            at: 1,
        })
        .expect("asset schedules");
    let asset = world.last_event_id();
    apply_faults(&mut world, seed, control, asset);
    world.apply(&WireOp::RunUntilIdle).expect("transport runs");
    world.apply(&WireOp::ApplyControl).expect("control applies");
    world.apply(&WireOp::ApplyAsset).expect("assets apply");
    let _ = world.apply(&WireOp::Attribute {
        transfer: String::from(ALLOCATE_ID),
    });
    world.observe()
}

fn apply_faults(world: &mut World, seed: u64, control: u64, asset: u64) {
    match seed % 8 {
        0 => world
            .apply(&WireOp::FaultDelay {
                event: control,
                ticks: (seed % 5) + 1,
            })
            .expect("delay"),
        1 => world
            .apply(&WireOp::FaultDrop { event: asset })
            .expect("drop"),
        2 => world
            .apply(&WireOp::FaultDuplicate {
                event: control,
                copies: 1,
                spacing: 0,
            })
            .expect("duplicate"),
        3 => world
            .apply(&WireOp::FaultPartial {
                event: asset,
                amount: 400_000,
            })
            .expect("partial"),
        4 => world
            .apply(&WireOp::FaultSplit {
                event: asset,
                pieces: vec![250_000, 250_000, 500_000],
                spacing: 1,
            })
            .expect("split"),
        5 => {
            world
                .apply(&WireOp::FaultDelay {
                    event: asset,
                    ticks: 3,
                })
                .expect("late asset");
        }
        6 => world
            .apply(&WireOp::FaultDuplicate {
                event: asset,
                copies: 1,
                spacing: 0,
            })
            .expect("duplicate asset"),
        _ => world
            .apply(&WireOp::FaultDelay {
                event: control,
                ticks: 2,
            })
            .expect("late message"),
    }
}

fn assert_invariants(seed: u64, observed: &Observation) {
    assert!(
        observed.principal <= AMOUNT,
        "seed {seed} over-attributed {}",
        observed.principal
    );
    assert!(
        observed.deployed <= observed.principal,
        "seed {seed} deployed beyond principal"
    );
    if observed.principal == 0 {
        assert_eq!(
            observed.deployed, 0,
            "seed {seed} deployed without principal"
        );
    }
}

#[test]
fn generated_faults_never_over_attribute_and_replay() {
    for seed in 1..=CASES {
        let first = run_case(seed);
        let second = run_case(seed);
        assert_eq!(first, second, "seed {seed} replay diverged");
        assert_invariants(seed, &first);
    }
}

#[test]
fn identical_scripts_share_one_trace() {
    let first = run_case(17);
    let second = run_case(17);
    assert_eq!(first.trace, second.trace);
    assert_eq!(first.lane_commitment, second.lane_commitment);
}
