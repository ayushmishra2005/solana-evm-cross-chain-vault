//! The deterministic transport engine.
//!
//! Control messages and asset value ride separate lanes. Nothing here reads a
//! system clock, draws a random number, or asks a map for its iteration order.

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use protocol_types::{AssetAmount, MessageId, TransferId};

use crate::endpoint::{DeliveredAsset, DeliveredControl, Endpoint, EndpointId, EndpointState};
use crate::error::{ConfigProblem, SimError};
use crate::event::{AssetEvent, ByteMutation, ControlEvent, Event, EventId, EventStatus};
use crate::fault::{ExclusionGroup, Fault, FaultAction, FaultId, FaultPlan, FaultTarget};
use crate::inspect::message_identity;
use crate::lane::{Lane, LaneState};
use crate::queue::{EventQueue, QueueKey};
use crate::state_hash::{StateHash, state_hash};
use crate::time::Tick;
use crate::trace::{BlockReason, FaultEffect, RejectReason, Subject, Trace, TraceAction};

/// What to do with a delivery that arrives past its deadline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LatePolicy {
    /// Hand it over and mark it late, leaving the choice to the endpoint.
    #[default]
    DeliverWithMarker,
    /// Stop it at the transport and never deliver it.
    Expire,
}

/// Per lane policy the caller picks before a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimulatorConfig {
    pub control_late_policy: LatePolicy,
    pub asset_late_policy: LatePolicy,
}

impl SimulatorConfig {
    #[must_use]
    pub const fn expiring() -> Self {
        Self {
            control_late_policy: LatePolicy::Expire,
            asset_late_policy: LatePolicy::Expire,
        }
    }

    #[must_use]
    pub const fn policy(self, lane: Lane) -> LatePolicy {
        match lane {
            Lane::Control => self.control_late_policy,
            Lane::Asset => self.asset_late_policy,
        }
    }
}

/// A control message the caller wants delivered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlRequest {
    pub id: Option<EventId>,
    pub source: EndpointId,
    pub destination: EndpointId,
    pub bytes: Vec<u8>,
    pub deliver_at: Tick,
    /// A transport deadline, never read from the message body.
    pub expires_at: Option<Tick>,
}

impl ControlRequest {
    #[must_use]
    pub fn new(
        source: EndpointId,
        destination: EndpointId,
        bytes: Vec<u8>,
        deliver_at: Tick,
    ) -> Self {
        Self {
            id: None,
            source,
            destination,
            bytes,
            deliver_at,
            expires_at: None,
        }
    }

    #[must_use]
    pub fn with_id(mut self, id: EventId) -> Self {
        self.id = Some(id);
        self
    }

    #[must_use]
    pub fn expiring_at(mut self, tick: Tick) -> Self {
        self.expires_at = Some(tick);
        self
    }
}

/// An asset movement the caller wants delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssetRequest {
    pub id: Option<EventId>,
    pub transfer: TransferId,
    pub source: EndpointId,
    pub destination: EndpointId,
    pub amount: AssetAmount,
    pub deliver_at: Tick,
    pub timeout_at: Option<Tick>,
}

impl AssetRequest {
    #[must_use]
    pub const fn new(
        transfer: TransferId,
        source: EndpointId,
        destination: EndpointId,
        amount: AssetAmount,
        deliver_at: Tick,
    ) -> Self {
        Self {
            id: None,
            transfer,
            source,
            destination,
            amount,
            deliver_at,
            timeout_at: None,
        }
    }

    #[must_use]
    pub const fn with_id(mut self, id: EventId) -> Self {
        self.id = Some(id);
        self
    }

    #[must_use]
    pub const fn timing_out_at(mut self, tick: Tick) -> Self {
        self.timeout_at = Some(tick);
        self
    }
}

/// What the fault passes decided about one attempt.
enum Outcome {
    Proceed,
    Delayed(Tick),
    Dropped(FaultId),
    Rejected(RejectReason),
}

/// The whole simulated network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Simulator {
    config: SimulatorConfig,
    now: Tick,
    endpoints: BTreeMap<EndpointId, Endpoint>,
    events: BTreeMap<EventId, Event>,
    queue: EventQueue,
    held: BTreeSet<EventId>,
    lanes: BTreeMap<Lane, LaneState>,
    plan: FaultPlan,
    resolved: BTreeSet<EventId>,
    trace: Trace,
    next_event: u64,
}

impl Simulator {
    /// Builds a network from a list of endpoint identifiers.
    pub fn new(endpoints: &[EndpointId]) -> Result<Self, SimError> {
        Self::with_config(SimulatorConfig::default(), endpoints)
    }

    pub fn with_config(
        config: SimulatorConfig,
        endpoints: &[EndpointId],
    ) -> Result<Self, SimError> {
        if endpoints.is_empty() {
            return Err(SimError::InvalidConfiguration(ConfigProblem::NoEndpoints));
        }
        let mut map = BTreeMap::new();
        for id in endpoints {
            if map.insert(*id, Endpoint::new(*id)).is_some() {
                return Err(SimError::DuplicateEndpoint(*id));
            }
        }
        let mut lanes = BTreeMap::new();
        for lane in Lane::ALL {
            lanes.insert(lane, LaneState::Running);
        }
        Ok(Self {
            config,
            now: Tick::ZERO,
            endpoints: map,
            events: BTreeMap::new(),
            queue: EventQueue::new(),
            held: BTreeSet::new(),
            lanes,
            plan: FaultPlan::new(),
            resolved: BTreeSet::new(),
            trace: Trace::new(),
            next_event: 1,
        })
    }

    pub fn add_endpoint(&mut self, id: EndpointId) -> Result<(), SimError> {
        if self.endpoints.contains_key(&id) {
            return Err(SimError::DuplicateEndpoint(id));
        }
        self.endpoints.insert(id, Endpoint::new(id));
        Ok(())
    }

    #[must_use]
    pub const fn config(&self) -> SimulatorConfig {
        self.config
    }

    #[must_use]
    pub const fn now(&self) -> Tick {
        self.now
    }

    #[must_use]
    pub fn endpoint(&self, id: EndpointId) -> Option<&Endpoint> {
        self.endpoints.get(&id)
    }

    pub fn endpoints(&self) -> impl Iterator<Item = &Endpoint> {
        self.endpoints.values()
    }

    #[must_use]
    pub fn endpoint_count(&self) -> usize {
        self.endpoints.len()
    }

    #[must_use]
    pub fn event(&self, id: EventId) -> Option<&Event> {
        self.events.get(&id)
    }

    pub fn events(&self) -> impl Iterator<Item = &Event> {
        self.events.values()
    }

    #[must_use]
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub const fn queue(&self) -> &EventQueue {
        &self.queue
    }

    #[must_use]
    pub const fn held(&self) -> &BTreeSet<EventId> {
        &self.held
    }

    #[must_use]
    pub const fn plan(&self) -> &FaultPlan {
        &self.plan
    }

    /// Events whose fault batch has already been decided.
    #[must_use]
    pub const fn resolved_events(&self) -> &BTreeSet<EventId> {
        &self.resolved
    }

    #[must_use]
    pub const fn trace(&self) -> &Trace {
        &self.trace
    }

    #[must_use]
    pub const fn next_event_number(&self) -> u64 {
        self.next_event
    }

    #[must_use]
    pub fn lane_state(&self, lane: Lane) -> LaneState {
        self.lanes.get(&lane).copied().unwrap_or_default()
    }

    /// Digest of everything above.
    #[must_use]
    pub fn state_hash(&self) -> StateHash {
        state_hash(self)
    }

    // Scheduling.

    pub fn schedule_control(&mut self, request: ControlRequest) -> Result<EventId, SimError> {
        self.check_route(request.source, request.destination)?;
        if request.bytes.is_empty() {
            return Err(SimError::InvalidConfiguration(ConfigProblem::EmptyMessage));
        }
        self.check_delivery_tick(request.deliver_at)?;
        let id = self.reserve_event_id(request.id)?;
        let message_id = message_identity(&request.bytes);
        let event = Event::Control(ControlEvent {
            id,
            source: request.source,
            destination: request.destination,
            intended_destination: request.destination,
            bytes: request.bytes,
            message_id,
            deliver_at: request.deliver_at,
            attempts: 0,
            duplicate_of: None,
            mutation: None,
            expires_at: request.expires_at,
            from_fault: false,
            status: EventStatus::Scheduled,
        });
        self.insert_scheduled(event, Subject::Message(message_id));
        Ok(id)
    }

    pub fn schedule_asset(&mut self, request: AssetRequest) -> Result<EventId, SimError> {
        self.check_route(request.source, request.destination)?;
        if request.amount.is_zero() {
            return Err(SimError::ZeroAssetAmount);
        }
        self.check_delivery_tick(request.deliver_at)?;
        let id = self.reserve_event_id(request.id)?;
        let event = Event::Asset(AssetEvent {
            id,
            transfer: request.transfer,
            source: request.source,
            destination: request.destination,
            intended_destination: request.destination,
            requested: request.amount,
            delivered: request.amount,
            deliver_at: request.deliver_at,
            attempts: 0,
            duplicate_of: None,
            piece: None,
            over_delivered: false,
            timeout_at: request.timeout_at,
            from_fault: false,
            status: EventStatus::Scheduled,
        });
        self.insert_scheduled(event, Subject::Transfer(request.transfer));
        Ok(id)
    }

    fn insert_scheduled(&mut self, event: Event, subject: Subject) {
        let key = QueueKey::new(event.deliver_at(), event.lane(), event.id());
        self.trace.push(
            self.now,
            TraceAction::EventScheduled {
                event: event.id(),
                lane: event.lane(),
                source: event.source(),
                destination: event.destination(),
                subject,
                deliver_at: event.deliver_at(),
            },
        );
        self.events.insert(event.id(), event);
        self.queue.insert(key);
    }

    fn check_route(&self, source: EndpointId, destination: EndpointId) -> Result<(), SimError> {
        self.require_endpoint(source)?;
        self.require_endpoint(destination)?;
        if source == destination {
            return Err(SimError::InvalidConfiguration(
                ConfigProblem::SameEndpointRoute,
            ));
        }
        Ok(())
    }

    fn require_endpoint(&self, id: EndpointId) -> Result<(), SimError> {
        if self.endpoints.contains_key(&id) {
            Ok(())
        } else {
            Err(SimError::UnknownEndpoint(id))
        }
    }

    fn check_delivery_tick(&self, tick: Tick) -> Result<(), SimError> {
        if tick < self.now {
            return Err(SimError::DeliveryTickInPast {
                now: self.now,
                requested: tick,
            });
        }
        Ok(())
    }

    fn reserve_event_id(&mut self, wanted: Option<EventId>) -> Result<EventId, SimError> {
        let id = wanted.unwrap_or(EventId::new(self.next_event));
        if self.events.contains_key(&id) {
            return Err(SimError::DuplicateEventId(id));
        }
        self.next_event = id
            .get()
            .checked_add(1)
            .ok_or(SimError::ArithmeticOverflow)?
            .max(self.next_event);
        Ok(id)
    }

    fn take_generated_id(&mut self) -> Option<EventId> {
        let mut candidate = self.next_event;
        loop {
            let id = EventId::new(candidate);
            if !self.events.contains_key(&id) {
                self.next_event = candidate.checked_add(1)?;
                return Some(id);
            }
            candidate = candidate.checked_add(1)?;
        }
    }

    // Faults.

    /// Registers a fault, refusing anything that cannot combine safely.
    pub fn add_fault(&mut self, fault: Fault) -> Result<(), SimError> {
        if self.plan.contains(fault.id) {
            return Err(SimError::DuplicateFaultId(fault.id));
        }
        let lane = self.target_lane(&fault)?;
        if let Some(required) = fault.action.required_lane()
            && required != lane
        {
            return Err(SimError::InvalidConfiguration(
                ConfigProblem::ActionLaneMismatch,
            ));
        }
        self.check_target_endpoints(&fault)?;
        check_action_shape(&fault.action)?;
        if let FaultTarget::Event(id) = fault.target {
            self.check_against_named_event(id, &fault.action)?;
        }
        if let Some(first) = self.same_group_on_same_target(&fault) {
            return Err(SimError::ConflictingFaults {
                first,
                second: fault.id,
            });
        }
        self.plan.insert(fault);
        Ok(())
    }

    fn target_lane(&self, fault: &Fault) -> Result<Lane, SimError> {
        match fault.target {
            FaultTarget::Event(id) => self
                .events
                .get(&id)
                .map(Event::lane)
                .ok_or(SimError::UnknownEvent(id)),
            other => other
                .lane_hint()
                .ok_or(SimError::UnknownFaultTarget(fault.id)),
        }
    }

    fn check_target_endpoints(&self, fault: &Fault) -> Result<(), SimError> {
        if let FaultTarget::Route {
            source,
            destination,
            ..
        } = fault.target
        {
            self.require_endpoint(source)?;
            self.require_endpoint(destination)?;
        }
        if let FaultAction::ReplaceDestination(to) = fault.action {
            self.require_endpoint(to)?;
        }
        Ok(())
    }

    fn check_against_named_event(&self, id: EventId, action: &FaultAction) -> Result<(), SimError> {
        let event = self.events.get(&id).ok_or(SimError::UnknownEvent(id))?;
        if event.status().is_terminal() {
            return Err(SimError::EventAlreadyTerminal(id));
        }
        if self.resolved.contains(&id) {
            return Err(SimError::FaultsAlreadyBound(id));
        }
        match action {
            FaultAction::CorruptByte { offset, .. } => {
                let width = event.control().map_or(0, |control| control.bytes.len());
                if *offset >= width {
                    return Err(SimError::InvalidConfiguration(
                        ConfigProblem::CorruptOffsetOutOfRange,
                    ));
                }
            }
            FaultAction::Partial { amount } => {
                let requested = event.asset().map_or(0, |asset| asset.requested.get());
                if amount.get() > requested {
                    return Err(SimError::PartialSumExceedsRequest {
                        requested,
                        offered: amount.get(),
                    });
                }
            }
            FaultAction::Split { pieces, .. } => {
                let requested = event.asset().map_or(0, |asset| asset.requested.get());
                let offered = sum_pieces(pieces).ok_or(SimError::ArithmeticOverflow)?;
                if offered > requested {
                    return Err(SimError::PartialSumExceedsRequest { requested, offered });
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn same_group_on_same_target(&self, fault: &Fault) -> Option<FaultId> {
        self.plan
            .iter()
            .find(|existing| {
                existing.target == fault.target && existing.action.group() == fault.action.group()
            })
            .map(|existing| existing.id)
    }

    // Time.

    pub fn advance_by(&mut self, ticks: u64) -> Result<(), SimError> {
        let target = self
            .now
            .checked_add(ticks)
            .ok_or(SimError::ArithmeticOverflow)?;
        self.advance_to(target)
    }

    pub fn advance_to(&mut self, target: Tick) -> Result<(), SimError> {
        if target < self.now {
            return Err(SimError::TimeMovesBackwards {
                now: self.now,
                requested: target,
            });
        }
        if target == self.now {
            return Ok(());
        }
        let from = self.now;
        self.now = target;
        self.trace
            .push(target, TraceAction::TickAdvanced { from, to: target });
        Ok(())
    }

    // Delivery.

    /// Attempts the earliest event that is due, if any.
    pub fn deliver_next(&mut self) -> Option<EventId> {
        let key = self.queue.peek()?;
        if key.deliver_at > self.now {
            return None;
        }
        self.queue.pop();
        self.attempt(key.event);
        Some(key.event)
    }

    /// Attempts every event that is due at the current tick.
    pub fn deliver_ready(&mut self) -> u32 {
        let mut count: u32 = 0;
        while self.deliver_next().is_some() {
            count = count.saturating_add(1);
        }
        count
    }

    /// Runs until the queue holds nothing, moving time to each due tick.
    ///
    /// An event whose tick already passed is delivered where the clock stands,
    /// because time only ever moves forward.
    pub fn run_until_idle(&mut self) -> u32 {
        let mut count: u32 = 0;
        while let Some(tick) = self.queue.next_tick() {
            if self.advance_to(tick.max(self.now)).is_err() {
                break;
            }
            count = count.saturating_add(self.deliver_ready());
        }
        count
    }

    /// Runs up to and including one tick, then leaves time there.
    pub fn run_until(&mut self, target: Tick) -> Result<u32, SimError> {
        if target < self.now {
            return Err(SimError::TimeMovesBackwards {
                now: self.now,
                requested: target,
            });
        }
        let mut count: u32 = 0;
        while let Some(tick) = self.queue.next_tick() {
            if tick > target {
                break;
            }
            self.advance_to(tick.max(self.now))?;
            count = count.saturating_add(self.deliver_ready());
        }
        self.advance_to(target)?;
        Ok(count)
    }

    fn attempt(&mut self, id: EventId) {
        let Some(mut event) = self.events.get(&id).cloned() else {
            return;
        };
        if event.status().is_terminal() {
            return;
        }
        event.bump_attempts();
        event.set_status(EventStatus::Ready);
        self.trace.push(
            self.now,
            TraceAction::DeliveryAttempted {
                event: id,
                destination: event.destination(),
                attempt: event.attempts(),
            },
        );

        match self.compose(&mut event) {
            Outcome::Rejected(reason) => {
                event.set_status(EventStatus::RejectedBySimulator);
                self.events.insert(id, event);
                self.trace
                    .push(self.now, TraceAction::EventRejected { event: id, reason });
                return;
            }
            Outcome::Dropped(fault) => {
                event.set_status(EventStatus::Dropped);
                self.events.insert(id, event);
                self.trace.push(
                    self.now,
                    TraceAction::EventDropped {
                        event: id,
                        fault: Some(fault),
                    },
                );
                return;
            }
            Outcome::Delayed(to) => {
                let lane = event.lane();
                event.set_status(EventStatus::Scheduled);
                event.set_deliver_at(to);
                self.events.insert(id, event);
                self.queue.insert(QueueKey::new(to, lane, id));
                return;
            }
            Outcome::Proceed => {}
        }

        let lane = event.lane();
        if self.lane_state(lane).is_paused() {
            self.block(event, BlockReason::LanePaused);
            return;
        }
        let destination = event.destination();
        match self.endpoints.get(&destination) {
            None => {
                event.set_status(EventStatus::RejectedBySimulator);
                self.events.insert(id, event);
                self.trace.push(
                    self.now,
                    TraceAction::EventRejected {
                        event: id,
                        reason: RejectReason::UnknownEndpoint,
                    },
                );
                return;
            }
            Some(endpoint) if endpoint.is_halted() => {
                self.block(event, BlockReason::EndpointHalted);
                return;
            }
            Some(_) => {}
        }

        let mut late = false;
        if let Some(deadline) = event.deadline()
            && self.now > deadline
        {
            match self.config.policy(lane) {
                LatePolicy::Expire => {
                    event.set_status(EventStatus::Expired);
                    self.events.insert(id, event);
                    self.trace.push(
                        self.now,
                        TraceAction::EventExpired {
                            event: id,
                            deadline,
                        },
                    );
                    return;
                }
                LatePolicy::DeliverWithMarker => late = true,
            }
        }

        self.hand_over(event, late);
    }

    fn block(&mut self, mut event: Event, reason: BlockReason) {
        let id = event.id();
        let destination = event.destination();
        event.set_status(EventStatus::Blocked);
        self.events.insert(id, event);
        self.held.insert(id);
        self.trace.push(
            self.now,
            TraceAction::DeliveryBlocked {
                event: id,
                destination,
                reason,
            },
        );
    }

    fn hand_over(&mut self, mut event: Event, late: bool) {
        let id = event.id();
        let destination = event.destination();
        event.set_status(EventStatus::Delivered);
        let subject = match &event {
            Event::Control(control) => {
                let record = DeliveredControl {
                    event: id,
                    source: control.source,
                    intended_destination: control.intended_destination,
                    bytes: control.bytes.clone(),
                    message_id: control.message_id,
                    delivered_at: self.now,
                    duplicate_of: control.duplicate_of,
                    mutation: control.mutation,
                    after_deadline: late,
                };
                if let Some(endpoint) = self.endpoints.get_mut(&destination) {
                    endpoint.push_control(record);
                }
                Subject::Message(control.message_id)
            }
            Event::Asset(asset) => {
                let record = DeliveredAsset {
                    event: id,
                    transfer: asset.transfer,
                    source: asset.source,
                    intended_destination: asset.intended_destination,
                    requested: asset.requested,
                    delivered: asset.delivered,
                    delivered_at: self.now,
                    duplicate_of: asset.duplicate_of,
                    piece: asset.piece,
                    over_delivered: asset.over_delivered,
                    after_timeout: late,
                };
                if let Some(endpoint) = self.endpoints.get_mut(&destination) {
                    endpoint.push_asset(record);
                }
                Subject::Transfer(asset.transfer)
            }
        };
        self.events.insert(id, event);
        self.trace.push(
            self.now,
            TraceAction::EventDelivered {
                event: id,
                destination,
                subject,
                after_deadline: late,
            },
        );
    }

    // Fault composition.

    fn compose(&mut self, event: &mut Event) -> Outcome {
        if !self.resolved.insert(event.id()) {
            return Outcome::Proceed;
        }
        let batch = self.pending_faults(event);
        if batch.is_empty() {
            return Outcome::Proceed;
        }
        if let Some((fault, reason)) = self.check_batch(&batch, event) {
            self.trace.push(
                self.now,
                TraceAction::FaultRejected {
                    fault,
                    event: event.id(),
                    reason,
                },
            );
            return Outcome::Rejected(reason);
        }

        let mut delayed = None;
        let mut dropped = None;
        for fault in &batch {
            match &fault.action {
                FaultAction::ReplaceDestination(to) => {
                    event.set_destination(*to);
                    self.record_effect(fault.id, event.id(), FaultEffect::Rerouted { to: *to });
                }
                FaultAction::CorruptByte { offset, mask } => {
                    self.corrupt(fault.id, event, *offset, *mask);
                }
                FaultAction::Partial { amount } => {
                    set_delivered(event, *amount, false);
                    self.record_effect(
                        fault.id,
                        event.id(),
                        FaultEffect::AmountSet { to: *amount },
                    );
                }
                FaultAction::OverDeliver { amount } => {
                    set_delivered(event, *amount, true);
                    self.record_effect(
                        fault.id,
                        event.id(),
                        FaultEffect::AmountSet { to: *amount },
                    );
                }
                FaultAction::Split { pieces, spacing } => {
                    self.split(fault.id, event, pieces, *spacing);
                }
                FaultAction::Duplicate { copies, spacing } => {
                    self.duplicate(fault.id, event, *copies, *spacing);
                }
                FaultAction::Delay { ticks } => {
                    let to = self.now.checked_add(*ticks).unwrap_or(Tick::new(u64::MAX));
                    delayed = Some(to);
                    self.record_effect(fault.id, event.id(), FaultEffect::Delayed { to });
                }
                FaultAction::Drop => {
                    dropped = Some(fault.id);
                    self.record_effect(fault.id, event.id(), FaultEffect::Dropped);
                }
            }
        }
        if let Some(fault) = dropped {
            return Outcome::Dropped(fault);
        }
        if let Some(tick) = delayed {
            return Outcome::Delayed(tick);
        }
        Outcome::Proceed
    }

    fn pending_faults(&self, event: &Event) -> Vec<Fault> {
        let from_fault = event.made_by_fault();
        self.plan
            .matching(event)
            .into_iter()
            .filter(|fault| !(from_fault && fault.action.creates_events()))
            .cloned()
            .collect()
    }

    fn check_batch(&self, batch: &[Fault], event: &Event) -> Option<(FaultId, RejectReason)> {
        let mut groups: Vec<ExclusionGroup> = Vec::new();
        for fault in batch {
            let group = fault.action.group();
            if groups.contains(&group) {
                return Some((fault.id, RejectReason::ConflictingGroup));
            }
            groups.push(group);
            if let Some(reason) = self.check_action(&fault.action, event) {
                return Some((fault.id, reason));
            }
        }
        None
    }

    fn check_action(&self, action: &FaultAction, event: &Event) -> Option<RejectReason> {
        match action {
            FaultAction::ReplaceDestination(to) => {
                (!self.endpoints.contains_key(to)).then_some(RejectReason::UnknownEndpoint)
            }
            FaultAction::Duplicate { copies, .. } => {
                (*copies == 0).then_some(RejectReason::DuplicateCopiesIsZero)
            }
            FaultAction::CorruptByte { offset, mask } => {
                let control = event.control()?;
                if *mask == 0 {
                    return Some(RejectReason::CorruptMaskIsZero);
                }
                // One event carries one edit, so the record stays truthful.
                if control.mutation.is_some() {
                    return Some(RejectReason::AlreadyCorrupted);
                }
                (*offset >= control.bytes.len()).then_some(RejectReason::CorruptOffsetOutOfRange)
            }
            FaultAction::Partial { amount } => {
                let asset = event.asset()?;
                if amount.is_zero() {
                    return Some(RejectReason::AmountIsZero);
                }
                (amount.get() > asset.requested.get()).then_some(RejectReason::AmountExceedsRequest)
            }
            FaultAction::OverDeliver { amount } => {
                let asset = event.asset()?;
                (amount.get() <= asset.requested.get())
                    .then_some(RejectReason::OverDeliveryNotAboveRequest)
            }
            FaultAction::Split { pieces, .. } => {
                let asset = event.asset()?;
                if pieces.is_empty() {
                    return Some(RejectReason::SplitHasNoPieces);
                }
                if pieces.iter().any(|piece| piece.is_zero()) {
                    return Some(RejectReason::SplitPieceIsZero);
                }
                match sum_pieces(pieces) {
                    None => Some(RejectReason::SplitExceedsRequest),
                    Some(total) => {
                        (total > asset.requested.get()).then_some(RejectReason::SplitExceedsRequest)
                    }
                }
            }
            FaultAction::Delay { .. } | FaultAction::Drop => None,
        }
    }

    fn record_effect(&mut self, fault: FaultId, event: EventId, effect: FaultEffect) {
        self.trace.push(
            self.now,
            TraceAction::FaultApplied {
                fault,
                event,
                effect,
            },
        );
    }

    fn corrupt(&mut self, fault: FaultId, event: &mut Event, offset: usize, mask: u8) {
        let Event::Control(control) = event else {
            return;
        };
        let Some(slot) = control.bytes.get_mut(offset) else {
            return;
        };
        let from = *slot;
        let to = from ^ mask;
        *slot = to;
        let original_message_id = control
            .mutation
            .map_or(control.message_id, |previous| previous.original_message_id);
        control.message_id = message_identity(&control.bytes);
        control.mutation = Some(ByteMutation {
            offset,
            from,
            to,
            original_message_id,
        });
        self.record_effect(
            fault,
            control.id,
            FaultEffect::Corrupted { offset, from, to },
        );
    }

    fn split(&mut self, fault: FaultId, event: &mut Event, pieces: &[AssetAmount], spacing: u64) {
        let Event::Asset(asset) = event else {
            return;
        };
        let Some(first) = pieces.first().copied() else {
            return;
        };
        asset.delivered = first;
        asset.piece = Some(0);
        let origin = *asset;
        let count = u16::try_from(pieces.len()).unwrap_or(u16::MAX);
        self.record_effect(fault, origin.id, FaultEffect::SplitInto { pieces: count });

        for (position, amount) in pieces.iter().enumerate().skip(1) {
            let Some(id) = self.take_generated_id() else {
                break;
            };
            let index = u16::try_from(position).unwrap_or(u16::MAX);
            let offset = spacing.saturating_mul(u64::from(index));
            let deliver_at = self.now.checked_add(offset).unwrap_or(Tick::new(u64::MAX));
            let piece = AssetEvent {
                id,
                deliver_at,
                attempts: 0,
                delivered: *amount,
                piece: Some(index),
                from_fault: true,
                status: EventStatus::Scheduled,
                ..origin
            };
            self.events.insert(id, Event::Asset(piece));
            self.queue
                .insert(QueueKey::new(deliver_at, Lane::Asset, id));
            self.trace.push(
                self.now,
                TraceAction::PartialDeliveryCreated {
                    original: origin.id,
                    piece_event: id,
                    transfer: origin.transfer,
                    amount: *amount,
                    piece: index,
                },
            );
        }
    }

    fn duplicate(&mut self, fault: FaultId, event: &Event, copies: u16, spacing: u64) {
        self.record_effect(fault, event.id(), FaultEffect::Duplicated { copies });
        for index in 1..=copies {
            let Some(id) = self.take_generated_id() else {
                break;
            };
            let offset = spacing.saturating_mul(u64::from(index));
            let deliver_at = self.now.checked_add(offset).unwrap_or(Tick::new(u64::MAX));
            let copy = clone_for_duplicate(event, id, deliver_at);
            let lane = copy.lane();
            self.events.insert(id, copy);
            self.queue.insert(QueueKey::new(deliver_at, lane, id));
            self.trace.push(
                self.now,
                TraceAction::DuplicateCreated {
                    original: event.id(),
                    duplicate: id,
                    deliver_at,
                },
            );
        }
    }

    // Endpoint and lane control.

    pub fn halt_endpoint(&mut self, id: EndpointId) -> Result<(), SimError> {
        let endpoint = self
            .endpoints
            .get_mut(&id)
            .ok_or(SimError::UnknownEndpoint(id))?;
        endpoint.set_state(EndpointState::Halted);
        self.trace
            .push(self.now, TraceAction::EndpointHalted { endpoint: id });
        Ok(())
    }

    pub fn resume_endpoint(&mut self, id: EndpointId) -> Result<(), SimError> {
        let endpoint = self
            .endpoints
            .get_mut(&id)
            .ok_or(SimError::UnknownEndpoint(id))?;
        endpoint.set_state(EndpointState::Active);
        let released = self.release_held();
        self.trace.push(
            self.now,
            TraceAction::EndpointResumed {
                endpoint: id,
                released,
            },
        );
        Ok(())
    }

    pub fn pause_lane(&mut self, lane: Lane) {
        self.lanes.insert(lane, LaneState::Paused);
        self.trace.push(self.now, TraceAction::LanePaused { lane });
    }

    pub fn resume_lane(&mut self, lane: Lane) {
        self.lanes.insert(lane, LaneState::Running);
        let released = self.release_held();
        self.trace
            .push(self.now, TraceAction::LaneResumed { lane, released });
    }

    /// Puts every held event whose path is clear back into the queue.
    fn release_held(&mut self) -> u32 {
        let ready: Vec<EventId> = self
            .held
            .iter()
            .copied()
            .filter(|id| self.path_is_clear(*id))
            .collect();
        let mut released: u32 = 0;
        for id in ready {
            self.held.remove(&id);
            let now = self.now;
            let Some(event) = self.events.get_mut(&id) else {
                continue;
            };
            let tick = now.max(event.deliver_at());
            event.set_status(EventStatus::Scheduled);
            event.set_deliver_at(tick);
            let lane = event.lane();
            self.queue.insert(QueueKey::new(tick, lane, id));
            released = released.saturating_add(1);
        }
        released
    }

    fn path_is_clear(&self, id: EventId) -> bool {
        let Some(event) = self.events.get(&id) else {
            return false;
        };
        if self.lane_state(event.lane()).is_paused() {
            return false;
        }
        self.endpoints
            .get(&event.destination())
            .is_some_and(|endpoint| !endpoint.is_halted())
    }

    // Reordering.

    /// Swaps the delivery ticks of two pending events.
    ///
    /// Message bytes never change, so sequence fields stay as they were.
    pub fn swap_delivery_ticks(&mut self, left: EventId, right: EventId) -> Result<(), SimError> {
        let left_key = self.pending_key(left)?;
        let right_key = self.pending_key(right)?;
        self.move_to(left_key, right_key.deliver_at);
        self.move_to(right_key, left_key.deliver_at);
        Ok(())
    }

    /// Moves one pending event one tick ahead of another.
    pub fn move_before(&mut self, event: EventId, other: EventId) -> Result<(), SimError> {
        let key = self.pending_key(event)?;
        let anchor = self.pending_key(other)?;
        let target = Tick::new(anchor.deliver_at.get().saturating_sub(1)).max(self.now);
        self.move_to(key, target);
        Ok(())
    }

    /// Moves one pending event one tick behind another.
    pub fn move_after(&mut self, event: EventId, other: EventId) -> Result<(), SimError> {
        let key = self.pending_key(event)?;
        let anchor = self.pending_key(other)?;
        let target = anchor
            .deliver_at
            .checked_add(1)
            .ok_or(SimError::ArithmeticOverflow)?;
        self.move_to(key, target);
        Ok(())
    }

    fn pending_key(&self, event: EventId) -> Result<QueueKey, SimError> {
        if !self.events.contains_key(&event) {
            return Err(SimError::UnknownEvent(event));
        }
        self.queue
            .find(event)
            .ok_or(SimError::EventAlreadyTerminal(event))
    }

    fn move_to(&mut self, key: QueueKey, target: Tick) {
        if key.deliver_at == target {
            return;
        }
        self.queue.remove(&key);
        let Some(event) = self.events.get_mut(&key.event) else {
            return;
        };
        event.set_deliver_at(target);
        let lane = event.lane();
        self.queue.insert(QueueKey::new(target, lane, key.event));
        self.trace.push(
            self.now,
            TraceAction::EventReordered {
                event: key.event,
                from: key.deliver_at,
                to: target,
            },
        );
    }

    // Views over delivered content.

    /// Every control message one endpoint received.
    #[must_use]
    pub fn control_inbox(&self, endpoint: EndpointId) -> &[DeliveredControl] {
        self.endpoints
            .get(&endpoint)
            .map_or(&[], Endpoint::control_inbox)
    }

    /// Every asset movement one endpoint received.
    #[must_use]
    pub fn asset_inbox(&self, endpoint: EndpointId) -> &[DeliveredAsset] {
        self.endpoints
            .get(&endpoint)
            .map_or(&[], Endpoint::asset_inbox)
    }

    /// Total value delivered for one transfer, across every piece and copy.
    ///
    /// This is a reporting view. It does not say the transfer is finished.
    #[must_use]
    pub fn delivered_for_transfer(&self, transfer: TransferId) -> u128 {
        self.endpoints
            .values()
            .flat_map(Endpoint::asset_inbox)
            .filter(|entry| entry.transfer == transfer)
            .fold(0u128, |total, entry| {
                total.saturating_add(entry.delivered.get())
            })
    }

    /// Every delivery that carried one message identity.
    pub fn deliveries_of_message(
        &self,
        message: MessageId,
    ) -> impl Iterator<Item = &DeliveredControl> {
        self.endpoints
            .values()
            .flat_map(Endpoint::control_inbox)
            .filter(move |entry| entry.message_id == message)
    }
}

fn sum_pieces(pieces: &[AssetAmount]) -> Option<u128> {
    pieces
        .iter()
        .try_fold(0u128, |total, piece| total.checked_add(piece.get()))
}

fn set_delivered(event: &mut Event, amount: AssetAmount, over: bool) {
    if let Event::Asset(asset) = event {
        asset.delivered = amount;
        asset.over_delivered = over;
    }
}

/// Makes an exact copy of one event under a new identifier.
///
/// Content is carried over untouched, so a copy is a true retransmission.
fn clone_for_duplicate(event: &Event, id: EventId, deliver_at: Tick) -> Event {
    match event {
        Event::Control(control) => Event::Control(ControlEvent {
            id,
            source: control.source,
            destination: control.destination,
            intended_destination: control.intended_destination,
            bytes: control.bytes.clone(),
            message_id: control.message_id,
            deliver_at,
            attempts: 0,
            duplicate_of: Some(control.id),
            mutation: control.mutation,
            expires_at: control.expires_at,
            from_fault: true,
            status: EventStatus::Scheduled,
        }),
        Event::Asset(asset) => Event::Asset(AssetEvent {
            id,
            deliver_at,
            attempts: 0,
            duplicate_of: Some(asset.id),
            from_fault: true,
            status: EventStatus::Scheduled,
            ..*asset
        }),
    }
}

fn check_action_shape(action: &FaultAction) -> Result<(), SimError> {
    match action {
        FaultAction::Duplicate { copies, .. } if *copies == 0 => Err(
            SimError::InvalidConfiguration(ConfigProblem::NoDuplicateCopies),
        ),
        FaultAction::CorruptByte { mask, .. } if *mask == 0 => {
            Err(SimError::InvalidConfiguration(ConfigProblem::NoByteChange))
        }
        FaultAction::Partial { amount } | FaultAction::OverDeliver { amount }
            if amount.is_zero() =>
        {
            Err(SimError::ZeroAssetAmount)
        }
        FaultAction::Split { pieces, .. } => {
            if pieces.is_empty() || pieces.iter().any(|piece| piece.is_zero()) {
                return Err(SimError::InvalidPartialSplit);
            }
            sum_pieces(pieces).ok_or(SimError::ArithmeticOverflow)?;
            Ok(())
        }
        _ => Ok(()),
    }
}
