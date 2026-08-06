//! Canonical messages built with the shared protocol crate.

#![allow(dead_code, unreachable_pub)]
#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

use protocol_types::{
    AllocateBody, ApplicationId, AssetAmount, BasisPoints, Body, ChainId, Commitment,
    ConfigUpdateBody, ConfigVersion, DeploymentId, Flags, Header, LaneId, Message,
    PROTOCOL_VERSION, RecallBody, SCHEMA_VERSION, Sequence, Timestamp, TransferId, VaultId,
};

use super::{
    CONFIG_VERSION, CONTROL_LANE_ID, DEPLOYMENT_ID, DESTINATION_CHAIN_ID, LOCAL_APPLICATION_ID,
    SOURCE_APPLICATION_ID, SOURCE_CHAIN_ID, START_TIMESTAMP, VAULT_ID,
};

pub const OBSERVED_AT: u64 = START_TIMESTAMP as u64 - 200;
pub const PUBLISHED_AT: u64 = START_TIMESTAMP as u64 - 100;
pub const EFFECTIVE_AT: u64 = START_TIMESTAMP as u64 - 50;
pub const EXPIRES_AT: u64 = START_TIMESTAMP as u64 + 100_000;

/// A canonical message under construction, with the domain already filled in.
#[derive(Clone, Debug)]
pub struct MessageBuilder {
    pub header: Header,
    pub body: Body,
}

impl MessageBuilder {
    /// A valid config update that moves the leg to the next version.
    pub fn config_update() -> Self {
        Self {
            header: default_header(),
            body: Body::ConfigUpdate(default_config_body()),
        }
    }

    pub fn allocate() -> Self {
        Self {
            header: default_header(),
            body: Body::Allocate(AllocateBody {
                transfer_id: TransferId::new([0x77; 32]),
                amount: AssetAmount::new(1_000_000),
                expected_source_balance: AssetAmount::new(5_000_000),
                minimum_destination_amount: AssetAmount::new(999_000),
                deadline: Timestamp::new(EXPIRES_AT),
                config_version: ConfigVersion::new(CONFIG_VERSION),
            }),
        }
    }

    pub fn recall() -> Self {
        Self {
            header: default_header(),
            body: Body::Recall(RecallBody {
                transfer_id: TransferId::new([0x88; 32]),
                requested_amount: AssetAmount::new(2_000_000),
                minimum_return_amount: AssetAmount::new(1_900_000),
                deadline: Timestamp::new(EXPIRES_AT),
                config_version: ConfigVersion::new(CONFIG_VERSION),
            }),
        }
    }

    pub fn sequence(mut self, sequence: u64) -> Self {
        self.header.sequence = Sequence::new(sequence);
        self
    }

    pub fn previous_commitment(mut self, commitment: [u8; 32]) -> Self {
        self.header.previous_commitment = Commitment::new(commitment);
        self
    }

    pub fn source_chain(mut self, chain: u32) -> Self {
        self.header.source_chain = ChainId::new(chain);
        self
    }

    pub fn destination_chain(mut self, chain: u32) -> Self {
        self.header.destination_chain = ChainId::new(chain);
        self
    }

    pub fn source_application(mut self, application: [u8; 32]) -> Self {
        self.header.source_application = ApplicationId::new(application);
        self
    }

    pub fn destination_application(mut self, application: [u8; 32]) -> Self {
        self.header.destination_application = ApplicationId::new(application);
        self
    }

    pub fn deployment(mut self, deployment: [u8; 32]) -> Self {
        self.header.deployment_id = DeploymentId::new(deployment);
        self
    }

    pub fn vault(mut self, vault: [u8; 32]) -> Self {
        self.header.vault_id = VaultId::new(vault);
        self
    }

    pub fn lane(mut self, lane: u32) -> Self {
        self.header.lane_id = LaneId::new(lane);
        self
    }

    pub fn expires_at(mut self, timestamp: u64) -> Self {
        self.header.expires_at = Timestamp::new(timestamp);
        self
    }

    pub fn published_at(mut self, timestamp: u64) -> Self {
        self.header.published_at = Timestamp::new(timestamp);
        self
    }

    pub fn observed_at(mut self, timestamp: u64) -> Self {
        self.header.observed_at = Timestamp::new(timestamp);
        self
    }

    /// Edits the config body in place, for tests that bend one field.
    pub fn config_body(mut self, edit: impl FnOnce(&mut ConfigUpdateBody)) -> Self {
        let Body::ConfigUpdate(mut body) = self.body else {
            panic!("builder does not hold a config update body");
        };
        edit(&mut body);
        self.body = Body::ConfigUpdate(body);
        self
    }

    /// Edits the allocate body in place, for tests that bend one field.
    pub fn allocate_body(mut self, edit: impl FnOnce(&mut AllocateBody)) -> Self {
        let Body::Allocate(mut body) = self.body else {
            panic!("builder does not hold an allocate body");
        };
        edit(&mut body);
        self.body = Body::Allocate(body);
        self
    }

    /// Edits the recall body in place, for tests that bend one field.
    pub fn recall_body(mut self, edit: impl FnOnce(&mut RecallBody)) -> Self {
        let Body::Recall(mut body) = self.body else {
            panic!("builder does not hold a recall body");
        };
        edit(&mut body);
        self.body = Body::Recall(body);
        self
    }

    pub fn transfer_id(self, transfer_id: [u8; 32]) -> Self {
        match self.body {
            Body::Allocate(_) => {
                self.allocate_body(|body| body.transfer_id = TransferId::new(transfer_id))
            }
            Body::Recall(_) => {
                self.recall_body(|body| body.transfer_id = TransferId::new(transfer_id))
            }
            _ => panic!("builder does not hold a transfer body"),
        }
    }

    pub fn build(self) -> Message {
        Message {
            header: self.header,
            body: self.body,
        }
    }

    /// Canonical bytes, as the transport would deliver them.
    pub fn encode(self) -> Vec<u8> {
        self.build()
            .encode()
            .expect("message encodes")
            .as_bytes()
            .to_vec()
    }

    /// Canonical bytes together with the id the protocol assigns them.
    pub fn encode_with_id(self) -> (Vec<u8>, [u8; 32]) {
        let encoded = self.build().encode().expect("message encodes");
        (encoded.as_bytes().to_vec(), encoded.message_id().to_bytes())
    }
}

fn default_header() -> Header {
    Header {
        protocol_version: PROTOCOL_VERSION,
        schema_version: SCHEMA_VERSION,
        flags: Flags::new(0),
        source_chain: ChainId::new(SOURCE_CHAIN_ID),
        destination_chain: ChainId::new(DESTINATION_CHAIN_ID),
        source_application: ApplicationId::new(SOURCE_APPLICATION_ID),
        destination_application: ApplicationId::new(LOCAL_APPLICATION_ID),
        deployment_id: DeploymentId::new(DEPLOYMENT_ID),
        vault_id: VaultId::new(VAULT_ID),
        lane_id: LaneId::new(CONTROL_LANE_ID),
        sequence: Sequence::new(1),
        previous_commitment: Commitment::ZERO,
        observed_at: Timestamp::new(OBSERVED_AT),
        published_at: Timestamp::new(PUBLISHED_AT),
        expires_at: Timestamp::new(EXPIRES_AT),
    }
}

fn default_config_body() -> ConfigUpdateBody {
    ConfigUpdateBody {
        config_version: ConfigVersion::new(CONFIG_VERSION + 1),
        previous_config_version: ConfigVersion::new(CONFIG_VERSION),
        max_remote_allocation_bps: BasisPoints::new(6_000),
        max_upward_deviation_bps: BasisPoints::new(200),
        max_downward_deviation_bps: BasisPoints::new(1_000),
        max_report_age: 3_600,
        effective_timestamp: Timestamp::new(EFFECTIVE_AT),
        config_commitment: Commitment::new([0xCC; 32]),
    }
}

/// Overwrites bytes at an absolute offset, leaving the body hash alone.
pub fn patch(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

/// Overwrites body bytes and rewrites the declared body hash to match.
///
/// This produces a structurally sound frame that still breaks a field rule.
pub fn patch_body(bytes: &mut [u8], offset: usize, value: &[u8]) {
    let start = protocol_types::layout::HEADER_LEN + offset;
    bytes[start..start + value.len()].copy_from_slice(value);

    let hash = protocol_types::keccak256(&bytes[protocol_types::layout::HEADER_LEN..]);
    patch(bytes, protocol_types::layout::BODY_HASH_OFFSET, &hash);
}

/// The commitment a lane holds after one message is accepted.
pub fn next_commitment(previous: [u8; 32], message_id: [u8; 32]) -> [u8; 32] {
    protocol_types::next_commitment(
        Commitment::new(previous),
        protocol_types::MessageId::new(message_id),
    )
    .to_bytes()
}
