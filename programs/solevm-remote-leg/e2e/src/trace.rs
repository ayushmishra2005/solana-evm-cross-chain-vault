//! Concise typed records for one harness step.

use serde::Serialize;

use crate::hexutil;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TraceLog {
    pub events: Vec<TraceEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TraceEvent {
    pub timestamp: u64,
    pub tick: u64,
    pub component: String,
    pub event: String,
    pub direction: String,
    pub transfer_id: String,
    pub message_type: String,
    pub sequence: u64,
    pub message_id: String,
    pub amount: u64,
    pub accepted: bool,
    pub detail: String,
}

impl TraceLog {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, event: TraceEvent) {
        self.events.push(event);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        timestamp: u64,
        tick: u64,
        component: &str,
        event: &str,
        direction: &str,
        transfer_id: Option<&[u8; 32]>,
        message_type: &str,
        sequence: u64,
        message_id: Option<&[u8; 32]>,
        amount: u64,
        accepted: bool,
        detail: &str,
    ) {
        self.push(TraceEvent {
            timestamp,
            tick,
            component: String::from(component),
            event: String::from(event),
            direction: String::from(direction),
            transfer_id: transfer_id.map(hexutil::encode_array).unwrap_or_default(),
            message_type: String::from(message_type),
            sequence,
            message_id: message_id.map(hexutil::encode_array).unwrap_or_default(),
            amount,
            accepted,
            detail: String::from(detail),
        });
    }
}
