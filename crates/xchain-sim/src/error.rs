//! Typed failures the simulator can report.

use protocol_types::DecodeError;

use crate::endpoint::EndpointId;
use crate::event::EventId;
use crate::fault::FaultId;
use crate::lane::Lane;
use crate::time::Tick;

/// Everything a caller can do wrong, plus the limits the simulator enforces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SimError {
    DuplicateEndpoint(EndpointId),
    UnknownEndpoint(EndpointId),
    DuplicateEventId(EventId),
    UnknownEvent(EventId),
    DuplicateFaultId(FaultId),
    UnknownFaultTarget(FaultId),
    TimeMovesBackwards {
        now: Tick,
        requested: Tick,
    },
    DeliveryTickInPast {
        now: Tick,
        requested: Tick,
    },
    ZeroAssetAmount,
    InvalidPartialSplit,
    PartialSumExceedsRequest {
        requested: u128,
        offered: u128,
    },
    ConflictingFaults {
        first: FaultId,
        second: FaultId,
    },
    EndpointHalted(EndpointId),
    LanePaused(Lane),
    Decode(DecodeError),
    ArithmeticOverflow,
    InvalidConfiguration(ConfigProblem),
    EventAlreadyTerminal(EventId),
    /// The event already had its faults decided on an earlier attempt.
    FaultsAlreadyBound(EventId),
}

/// Why a fault or a schedule request does not describe a usable setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigProblem {
    EmptyMessage,
    ActionLaneMismatch,
    SameEndpointRoute,
    NoDuplicateCopies,
    NoEndpoints,
    CorruptOffsetOutOfRange,
    NoByteChange,
}

impl ConfigProblem {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::EmptyMessage => "message has no bytes",
            Self::ActionLaneMismatch => "fault action does not match the target lane",
            Self::SameEndpointRoute => "source and destination are the same endpoint",
            Self::NoDuplicateCopies => "duplicate fault asks for zero copies",
            Self::NoEndpoints => "the simulator has no endpoints",
            Self::CorruptOffsetOutOfRange => "corrupt offset is past the end of the message",
            Self::NoByteChange => "corrupt mask would leave the byte unchanged",
        }
    }
}

impl core::fmt::Display for ConfigProblem {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.name())
    }
}

impl core::fmt::Display for SimError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DuplicateEndpoint(id) => write!(formatter, "endpoint {id} already exists"),
            Self::UnknownEndpoint(id) => write!(formatter, "endpoint {id} is not registered"),
            Self::DuplicateEventId(id) => write!(formatter, "event {id} already exists"),
            Self::UnknownEvent(id) => write!(formatter, "event {id} is not known"),
            Self::DuplicateFaultId(id) => write!(formatter, "fault {id} already exists"),
            Self::UnknownFaultTarget(id) => write!(formatter, "fault {id} targets nothing known"),
            Self::TimeMovesBackwards { now, requested } => {
                write!(formatter, "cannot move from {now} back to {requested}")
            }
            Self::DeliveryTickInPast { now, requested } => {
                write!(formatter, "delivery at {requested} is before {now}")
            }
            Self::ZeroAssetAmount => formatter.write_str("asset amount must be above zero"),
            Self::InvalidPartialSplit => formatter.write_str("split pieces are not usable"),
            Self::PartialSumExceedsRequest { requested, offered } => write!(
                formatter,
                "pieces total {offered} above the requested {requested}"
            ),
            Self::ConflictingFaults { first, second } => {
                write!(formatter, "faults {first} and {second} cannot combine")
            }
            Self::EndpointHalted(id) => write!(formatter, "endpoint {id} is halted"),
            Self::LanePaused(lane) => write!(formatter, "the {lane} lane is paused"),
            Self::Decode(inner) => write!(formatter, "decode failed: {inner}"),
            Self::ArithmeticOverflow => formatter.write_str("a value went past its limit"),
            Self::InvalidConfiguration(problem) => write!(formatter, "invalid setup: {problem}"),
            Self::EventAlreadyTerminal(id) => write!(formatter, "event {id} is already finished"),
            Self::FaultsAlreadyBound(id) => {
                write!(formatter, "event {id} already decided its faults")
            }
        }
    }
}

impl core::error::Error for SimError {}

impl From<DecodeError> for SimError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::string::{String, ToString};
    use alloc::vec::Vec;

    use super::*;

    const PROBLEMS: [ConfigProblem; 7] = [
        ConfigProblem::EmptyMessage,
        ConfigProblem::ActionLaneMismatch,
        ConfigProblem::SameEndpointRoute,
        ConfigProblem::NoDuplicateCopies,
        ConfigProblem::NoEndpoints,
        ConfigProblem::CorruptOffsetOutOfRange,
        ConfigProblem::NoByteChange,
    ];

    fn errors() -> Vec<SimError> {
        let endpoint = EndpointId::new(1);
        let event = EventId::new(2);
        let fault = FaultId::new(3);
        alloc::vec![
            SimError::DuplicateEndpoint(endpoint),
            SimError::UnknownEndpoint(endpoint),
            SimError::DuplicateEventId(event),
            SimError::UnknownEvent(event),
            SimError::DuplicateFaultId(fault),
            SimError::UnknownFaultTarget(fault),
            SimError::TimeMovesBackwards {
                now: Tick::new(5),
                requested: Tick::new(4),
            },
            SimError::DeliveryTickInPast {
                now: Tick::new(5),
                requested: Tick::new(4),
            },
            SimError::ZeroAssetAmount,
            SimError::InvalidPartialSplit,
            SimError::PartialSumExceedsRequest {
                requested: 10,
                offered: 11,
            },
            SimError::ConflictingFaults {
                first: fault,
                second: FaultId::new(4),
            },
            SimError::EndpointHalted(endpoint),
            SimError::LanePaused(Lane::Control),
            SimError::Decode(DecodeError::InvalidMagic),
            SimError::ArithmeticOverflow,
            SimError::InvalidConfiguration(ConfigProblem::EmptyMessage),
            SimError::EventAlreadyTerminal(event),
            SimError::FaultsAlreadyBound(event),
        ]
    }

    #[test]
    fn every_error_prints_a_distinct_message() {
        let mut seen: Vec<String> = Vec::new();
        for error in errors() {
            let text = error.to_string();
            assert!(!text.is_empty());
            assert!(!seen.contains(&text), "duplicate text {text}");
            seen.push(text);
        }
    }

    #[test]
    fn every_configuration_problem_has_a_distinct_name() {
        let mut seen: Vec<&str> = Vec::new();
        for problem in PROBLEMS {
            assert!(!seen.contains(&problem.name()));
            assert_eq!(problem.to_string(), problem.name());
            seen.push(problem.name());
        }
    }

    #[test]
    fn a_decode_failure_converts_into_a_simulator_error() {
        let error: SimError = DecodeError::BodyHashMismatch.into();
        assert_eq!(error, SimError::Decode(DecodeError::BodyHashMismatch));
    }
}
