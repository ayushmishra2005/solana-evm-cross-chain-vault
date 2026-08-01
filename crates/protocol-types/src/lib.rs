//! Canonical wire format for SolEVM Vault control messages.
//!
//! Every message is a fixed width envelope followed by a fixed width body.
//! All multi byte integers are big endian, so the same bytes can be decoded
//! by other runtimes without an ABI.

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

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod body;
pub mod layout;

mod codec;
mod commitment;
mod error;
mod hash;
mod identifier;
mod message;
mod validation;

pub use body::{
    AllocateBody, Body, ConfigUpdateBody, ProbeStatus, RecallBody, RecallSentBody, RemoteReportBody,
};
pub use codec::{EncodedMessage, decode_message, encode_into};
pub use commitment::{COMMITMENT_DOMAIN, MESSAGE_ID_DOMAIN, next_commitment};
pub use error::{AmountField, DecodeError, EncodeError, IdentifierField, ValidationError};
pub use hash::keccak256;
pub use identifier::{
    ApplicationId, AssetAmount, BasisPoints, BodyHash, ChainId, Commitment, ConfigVersion,
    DeploymentId, DestinationReference, EpochId, Flags, LaneId, MAX_BASIS_POINTS, MessageId,
    ProtocolVersion, SchemaVersion, Sequence, Timestamp, TransferId, VaultId,
};
pub use message::{Header, Message, MessageType};

#[cfg(feature = "alloc")]
pub use codec::encode_message;

/// Magic prefix of every canonical message.
pub const MAGIC: [u8; 4] = *b"SVE1";

/// The only protocol version this build accepts.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::new(1);

/// The only schema version this build accepts.
pub const SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1);
