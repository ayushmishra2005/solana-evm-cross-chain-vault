//! Wire operations the Foundry harness and generated tests send.

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WireOp {
    Warp {
        ts: u64,
    },
    ScheduleControl {
        bytes: String,
        at: u64,
    },
    ScheduleAsset {
        transfer: String,
        amount: u64,
        at: u64,
    },
    FaultDrop {
        event: u64,
    },
    FaultDelay {
        event: u64,
        ticks: u64,
    },
    FaultDuplicate {
        event: u64,
        copies: u16,
        spacing: u64,
    },
    FaultPartial {
        event: u64,
        amount: u64,
    },
    FaultSplit {
        event: u64,
        pieces: Vec<u64>,
        spacing: u64,
    },
    Swap {
        left: u64,
        right: u64,
    },
    AdvanceTo {
        tick: u64,
    },
    DeliverReady,
    RunUntilIdle,
    ApplyControl,
    ApplyAsset,
    Attribute {
        transfer: String,
    },
    Deploy {
        amount: u64,
    },
    ConfigureAdapter {
        max_liquid: u64,
        loss_bps: u16,
    },
    Withdraw {
        transfer: String,
        amount: u64,
    },
    SendRecall {
        transfer: String,
        amount: u64,
        at: u64,
    },
    PublishReport {
        epoch: u64,
        at: u64,
    },
    PublishRecallSent {
        transfer: String,
        at: u64,
    },
    Freeze,
    ConfigUpdate {
        bytes: String,
    },
    ConfigUpdateNext,
    ConfigUpdateWrongLane,
    ConfigUpdateReuseSequence,
    AllocateNext {
        transfer: String,
        amount: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Script {
    pub ops: Vec<WireOp>,
}
