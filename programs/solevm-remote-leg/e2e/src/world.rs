//! Live LiteSVM remote-leg plus independent xchain-sim transport.

use protocol_types::{AssetAmount, Body, MessageType, ResolveAction, TransferId};
use serde::Serialize;
use solevm_remote_leg::{RecallSentPublished, RemoteReportPublished, TransportSettlementPublished};
use xchain_sim::{
    AssetRequest, ControlRequest, EndpointId, EventId, Fault, FaultAction, FaultId, FaultTarget,
    Simulator, Tick,
};

use crate::common::messages::MessageBuilder;
use crate::common::{CONTROL_LANE_ID, Fixture, TxResult, last_event};
use crate::hexutil;
use crate::ops::{Script, WireOp};
use crate::trace::TraceLog;

const HUB: EndpointId = EndpointId::new(1);
const LEG: EndpointId = EndpointId::new(2);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Observation {
    pub timestamp: u64,
    pub tick: u64,
    pub principal: u64,
    pub deployed: u64,
    pub custody: u64,
    pub unattributed: u64,
    pub recalled: u64,
    pub realized_loss: u64,
    pub lane_sequence: u64,
    pub lane_commitment: String,
    pub outbound_next: u64,
    pub outbound_commitment: String,
    pub active_transfer: String,
    pub transfer_open: bool,
    pub frozen: bool,
    pub last_accepted: bool,
    pub last_error: String,
    pub last_event_id: u64,
    pub inbound_asset: u128,
    pub impaired_outstanding: u128,
    pub reports: Vec<String>,
    pub recall_sents: Vec<String>,
    pub settlements: Vec<String>,
    pub latest_report: String,
    pub latest_recall_sent: String,
    pub latest_settlement: String,
    pub trace: TraceLog,
}

pub struct World {
    fixture: Fixture,
    sim: Simulator,
    hub: EndpointId,
    leg: EndpointId,
    control_applied: usize,
    asset_applied: usize,
    next_fault: u32,
    last_accepted: bool,
    last_error: String,
    last_event_id: u64,
    trace: TraceLog,
}

impl World {
    #[must_use]
    pub fn new() -> Self {
        let sim = Simulator::new(&[HUB, LEG]).expect("simulator starts");
        Self {
            fixture: Fixture::e2e(),
            sim,
            hub: HUB,
            leg: LEG,
            control_applied: 0,
            asset_applied: 0,
            next_fault: 1,
            last_accepted: true,
            last_error: String::new(),
            last_event_id: 0,
            trace: TraceLog::new(),
        }
    }

    pub fn run_json(input: &str) -> Result<Observation, String> {
        let script: Script = serde_json::from_str(input).map_err(|error| error.to_string())?;
        let mut world = Self::new();
        world.apply_all(&script.ops)?;
        Ok(world.observe())
    }

    pub fn apply_all(&mut self, ops: &[WireOp]) -> Result<(), String> {
        for op in ops {
            self.apply(op)?;
        }
        Ok(())
    }

    pub fn apply(&mut self, op: &WireOp) -> Result<(), String> {
        match op {
            WireOp::Warp { ts } => {
                let stamped = i64::try_from(*ts).map_err(|_| String::from("timestamp overflow"))?;
                self.fixture.set_time(stamped);
                self.note("solana", "warp", "", None, "", 0, None, 0, true, "");
            }
            WireOp::ScheduleControl { bytes, at } => {
                let payload = hexutil::decode(bytes)?;
                self.schedule_control(self.hub, self.leg, payload, *at)?;
            }
            WireOp::ScheduleAsset {
                transfer,
                amount,
                at,
            } => {
                let transfer = hexutil::decode32(transfer)?;
                self.schedule_asset(self.hub, self.leg, transfer, *amount, *at)?;
            }
            WireOp::FaultDrop { event } => {
                self.add_fault(*event, FaultAction::Drop)?;
            }
            WireOp::FaultDelay { event, ticks } => {
                self.add_fault(*event, FaultAction::Delay { ticks: *ticks })?;
            }
            WireOp::FaultDuplicate {
                event,
                copies,
                spacing,
            } => {
                self.add_fault(
                    *event,
                    FaultAction::Duplicate {
                        copies: *copies,
                        spacing: *spacing,
                    },
                )?;
            }
            WireOp::FaultPartial { event, amount } => {
                self.add_fault(
                    *event,
                    FaultAction::Partial {
                        amount: AssetAmount::new(u128::from(*amount)),
                    },
                )?;
            }
            WireOp::FaultSplit {
                event,
                pieces,
                spacing,
            } => {
                let pieces = pieces
                    .iter()
                    .map(|piece| AssetAmount::new(u128::from(*piece)))
                    .collect();
                self.add_fault(
                    *event,
                    FaultAction::Split {
                        pieces,
                        spacing: *spacing,
                    },
                )?;
            }
            WireOp::Swap { left, right } => {
                self.sim
                    .swap_delivery_ticks(EventId::new(*left), EventId::new(*right))
                    .map_err(|error| error.to_string())?;
                self.note("transport", "swap", "", None, "", 0, None, 0, true, "");
            }
            WireOp::AdvanceTo { tick } => {
                self.sim
                    .advance_to(Tick::new(*tick))
                    .map_err(|error| error.to_string())?;
            }
            WireOp::DeliverReady => {
                self.sim.deliver_ready();
                self.note("transport", "deliver", "", None, "", 0, None, 0, true, "");
            }
            WireOp::RunUntilIdle => {
                self.sim.run_until_idle();
                self.note("transport", "idle", "", None, "", 0, None, 0, true, "");
            }
            WireOp::ApplyControl => self.apply_control(),
            WireOp::ApplyAsset => self.apply_asset(),
            WireOp::Attribute { transfer } => {
                let transfer = hexutil::decode32(transfer)?;
                let result = self.fixture.attribute(transfer);
                self.finish_tx(result, "attribute", Some(&transfer), 0);
            }
            WireOp::Deploy { amount } => {
                let result = self.fixture.deploy(*amount);
                self.finish_tx(result, "deploy", None, *amount);
            }
            WireOp::ConfigureAdapter {
                max_liquid,
                loss_bps,
            } => {
                let result = self
                    .fixture
                    .configure_adapter(*max_liquid, *loss_bps, false);
                self.finish_tx(result, "configure_adapter", None, 0);
            }
            WireOp::Withdraw { transfer, amount } => {
                let transfer = hexutil::decode32(transfer)?;
                let result = self.fixture.withdraw(transfer, *amount);
                self.finish_tx(result, "withdraw", Some(&transfer), *amount);
            }
            WireOp::SendRecall {
                transfer,
                amount,
                at,
            } => {
                let transfer = hexutil::decode32(transfer)?;
                self.send_recall(transfer, *amount, *at);
            }
            WireOp::PublishReport { epoch, at } => self.publish_report(*epoch, *at),
            WireOp::PublishRecallSent { transfer, at } => {
                let transfer = hexutil::decode32(transfer)?;
                self.publish_recall_sent(transfer, *at);
            }
            WireOp::Freeze => {
                let administrator = self.fixture.administrator.insecure_clone();
                let result = self.fixture.freeze(&administrator);
                self.finish_tx(result, "freeze", None, 0);
            }
            WireOp::ConfigUpdate { bytes } => {
                let payload = hexutil::decode(bytes)?;
                self.schedule_control(self.hub, self.leg, payload, self.sim.now().get())?;
            }
            WireOp::ConfigUpdateNext => self.schedule_config_update(None, None)?,
            WireOp::ConfigUpdateWrongLane => {
                self.schedule_config_update(None, Some(CONTROL_LANE_ID.saturating_add(1)))?
            }
            WireOp::ConfigUpdateReuseSequence => self.schedule_config_update(Some(1), None)?,
            WireOp::AllocateNext { transfer, amount } => {
                let transfer = hexutil::decode32(transfer)?;
                self.schedule_allocate_next(transfer, *amount)?;
            }
        }
        Ok(())
    }

    fn schedule_config_update(
        &mut self,
        sequence: Option<u64>,
        lane: Option<u32>,
    ) -> Result<(), String> {
        let lane_state = self.fixture.control_lane();
        let sequence = sequence.unwrap_or(lane_state.highest_consumed_sequence.saturating_add(1));
        let previous = if sequence == 1 {
            [0u8; 32]
        } else {
            lane_state.message_commitment
        };
        let mut builder = MessageBuilder::config_update()
            .sequence(sequence)
            .previous_commitment(previous);
        if let Some(lane_id) = lane {
            builder = builder.lane(lane_id);
        }
        let bytes = builder.encode();
        self.schedule_control(self.hub, self.leg, bytes, self.sim.now().get())
    }

    fn schedule_allocate_next(&mut self, transfer: [u8; 32], amount: u64) -> Result<(), String> {
        let lane = self.fixture.control_lane();
        let sequence = lane.highest_consumed_sequence.saturating_add(1);
        let bytes = MessageBuilder::allocate()
            .sequence(sequence)
            .previous_commitment(lane.message_commitment)
            .transfer_id(transfer)
            .allocate_body(|body| {
                body.amount = protocol_types::AssetAmount::new(u128::from(amount));
                body.minimum_destination_amount =
                    protocol_types::AssetAmount::new(u128::from(amount));
                body.config_version =
                    protocol_types::ConfigVersion::new(self.fixture.config().config_version);
            })
            .encode();
        self.schedule_control(self.hub, self.leg, bytes, self.sim.now().get())
    }

    fn schedule_control(
        &mut self,
        source: EndpointId,
        destination: EndpointId,
        bytes: Vec<u8>,
        at: u64,
    ) -> Result<(), String> {
        let decoded = protocol_types::decode_message(&bytes).ok();
        let message_type = decoded
            .as_ref()
            .map(|message| format!("{:?}", message.message_type()))
            .unwrap_or_default();
        let sequence = decoded
            .as_ref()
            .map(|message| message.header.sequence.get())
            .unwrap_or(0);
        let transfer = transfer_of(decoded.as_ref());
        let id = self
            .sim
            .schedule_control(ControlRequest::new(
                source,
                destination,
                bytes,
                Tick::new(at.max(self.sim.now().get())),
            ))
            .map_err(|error| error.to_string())?;
        self.last_event_id = id.get();
        self.note(
            "transport",
            "schedule_control",
            direction(source),
            transfer.as_ref(),
            &message_type,
            sequence,
            None,
            0,
            true,
            "",
        );
        Ok(())
    }

    fn schedule_asset(
        &mut self,
        source: EndpointId,
        destination: EndpointId,
        transfer: [u8; 32],
        amount: u64,
        at: u64,
    ) -> Result<(), String> {
        if amount == 0 {
            return Ok(());
        }
        let id = self
            .sim
            .schedule_asset(AssetRequest::new(
                TransferId::new(transfer),
                source,
                destination,
                AssetAmount::new(u128::from(amount)),
                Tick::new(at.max(self.sim.now().get())),
            ))
            .map_err(|error| error.to_string())?;
        self.last_event_id = id.get();
        self.note(
            "transport",
            "schedule_asset",
            direction(source),
            Some(&transfer),
            "",
            0,
            None,
            amount,
            true,
            "",
        );
        Ok(())
    }

    fn add_fault(&mut self, event: u64, action: FaultAction) -> Result<(), String> {
        let id = FaultId::new(self.next_fault);
        self.next_fault = self.next_fault.saturating_add(1);
        self.sim
            .add_fault(Fault::new(
                id,
                FaultTarget::Event(EventId::new(event)),
                action,
            ))
            .map_err(|error| error.to_string())?;
        self.note("transport", "fault", "", None, "", 0, None, 0, true, "");
        Ok(())
    }

    fn apply_control(&mut self) {
        let inbox = self.sim.control_inbox(self.leg).to_vec();
        let start = self.control_applied;
        for item in inbox.iter().skip(start) {
            self.dispatch_control(&item.bytes);
        }
        self.control_applied = inbox.len();
    }

    fn dispatch_control(&mut self, bytes: &[u8]) {
        let decoded = match protocol_types::decode_message(bytes) {
            Ok(message) => message,
            Err(error) => {
                self.last_accepted = false;
                self.last_error = format!("{error:?}");
                let detail = self.last_error.clone();
                self.note(
                    "solana",
                    "decode",
                    "evm_to_solana",
                    None,
                    "",
                    0,
                    None,
                    0,
                    false,
                    &detail,
                );
                return;
            }
        };
        let sequence = decoded.header.sequence.get();
        let message_type = format!("{:?}", decoded.message_type());
        let transfer = transfer_of(Some(&decoded));
        let result = match &decoded.body {
            Body::Allocate(body) => {
                self.fixture
                    .allocate(body.transfer_id.to_bytes(), sequence, bytes.to_vec())
            }
            Body::Recall(body) => {
                self.fixture
                    .recall(body.transfer_id.to_bytes(), sequence, bytes.to_vec())
            }
            Body::ConfigUpdate(_) => self.fixture.config_update(sequence, bytes.to_vec()),
            Body::Resolve(body) if body.action == ResolveAction::Admit => {
                self.fixture
                    .resolve(body.transfer_id.to_bytes(), sequence, bytes.to_vec())
            }
            Body::Resolve(body) => {
                let result =
                    self.fixture
                        .settle(body.transfer_id.to_bytes(), sequence, bytes.to_vec());
                if self.record_tx(
                    result,
                    "settle",
                    transfer.as_ref(),
                    &message_type,
                    sequence,
                    0,
                ) {
                    let event: TransportSettlementPublished = last_event(&self.fixture.last_logs());
                    let at = self.sim.now().get();
                    let _ = self.schedule_control(self.leg, self.hub, event.message_bytes, at);
                }
                return;
            }
            _ => {
                self.last_accepted = false;
                self.last_error = format!("unexpected {message_type}");
                let detail = self.last_error.clone();
                self.note(
                    "solana",
                    "reject",
                    "evm_to_solana",
                    transfer.as_ref(),
                    &message_type,
                    sequence,
                    None,
                    0,
                    false,
                    &detail,
                );
                return;
            }
        };
        self.record_tx(
            result,
            "control",
            transfer.as_ref(),
            &message_type,
            sequence,
            0,
        );
    }

    fn apply_asset(&mut self) {
        let inbox = self.sim.asset_inbox(self.leg).to_vec();
        let start = self.asset_applied;
        for item in inbox.iter().skip(start) {
            let amount = u64::try_from(item.delivered.get()).unwrap_or(u64::MAX);
            self.fixture.credit(self.fixture.custody, amount);
            self.note(
                "solana",
                "credit",
                "asset",
                Some(item.transfer.as_bytes()),
                "",
                0,
                None,
                amount,
                true,
                "",
            );
            self.last_accepted = true;
            self.last_error.clear();
        }
        self.asset_applied = inbox.len();
    }

    fn send_recall(&mut self, transfer: [u8; 32], amount: u64, at: u64) {
        let before = self.fixture.token_amount(self.fixture.escrow);
        let result = self.fixture.send_recall(transfer, amount);
        self.finish_tx(result, "send", Some(&transfer), amount);
        if !self.last_accepted {
            return;
        }
        let after = self.fixture.token_amount(self.fixture.escrow);
        let sent = after.saturating_sub(before);
        if let Err(error) = self.schedule_asset(self.leg, self.hub, transfer, sent, at) {
            self.last_accepted = false;
            self.last_error = error;
        }
    }

    fn publish_report(&mut self, epoch: u64, at: u64) {
        let result = self.fixture.publish_report(epoch);
        if !self.record_tx(result, "publish_report", None, "RemoteReport", 0, 0) {
            return;
        }
        let event: RemoteReportPublished = last_event(&self.fixture.last_logs());
        let _ = self.schedule_control(self.leg, self.hub, event.message_bytes, at);
    }

    fn publish_recall_sent(&mut self, transfer: [u8; 32], at: u64) {
        let result = self.fixture.publish_recall_sent(transfer);
        if !self.record_tx(
            result,
            "publish_recall_sent",
            Some(&transfer),
            "RecallSent",
            0,
            0,
        ) {
            return;
        }
        let event: RecallSentPublished = last_event(&self.fixture.last_logs());
        let _ = self.schedule_control(self.leg, self.hub, event.message_bytes, at);
    }

    fn finish_tx(
        &mut self,
        result: TxResult,
        event: &str,
        transfer: Option<&[u8; 32]>,
        amount: u64,
    ) {
        self.record_tx(result, event, transfer, "", 0, amount);
    }

    fn record_tx(
        &mut self,
        result: TxResult,
        event: &str,
        transfer: Option<&[u8; 32]>,
        message_type: &str,
        sequence: u64,
        amount: u64,
    ) -> bool {
        match result {
            Ok(()) => {
                self.last_accepted = true;
                self.last_error.clear();
                self.note(
                    "solana",
                    event,
                    "evm_to_solana",
                    transfer,
                    message_type,
                    sequence,
                    None,
                    amount,
                    true,
                    "",
                );
                true
            }
            Err(failure) => {
                self.last_accepted = false;
                self.last_error = format!("{:?}", failure.err);
                let detail = self.last_error.clone();
                self.note(
                    "solana",
                    event,
                    "evm_to_solana",
                    transfer,
                    message_type,
                    sequence,
                    None,
                    amount,
                    false,
                    &detail,
                );
                false
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn note(
        &mut self,
        component: &str,
        event: &str,
        direction: &str,
        transfer: Option<&[u8; 32]>,
        message_type: &str,
        sequence: u64,
        message_id: Option<&[u8; 32]>,
        amount: u64,
        accepted: bool,
        detail: &str,
    ) {
        self.trace.record(
            self.now(),
            self.sim.now().get(),
            component,
            event,
            direction,
            transfer,
            message_type,
            sequence,
            message_id,
            amount,
            accepted,
            detail,
        );
    }

    fn now(&self) -> u64 {
        u64::try_from(
            self.fixture
                .svm
                .get_sysvar::<anchor_lang::solana_program::clock::Clock>()
                .unix_timestamp
                .max(0),
        )
        .unwrap_or(0)
    }

    #[must_use]
    pub const fn last_event_id(&self) -> u64 {
        self.last_event_id
    }

    #[must_use]
    pub fn observe(&self) -> Observation {
        let position = self.fixture.position();
        let lane = self.fixture.control_lane();
        let outbound = self.fixture.outbound_lane();
        let mut reports = Vec::new();
        let mut recall_sents = Vec::new();
        let mut settlements = Vec::new();
        for item in self.sim.control_inbox(self.hub) {
            if let Ok(message) = protocol_types::decode_message(&item.bytes) {
                match message.message_type() {
                    MessageType::RemoteReport => reports.push(hexutil::encode(&item.bytes)),
                    MessageType::RecallSent => recall_sents.push(hexutil::encode(&item.bytes)),
                    MessageType::TransportSettlement => {
                        settlements.push(hexutil::encode(&item.bytes));
                    }
                    _ => {}
                }
            }
        }
        let inbound_asset = self
            .sim
            .asset_inbox(self.hub)
            .iter()
            .fold(0u128, |total, item| {
                total.saturating_add(item.delivered.get())
            });
        Observation {
            timestamp: self.now(),
            tick: self.sim.now().get(),
            principal: position.attributed_principal,
            deployed: position.deployed_principal,
            custody: self.fixture.token_amount(self.fixture.custody),
            unattributed: position.unattributed_custody,
            recalled: position.recalled_custody,
            realized_loss: position.cumulative_realized_loss,
            lane_sequence: lane.highest_consumed_sequence,
            lane_commitment: hexutil::encode_array(&lane.message_commitment),
            outbound_next: outbound.next_sequence,
            outbound_commitment: hexutil::encode_array(&outbound.current_commitment),
            active_transfer: hexutil::encode_array(&position.active_transfer_id),
            transfer_open: position.has_active_transfer(),
            frozen: self.fixture.config().frozen,
            last_accepted: self.last_accepted,
            last_error: self.last_error.clone(),
            last_event_id: self.last_event_id,
            inbound_asset,
            impaired_outstanding: self
                .fixture
                .transport_ledger()
                .map_or(0, |ledger| ledger.impaired_outstanding),
            latest_report: reports.last().cloned().unwrap_or_default(),
            latest_recall_sent: recall_sents.last().cloned().unwrap_or_default(),
            latest_settlement: settlements.last().cloned().unwrap_or_default(),
            reports,
            recall_sents,
            settlements,
            trace: self.trace.clone(),
        }
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for World {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("World").finish_non_exhaustive()
    }
}

fn transfer_of(message: Option<&protocol_types::Message>) -> Option<[u8; 32]> {
    match message.map(|item| &item.body) {
        Some(Body::Allocate(body)) => Some(body.transfer_id.to_bytes()),
        Some(Body::Recall(body)) => Some(body.transfer_id.to_bytes()),
        Some(Body::RecallSent(body)) => Some(body.transfer_id.to_bytes()),
        Some(Body::Resolve(body)) => Some(body.transfer_id.to_bytes()),
        Some(Body::TransportSettlement(body)) => Some(body.transfer_id.to_bytes()),
        _ => None,
    }
}

fn direction(source: EndpointId) -> &'static str {
    if source == HUB {
        "evm_to_solana"
    } else {
        "solana_to_evm"
    }
}
