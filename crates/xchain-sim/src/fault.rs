//! Declared transport faults.
//!
//! A plan is fixed before it runs. Nothing here draws random numbers while the
//! simulator is delivering, so the same plan always bends the same events the
//! same way.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use protocol_types::{AssetAmount, MessageId, TransferId};

use crate::endpoint::EndpointId;
use crate::event::{Event, EventId};
use crate::lane::Lane;

/// Names one declared fault.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaultId(u32);

impl FaultId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for FaultId {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl core::fmt::Display for FaultId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "f{}", self.0)
    }
}

/// Which events a fault bends.
///
/// Every target names a lane, either directly or through the kind of
/// identifier it carries, so a control-only action can never reach an asset
/// event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FaultTarget {
    /// One scheduled event.
    Event(EventId),
    /// Every control event carrying this message id, including duplicates.
    Message(MessageId),
    /// Every asset event for this transfer, including pieces.
    Transfer(TransferId),
    /// Every event of one lane on one route.
    Route {
        source: EndpointId,
        destination: EndpointId,
        lane: Lane,
    },
    /// Every event of one lane.
    Lane(Lane),
}

impl FaultTarget {
    /// The lane this target implies, when it can be read without a lookup.
    #[must_use]
    pub const fn lane_hint(self) -> Option<Lane> {
        match self {
            Self::Event(_) => None,
            Self::Message(_) => Some(Lane::Control),
            Self::Transfer(_) => Some(Lane::Asset),
            Self::Route { lane, .. } | Self::Lane(lane) => Some(lane),
        }
    }

    /// True when this event is in scope of the target.
    #[must_use]
    pub fn matches(self, event: &Event) -> bool {
        match self {
            Self::Event(id) => event.id() == id,
            Self::Message(id) => event.message_id() == Some(id),
            Self::Transfer(id) => event.transfer_id() == Some(id),
            Self::Route {
                source,
                destination,
                lane,
            } => {
                event.lane() == lane
                    && event.source() == source
                    && event.intended_destination() == destination
            }
            Self::Lane(lane) => event.lane() == lane,
        }
    }
}

/// What a fault does to a matched event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FaultAction {
    /// Push the delivery further into the future.
    Delay { ticks: u64 },
    /// Never deliver the event.
    Drop,
    /// Send extra copies with the same content.
    Duplicate { copies: u16, spacing: u64 },
    /// Deliver to an endpoint the sender did not choose.
    ReplaceDestination(EndpointId),
    /// Flip the bits of one byte, leaving every other byte alone.
    CorruptByte { offset: usize, mask: u8 },
    /// Deliver less than the request.
    Partial { amount: AssetAmount },
    /// Deliver the request across several events.
    Split {
        pieces: Vec<AssetAmount>,
        spacing: u64,
    },
    /// Deliver more than the request.
    OverDeliver { amount: AssetAmount },
}

/// The pass in which an action runs.
///
/// Composition follows this order so a reroute is decided before content, and
/// a drop is decided last. Copies made in the fanout pass therefore carry the
/// rerouted destination and the corrupted bytes, and they survive a drop that
/// targets the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FaultStage {
    Reroute,
    Content,
    Fanout,
    Delay,
    Drop,
}

/// Actions in one group cannot both apply to a single event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExclusionGroup {
    Destination,
    Content,
    Fanout,
    Timing,
    Removal,
}

impl FaultAction {
    #[must_use]
    pub const fn stage(&self) -> FaultStage {
        match self {
            Self::ReplaceDestination(_) => FaultStage::Reroute,
            Self::CorruptByte { .. }
            | Self::Partial { .. }
            | Self::Split { .. }
            | Self::OverDeliver { .. } => FaultStage::Content,
            Self::Duplicate { .. } => FaultStage::Fanout,
            Self::Delay { .. } => FaultStage::Delay,
            Self::Drop => FaultStage::Drop,
        }
    }

    #[must_use]
    pub const fn group(&self) -> ExclusionGroup {
        match self {
            Self::ReplaceDestination(_) => ExclusionGroup::Destination,
            Self::CorruptByte { .. }
            | Self::Partial { .. }
            | Self::Split { .. }
            | Self::OverDeliver { .. } => ExclusionGroup::Content,
            Self::Duplicate { .. } => ExclusionGroup::Fanout,
            Self::Delay { .. } => ExclusionGroup::Timing,
            Self::Drop => ExclusionGroup::Removal,
        }
    }

    /// True when the action adds events instead of only bending one.
    ///
    /// An event the simulator made never runs these again, so copies and
    /// pieces cannot grow without end.
    #[must_use]
    pub const fn creates_events(&self) -> bool {
        matches!(self, Self::Duplicate { .. } | Self::Split { .. })
    }

    /// The lane this action makes sense on, or `None` when it fits both.
    #[must_use]
    pub const fn required_lane(&self) -> Option<Lane> {
        match self {
            Self::CorruptByte { .. } => Some(Lane::Control),
            Self::Partial { .. } | Self::Split { .. } | Self::OverDeliver { .. } => {
                Some(Lane::Asset)
            }
            Self::Delay { .. }
            | Self::Drop
            | Self::Duplicate { .. }
            | Self::ReplaceDestination(_) => None,
        }
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Delay { .. } => "delay",
            Self::Drop => "drop",
            Self::Duplicate { .. } => "duplicate",
            Self::ReplaceDestination(_) => "reroute",
            Self::CorruptByte { .. } => "corrupt",
            Self::Partial { .. } => "partial",
            Self::Split { .. } => "split",
            Self::OverDeliver { .. } => "over deliver",
        }
    }
}

/// One declared fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub id: FaultId,
    pub target: FaultTarget,
    pub action: FaultAction,
}

impl Fault {
    #[must_use]
    pub const fn new(id: FaultId, target: FaultTarget, action: FaultAction) -> Self {
        Self { id, target, action }
    }

    /// Sort key that fixes composition order between two faults.
    #[must_use]
    pub fn order_key(&self) -> (FaultStage, FaultId) {
        (self.action.stage(), self.id)
    }
}

impl core::fmt::Display for Fault {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "{} {}", self.id, self.action.name())
    }
}

/// Every fault that is in force, ordered by identifier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaultPlan {
    faults: BTreeMap<FaultId, Fault>,
}

impl FaultPlan {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.faults.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.faults.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: FaultId) -> Option<&Fault> {
        self.faults.get(&id)
    }

    #[must_use]
    pub fn contains(&self, id: FaultId) -> bool {
        self.faults.contains_key(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Fault> {
        self.faults.values()
    }

    pub(crate) fn insert(&mut self, fault: Fault) {
        self.faults.insert(fault.id, fault);
    }

    /// Faults that reach one event, already in composition order.
    #[must_use]
    pub fn matching(&self, event: &Event) -> Vec<&Fault> {
        let mut found: Vec<&Fault> = self
            .faults
            .values()
            .filter(|fault| fault.target.matches(event))
            .collect();
        found.sort_by_key(|fault| fault.order_key());
        found
    }
}

impl core::fmt::Display for FaultPlan {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("plan[")?;
        for (position, fault) in self.faults.values().enumerate() {
            if position > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{fault}")?;
        }
        formatter.write_str("]")
    }
}

/// Deterministic bit mixer used only to lay out a seeded plan.
///
/// It never runs while events are being delivered.
#[derive(Clone, Copy, Debug)]
pub struct PlanSeed {
    state: u64,
}

impl PlanSeed {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    fn below(&mut self, limit: u64) -> u64 {
        self.next().checked_rem(limit).unwrap_or(0)
    }
}

/// Builds one concrete plan from a seed.
///
/// The plan is produced up front and can be printed, so a property failure
/// carries the exact faults that ran.
#[must_use]
pub fn seeded_plan(seed: u64, targets: &[(EventId, Lane)]) -> FaultPlan {
    let mut source = PlanSeed::new(seed);
    let mut plan = FaultPlan::new();
    for (position, (event, lane)) in targets.iter().enumerate() {
        let Ok(index) = u32::try_from(position) else {
            break;
        };
        let action = match (lane, source.below(4)) {
            (_, 0) => FaultAction::Delay {
                ticks: source.below(8).saturating_add(1),
            },
            (_, 1) => FaultAction::Drop,
            (_, 2) => FaultAction::Duplicate {
                copies: 1,
                spacing: source.below(4),
            },
            (Lane::Control, _) => FaultAction::CorruptByte {
                offset: usize::try_from(source.below(64)).unwrap_or(0),
                mask: u8::try_from(source.below(255).saturating_add(1)).unwrap_or(1),
            },
            (Lane::Asset, _) => FaultAction::Partial {
                amount: AssetAmount::new(u128::from(source.below(16).saturating_add(1))),
            },
        };
        plan.insert(Fault::new(
            FaultId::new(index),
            FaultTarget::Event(*event),
            action,
        ));
    }
    plan
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;
    use crate::event::{AssetEvent, ControlEvent, EventStatus};
    use crate::time::Tick;

    fn control_event(id: u64, source: u32, destination: u32) -> Event {
        Event::Control(ControlEvent {
            id: EventId::new(id),
            source: EndpointId::new(source),
            destination: EndpointId::new(destination),
            intended_destination: EndpointId::new(destination),
            bytes: alloc::vec![1, 2, 3, 4],
            message_id: MessageId::new([3u8; 32]),
            deliver_at: Tick::new(1),
            attempts: 0,
            duplicate_of: None,
            mutation: None,
            expires_at: None,
            from_fault: false,
            status: EventStatus::Scheduled,
        })
    }

    fn asset_event(id: u64) -> Event {
        Event::Asset(AssetEvent {
            id: EventId::new(id),
            transfer: TransferId::new([5u8; 32]),
            source: EndpointId::new(1),
            destination: EndpointId::new(2),
            intended_destination: EndpointId::new(2),
            requested: AssetAmount::new(50),
            delivered: AssetAmount::new(50),
            deliver_at: Tick::new(1),
            attempts: 0,
            duplicate_of: None,
            piece: None,
            over_delivered: false,
            timeout_at: None,
            from_fault: false,
            status: EventStatus::Scheduled,
        })
    }

    #[test]
    fn stages_run_from_reroute_through_drop() {
        assert!(FaultStage::Reroute < FaultStage::Content);
        assert!(FaultStage::Content < FaultStage::Fanout);
        assert!(FaultStage::Fanout < FaultStage::Delay);
        assert!(FaultStage::Delay < FaultStage::Drop);
    }

    #[test]
    fn every_amount_action_shares_one_exclusion_group() {
        let content = [
            FaultAction::Partial {
                amount: AssetAmount::new(1),
            },
            FaultAction::Split {
                pieces: alloc::vec![AssetAmount::new(1)],
                spacing: 0,
            },
            FaultAction::OverDeliver {
                amount: AssetAmount::new(1),
            },
            FaultAction::CorruptByte { offset: 0, mask: 1 },
        ];
        for action in content {
            assert_eq!(action.group(), ExclusionGroup::Content);
            assert_eq!(action.stage(), FaultStage::Content);
        }
    }

    #[test]
    fn each_action_declares_the_lane_it_needs() {
        assert_eq!(
            FaultAction::CorruptByte { offset: 0, mask: 1 }.required_lane(),
            Some(Lane::Control)
        );
        assert_eq!(
            FaultAction::Partial {
                amount: AssetAmount::new(1)
            }
            .required_lane(),
            Some(Lane::Asset)
        );
        assert_eq!(FaultAction::Drop.required_lane(), None);
    }

    #[test]
    fn a_target_reports_the_lane_it_implies() {
        assert_eq!(
            FaultTarget::Message(MessageId::ZERO).lane_hint(),
            Some(Lane::Control)
        );
        assert_eq!(
            FaultTarget::Transfer(TransferId::ZERO).lane_hint(),
            Some(Lane::Asset)
        );
        assert_eq!(
            FaultTarget::Lane(Lane::Asset).lane_hint(),
            Some(Lane::Asset)
        );
        assert_eq!(FaultTarget::Event(EventId::new(1)).lane_hint(), None);
    }

    #[test]
    fn a_route_target_matches_only_that_route_and_lane() {
        let event = control_event(1, 7, 8);
        assert!(
            FaultTarget::Route {
                source: EndpointId::new(7),
                destination: EndpointId::new(8),
                lane: Lane::Control,
            }
            .matches(&event)
        );
        assert!(
            !FaultTarget::Route {
                source: EndpointId::new(7),
                destination: EndpointId::new(9),
                lane: Lane::Control,
            }
            .matches(&event)
        );
        assert!(
            !FaultTarget::Route {
                source: EndpointId::new(7),
                destination: EndpointId::new(8),
                lane: Lane::Asset,
            }
            .matches(&event)
        );
    }

    #[test]
    fn identifier_targets_pick_the_matching_lane_only() {
        let control = control_event(1, 1, 2);
        let asset = asset_event(2);
        assert!(FaultTarget::Message(MessageId::new([3u8; 32])).matches(&control));
        assert!(!FaultTarget::Message(MessageId::new([3u8; 32])).matches(&asset));
        assert!(FaultTarget::Transfer(TransferId::new([5u8; 32])).matches(&asset));
        assert!(!FaultTarget::Transfer(TransferId::new([5u8; 32])).matches(&control));
        assert!(FaultTarget::Lane(Lane::Asset).matches(&asset));
        assert!(FaultTarget::Event(EventId::new(2)).matches(&asset));
    }

    #[test]
    fn matching_faults_come_back_in_stage_then_identifier_order() {
        let mut plan = FaultPlan::new();
        plan.insert(Fault::new(
            FaultId::new(9),
            FaultTarget::Lane(Lane::Control),
            FaultAction::Drop,
        ));
        plan.insert(Fault::new(
            FaultId::new(3),
            FaultTarget::Lane(Lane::Control),
            FaultAction::Delay { ticks: 1 },
        ));
        plan.insert(Fault::new(
            FaultId::new(7),
            FaultTarget::Lane(Lane::Control),
            FaultAction::ReplaceDestination(EndpointId::new(5)),
        ));
        plan.insert(Fault::new(
            FaultId::new(1),
            FaultTarget::Lane(Lane::Asset),
            FaultAction::Drop,
        ));

        let order: Vec<u32> = plan
            .matching(&control_event(1, 1, 2))
            .iter()
            .map(|fault| fault.id.get())
            .collect();
        assert_eq!(order, alloc::vec![7, 3, 9]);
    }

    #[test]
    fn a_plan_reports_its_size_and_members() {
        let mut plan = FaultPlan::new();
        assert!(plan.is_empty());
        plan.insert(Fault::new(
            FaultId::new(2),
            FaultTarget::Lane(Lane::Control),
            FaultAction::Drop,
        ));
        assert_eq!(plan.len(), 1);
        assert!(plan.contains(FaultId::new(2)));
        assert!(plan.get(FaultId::new(2)).is_some());
        assert_eq!(plan.iter().count(), 1);
        assert_eq!(plan.to_string(), "plan[f2 drop]");
    }

    #[test]
    fn the_same_seed_lays_out_the_same_plan() {
        let targets = [
            (EventId::new(1), Lane::Control),
            (EventId::new(2), Lane::Asset),
            (EventId::new(3), Lane::Control),
            (EventId::new(4), Lane::Asset),
        ];
        assert_eq!(seeded_plan(77, &targets), seeded_plan(77, &targets));
        assert_ne!(seeded_plan(77, &targets), seeded_plan(78, &targets));
        assert_eq!(seeded_plan(77, &targets).len(), 4);
    }

    #[test]
    fn a_seeded_plan_never_puts_a_control_action_on_an_asset_event() {
        let targets: Vec<(EventId, Lane)> = (0..64)
            .map(|index| {
                let lane = if index % 2 == 0 {
                    Lane::Control
                } else {
                    Lane::Asset
                };
                (EventId::new(index), lane)
            })
            .collect();
        for seed in 0..32u64 {
            let plan = seeded_plan(seed, &targets);
            for fault in plan.iter() {
                let FaultTarget::Event(event) = fault.target else {
                    continue;
                };
                let lane = targets
                    .iter()
                    .find(|(id, _)| *id == event)
                    .map(|(_, lane)| *lane);
                if let Some(required) = fault.action.required_lane() {
                    assert_eq!(Some(required), lane);
                }
            }
        }
    }

    #[test]
    fn every_action_has_a_short_name() {
        let actions = [
            FaultAction::Delay { ticks: 1 },
            FaultAction::Drop,
            FaultAction::Duplicate {
                copies: 1,
                spacing: 0,
            },
            FaultAction::ReplaceDestination(EndpointId::new(1)),
            FaultAction::CorruptByte { offset: 0, mask: 1 },
            FaultAction::Partial {
                amount: AssetAmount::new(1),
            },
            FaultAction::Split {
                pieces: alloc::vec![AssetAmount::new(1)],
                spacing: 0,
            },
            FaultAction::OverDeliver {
                amount: AssetAmount::new(1),
            },
        ];
        let mut seen: Vec<&str> = Vec::new();
        for action in actions {
            assert!(!seen.contains(&action.name()));
            seen.push(action.name());
        }
    }

    #[test]
    fn a_fault_id_prints_and_converts() {
        assert_eq!(FaultId::from(4u32).get(), 4);
        assert_eq!(FaultId::new(4).to_string(), "f4");
    }
}
