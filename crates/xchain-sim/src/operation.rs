//! One typed step of a run.
//!
//! A list of these is enough to rebuild a run from any snapshot, which is what
//! makes replay and branch comparison possible.

extern crate alloc;

use crate::endpoint::EndpointId;
use crate::error::SimError;
use crate::event::EventId;
use crate::fault::Fault;
use crate::lane::Lane;
use crate::simulator::{AssetRequest, ControlRequest, Simulator};
use crate::time::Tick;

/// Something a caller can ask the simulator to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    ScheduleControl(ControlRequest),
    ScheduleAsset(AssetRequest),
    AddFault(Fault),
    AdvanceBy(u64),
    AdvanceTo(Tick),
    DeliverNext,
    DeliverReady,
    RunUntilIdle,
    RunUntil(Tick),
    HaltEndpoint(EndpointId),
    ResumeEndpoint(EndpointId),
    PauseLane(Lane),
    ResumeLane(Lane),
    SwapDeliveryTicks { left: EventId, right: EventId },
    MoveBefore { event: EventId, other: EventId },
    MoveAfter { event: EventId, other: EventId },
}

impl Operation {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::ScheduleControl(_) => "schedule control",
            Self::ScheduleAsset(_) => "schedule asset",
            Self::AddFault(_) => "add fault",
            Self::AdvanceBy(_) => "advance by",
            Self::AdvanceTo(_) => "advance to",
            Self::DeliverNext => "deliver next",
            Self::DeliverReady => "deliver ready",
            Self::RunUntilIdle => "run until idle",
            Self::RunUntil(_) => "run until",
            Self::HaltEndpoint(_) => "halt endpoint",
            Self::ResumeEndpoint(_) => "resume endpoint",
            Self::PauseLane(_) => "pause lane",
            Self::ResumeLane(_) => "resume lane",
            Self::SwapDeliveryTicks { .. } => "swap ticks",
            Self::MoveBefore { .. } => "move before",
            Self::MoveAfter { .. } => "move after",
        }
    }
}

impl core::fmt::Display for Operation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.name())
    }
}

impl Simulator {
    /// Runs one step.
    ///
    /// A refused step changes nothing, because every check runs before any
    /// state is written.
    pub fn apply(&mut self, operation: &Operation) -> Result<(), SimError> {
        match operation {
            Operation::ScheduleControl(request) => {
                self.schedule_control(request.clone()).map(|_| ())
            }
            Operation::ScheduleAsset(request) => self.schedule_asset(*request).map(|_| ()),
            Operation::AddFault(fault) => self.add_fault(fault.clone()),
            Operation::AdvanceBy(ticks) => self.advance_by(*ticks),
            Operation::AdvanceTo(tick) => self.advance_to(*tick),
            Operation::DeliverNext => {
                self.deliver_next();
                Ok(())
            }
            Operation::DeliverReady => {
                self.deliver_ready();
                Ok(())
            }
            Operation::RunUntilIdle => {
                self.run_until_idle();
                Ok(())
            }
            Operation::RunUntil(tick) => self.run_until(*tick).map(|_| ()),
            Operation::HaltEndpoint(id) => self.halt_endpoint(*id),
            Operation::ResumeEndpoint(id) => self.resume_endpoint(*id),
            Operation::PauseLane(lane) => {
                self.pause_lane(*lane);
                Ok(())
            }
            Operation::ResumeLane(lane) => {
                self.resume_lane(*lane);
                Ok(())
            }
            Operation::SwapDeliveryTicks { left, right } => self.swap_delivery_ticks(*left, *right),
            Operation::MoveBefore { event, other } => self.move_before(*event, *other),
            Operation::MoveAfter { event, other } => self.move_after(*event, *other),
        }
    }

    /// Runs a list of steps, stopping at the first refusal.
    pub fn apply_all(&mut self, operations: &[Operation]) -> Result<(), SimError> {
        for operation in operations {
            self.apply(operation)?;
        }
        Ok(())
    }

    /// Runs a list of steps, keeping going past refusals.
    ///
    /// The returned list says what each step did, which keeps a random
    /// property run reproducible.
    pub fn apply_best_effort(
        &mut self,
        operations: &[Operation],
    ) -> alloc::vec::Vec<Result<(), SimError>> {
        operations
            .iter()
            .map(|operation| self.apply(operation))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;

    use protocol_types::{AssetAmount, TransferId};

    use super::*;
    use crate::fault::{FaultAction, FaultId, FaultTarget};

    fn every_operation() -> Vec<Operation> {
        alloc::vec![
            Operation::ScheduleControl(ControlRequest::new(
                EndpointId::new(1),
                EndpointId::new(2),
                alloc::vec![1],
                Tick::ZERO,
            )),
            Operation::ScheduleAsset(AssetRequest::new(
                TransferId::ZERO,
                EndpointId::new(1),
                EndpointId::new(2),
                AssetAmount::new(1),
                Tick::ZERO,
            )),
            Operation::AddFault(Fault::new(
                FaultId::new(1),
                FaultTarget::Lane(Lane::Control),
                FaultAction::Drop,
            )),
            Operation::AdvanceBy(1),
            Operation::AdvanceTo(Tick::new(1)),
            Operation::DeliverNext,
            Operation::DeliverReady,
            Operation::RunUntilIdle,
            Operation::RunUntil(Tick::new(1)),
            Operation::HaltEndpoint(EndpointId::new(1)),
            Operation::ResumeEndpoint(EndpointId::new(1)),
            Operation::PauseLane(Lane::Control),
            Operation::ResumeLane(Lane::Control),
            Operation::SwapDeliveryTicks {
                left: EventId::new(1),
                right: EventId::new(2),
            },
            Operation::MoveBefore {
                event: EventId::new(1),
                other: EventId::new(2),
            },
            Operation::MoveAfter {
                event: EventId::new(1),
                other: EventId::new(2),
            },
        ]
    }

    #[test]
    fn every_operation_has_its_own_name() {
        let mut seen: Vec<String> = Vec::new();
        for operation in every_operation() {
            let name = operation.to_string();
            assert!(!seen.contains(&name), "repeated name {name}");
            seen.push(name);
        }
        assert_eq!(seen.len(), 16);
    }
}
