//! Canonical inbound messages, built as the EVM hub would publish them.
//!
//! No EVM chain takes part in the devnet run: the driver plays the hub and the
//! transport verifier signs what it delivers, exactly as a relayer would.

use protocol_types::{
    AllocateBody, ApplicationId, AssetAmount, Body, ChainId, Commitment, ConfigVersion,
    DeploymentId, Flags, Header, LaneId, Message, MessageId, PROTOCOL_VERSION, RecallBody,
    ResolveAction, ResolveBody, SCHEMA_VERSION, Sequence, Timestamp, TransferId, VaultId,
};

use crate::Res;

/// Local Anvil chain id: the hub is simulated, so no live EVM chain is named.
pub(crate) const SOURCE_CHAIN_ID: u32 = 31_337;
pub(crate) const DESTINATION_CHAIN_ID: u32 = 900;
pub(crate) const CONTROL_LANE_ID: u32 = 1;
pub(crate) const REPORT_LANE_ID: u32 = 2;
pub(crate) const CONFIG_VERSION: u64 = 1;
/// Informational only; the leg records it and never verifies it.
const EXPECTED_SOURCE_BALANCE: u64 = 5_000_000;
/// Long enough that RPC delays never let a message expire into quarantine.
const LIFETIME: u64 = 21_600;
/// Stamps sit in the past so cluster clock skew never makes a message early.
const CLOCK_MARGIN: u64 = 120;

/// The identities every inbound header must carry.
#[derive(Clone, Debug)]
pub(crate) struct Domain {
    pub source_application: [u8; 32],
    pub local_application: [u8; 32],
    pub deployment_id: [u8; 32],
    pub vault_id: [u8; 32],
}

/// Canonical bytes and the protocol message id.
#[derive(Clone, Debug)]
pub(crate) struct Encoded {
    pub bytes: Vec<u8>,
    pub id: [u8; 32],
}

fn header(domain: &Domain, sequence: u64, previous: [u8; 32], now: u64) -> Header {
    let stamped = now.saturating_sub(CLOCK_MARGIN);
    Header {
        protocol_version: PROTOCOL_VERSION,
        schema_version: SCHEMA_VERSION,
        flags: Flags::new(0),
        source_chain: ChainId::new(SOURCE_CHAIN_ID),
        destination_chain: ChainId::new(DESTINATION_CHAIN_ID),
        source_application: ApplicationId::new(domain.source_application),
        destination_application: ApplicationId::new(domain.local_application),
        deployment_id: DeploymentId::new(domain.deployment_id),
        vault_id: VaultId::new(domain.vault_id),
        lane_id: LaneId::new(CONTROL_LANE_ID),
        sequence: Sequence::new(sequence),
        previous_commitment: Commitment::new(previous),
        observed_at: Timestamp::new(stamped),
        published_at: Timestamp::new(stamped),
        expires_at: Timestamp::new(now.saturating_add(LIFETIME)),
    }
}

pub(crate) fn allocate(
    domain: &Domain,
    sequence: u64,
    previous: [u8; 32],
    now: u64,
    transfer_id: [u8; 32],
    amount: u64,
) -> Res<Encoded> {
    let body = AllocateBody {
        transfer_id: TransferId::new(transfer_id),
        amount: AssetAmount::new(u128::from(amount)),
        expected_source_balance: AssetAmount::new(u128::from(EXPECTED_SOURCE_BALANCE)),
        minimum_destination_amount: AssetAmount::new(u128::from(amount)),
        deadline: Timestamp::new(now.saturating_add(LIFETIME)),
        config_version: ConfigVersion::new(CONFIG_VERSION),
    };
    encode(Message {
        header: header(domain, sequence, previous, now),
        body: Body::Allocate(body),
    })
}

/// A recall of `amount` that accepts nothing less in return.
pub(crate) fn recall(
    domain: &Domain,
    sequence: u64,
    previous: [u8; 32],
    now: u64,
    transfer_id: [u8; 32],
    amount: u64,
) -> Res<Encoded> {
    let body = RecallBody {
        transfer_id: TransferId::new(transfer_id),
        requested_amount: AssetAmount::new(u128::from(amount)),
        minimum_return_amount: AssetAmount::new(u128::from(amount)),
        deadline: Timestamp::new(now.saturating_add(LIFETIME)),
        config_version: ConfigVersion::new(CONFIG_VERSION),
    };
    encode(Message {
        header: header(domain, sequence, previous, now),
        body: Body::Recall(body),
    })
}

pub(crate) fn resolve(
    domain: &Domain,
    sequence: u64,
    previous: [u8; 32],
    now: u64,
    transfer_id: [u8; 32],
    target_sequence: u64,
    action: ResolveAction,
) -> Res<Encoded> {
    let body = ResolveBody {
        transfer_id: TransferId::new(transfer_id),
        target_sequence: Sequence::new(target_sequence),
        action,
        config_version: ConfigVersion::new(CONFIG_VERSION),
    };
    encode(Message {
        header: header(domain, sequence, previous, now),
        body: Body::Resolve(body),
    })
}

fn encode(message: Message) -> Res<Encoded> {
    let encoded = message
        .encode()
        .map_err(|error| format!("encode: {error:?}"))?;
    Ok(Encoded {
        bytes: encoded.as_bytes().to_vec(),
        id: encoded.message_id().to_bytes(),
    })
}

/// The lane commitment after `id` is accepted on top of `previous`.
pub(crate) fn next_commitment(previous: [u8; 32], id: [u8; 32]) -> [u8; 32] {
    protocol_types::next_commitment(Commitment::new(previous), MessageId::new(id)).to_bytes()
}
