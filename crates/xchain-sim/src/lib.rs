//! Deterministic transport simulator for the SolEVM Vault.
//!
//! Two lanes move independently: control messages carry canonical bytes, and
//! asset events carry value. Nothing links them, so a message can arrive with
//! no transfer, twice, or long after the value it describes.
//!
//! The simulator only delivers. It never decides whether a delivery is
//! authorised, priced, or worth accepting. Those calls belong to the endpoint
//! code that will run on top of it.
//!
//! ```
//! use protocol_types::{AssetAmount, TransferId};
//! use xchain_sim::{AssetRequest, EndpointId, Simulator, Tick};
//!
//! let hub = EndpointId::new(1);
//! let leg = EndpointId::new(2);
//! let mut sim = Simulator::new(&[hub, leg])?;
//! sim.schedule_asset(AssetRequest::new(
//!     TransferId::new([1u8; 32]),
//!     hub,
//!     leg,
//!     AssetAmount::new(500),
//!     Tick::new(3),
//! ))?;
//! sim.run_until_idle();
//! assert_eq!(sim.asset_inbox(leg).len(), 1);
//! # Ok::<(), xchain_sim::SimError>(())
//! ```

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::as_conversions
)]

extern crate alloc;

pub mod endpoint;
pub mod error;
pub mod event;
pub mod fault;
pub mod inspect;
pub mod lane;
pub mod operation;
pub mod queue;
pub mod simulator;
pub mod snapshot;
pub mod state_hash;
pub mod time;
pub mod trace;

pub use endpoint::{DeliveredAsset, DeliveredControl, Endpoint, EndpointId, EndpointState};
pub use error::{ConfigProblem, SimError};
pub use event::{AssetEvent, ByteMutation, ControlEvent, Event, EventId, EventStatus};
pub use fault::{
    ExclusionGroup, Fault, FaultAction, FaultId, FaultPlan, FaultStage, FaultTarget, PlanSeed,
    seeded_plan,
};
pub use lane::{Lane, LaneState};
pub use operation::Operation;
pub use queue::{EventQueue, QueueKey};
pub use simulator::{AssetRequest, ControlRequest, LatePolicy, Simulator, SimulatorConfig};
pub use snapshot::Snapshot;
pub use state_hash::{StateHash, state_hash};
pub use time::Tick;
pub use trace::{
    BlockReason, FaultEffect, RejectReason, Subject, Trace, TraceAction, TraceIndex, TraceRecord,
};
