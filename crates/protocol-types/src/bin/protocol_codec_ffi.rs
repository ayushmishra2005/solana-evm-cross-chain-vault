//! Prints canonical message bytes so Foundry can compare Solidity encodings.
//!
//! The fixtures match `tests/common` in this crate. This binary is test
//! infrastructure, not a production encoder.

use std::io::{self, Write};
use std::process::ExitCode;

use protocol_types::{
    AllocateBody, ApplicationId, AssetAmount, Body, ChainId, Commitment, ConfigVersion,
    DeploymentId, DestinationReference, EpochId, Flags, Header, LaneId, Message, MessageType,
    PROTOCOL_VERSION, ProbeStatus, RecallBody, RecallSentBody, RemoteReportBody, SCHEMA_VERSION,
    Sequence, Timestamp, TransferId, VaultId, decode_message, encode_message, next_commitment,
};

const OBSERVED_AT: u64 = 1_700_000_000;
const PUBLISHED_AT: u64 = 1_700_000_060;
const EXPIRES_AT: u64 = 1_700_003_600;

const EVM_APPLICATION: [u8; 20] = [0xA1; 20];
const SOLANA_APPLICATION: [u8; 32] = [0xB2; 32];

fn main() -> ExitCode {
    let mut args = std::env::args();
    let _ = args.next();
    let Some(command) = args.next() else {
        return fail("missing command");
    };

    match command.as_str() {
        "encode-allocate" => write_hex(&encode_kind(MessageType::Allocate)),
        "encode-recall" => write_hex(&encode_kind(MessageType::Recall)),
        "encode-remote-report" => write_hex(&encode_kind(MessageType::RemoteReport)),
        "encode-recall-sent" => write_hex(&encode_kind(MessageType::RecallSent)),
        "ids-allocate" => write_ids(MessageType::Allocate),
        "ids-recall" => write_ids(MessageType::Recall),
        "ids-remote-report" => write_ids(MessageType::RemoteReport),
        "ids-recall-sent" => write_ids(MessageType::RecallSent),
        "decode-remote-report" => decode_kind(MessageType::RemoteReport),
        "decode-recall-sent" => decode_kind(MessageType::RecallSent),
        other => fail(&format!("unknown command {other}")),
    }
}

fn header() -> Header {
    Header {
        protocol_version: PROTOCOL_VERSION,
        schema_version: SCHEMA_VERSION,
        flags: Flags::NONE,
        source_chain: ChainId::new(1),
        destination_chain: ChainId::new(2),
        source_application: ApplicationId::from_evm_address(EVM_APPLICATION),
        destination_application: ApplicationId::from_solana_pubkey(SOLANA_APPLICATION),
        deployment_id: DeploymentId::new([0xD0; 32]),
        vault_id: VaultId::new([0x0E; 32]),
        lane_id: LaneId::new(7),
        sequence: Sequence::new(42),
        previous_commitment: Commitment::new([0xC1; 32]),
        observed_at: Timestamp::new(OBSERVED_AT),
        published_at: Timestamp::new(PUBLISHED_AT),
        expires_at: Timestamp::new(EXPIRES_AT),
    }
}

fn body_of(kind: MessageType) -> Body {
    match kind {
        MessageType::Allocate => Body::Allocate(AllocateBody {
            transfer_id: TransferId::new([0x11; 32]),
            amount: AssetAmount::new(1_000_000),
            expected_source_balance: AssetAmount::new(5_000_000),
            minimum_destination_amount: AssetAmount::new(999_000),
            deadline: Timestamp::new(PUBLISHED_AT + 900),
            config_version: ConfigVersion::new(4),
        }),
        MessageType::Recall => Body::Recall(RecallBody {
            transfer_id: TransferId::new([0x33; 32]),
            requested_amount: AssetAmount::new(750_000),
            minimum_return_amount: AssetAmount::new(740_000),
            deadline: Timestamp::new(PUBLISHED_AT + 1_800),
            config_version: ConfigVersion::new(9),
        }),
        MessageType::RemoteReport => Body::RemoteReport(RemoteReportBody {
            epoch_id: EpochId::new(12),
            remote_principal: AssetAmount::new(2_000_000),
            reported_value: AssetAmount::new(2_050_000),
            realized_loss: AssetAmount::new(1_500),
            unattributed_balance: AssetAmount::new(25),
            latest_completed_transfer_id: TransferId::new([0x77; 32]),
            probe_status: ProbeStatus::Fresh,
            probe_timestamp: Timestamp::new(OBSERVED_AT - 120),
            config_version: ConfigVersion::new(4),
            remote_state_commitment: Commitment::new([0x88; 32]),
        }),
        MessageType::RecallSent => Body::RecallSent(RecallSentBody {
            transfer_id: TransferId::new([0x99; 32]),
            principal_sent: AssetAmount::new(500_000),
            actual_amount_sent: AssetAmount::new(498_000),
            realized_loss: AssetAmount::new(2_000),
            destination_reference: DestinationReference::new([0xBB; 32]),
            sent_timestamp: Timestamp::new(OBSERVED_AT - 30),
            config_version: ConfigVersion::new(4),
        }),
        MessageType::ConfigUpdate => {
            let _ = writeln!(io::stderr(), "config update is not used by this helper");
            std::process::exit(1);
        }
    }
}

fn message(kind: MessageType) -> Message {
    Message {
        header: header(),
        body: body_of(kind),
    }
}

fn encode_kind(kind: MessageType) -> Vec<u8> {
    match encode_message(&message(kind)) {
        Ok(bytes) => bytes,
        Err(reason) => {
            let _ = writeln!(io::stderr(), "{reason}");
            std::process::exit(1);
        }
    }
}

fn write_ids(kind: MessageType) -> ExitCode {
    let msg = message(kind);
    let encoded = match encode_message(&msg) {
        Ok(bytes) => bytes,
        Err(reason) => return fail(&reason.to_string()),
    };
    let id = match msg.message_id() {
        Ok(value) => value,
        Err(reason) => return fail(&reason.to_string()),
    };
    let next = next_commitment(header().previous_commitment, id);
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(id.as_bytes());
    out.extend_from_slice(next.as_bytes());
    let _ = encoded;
    write_hex(&out)
}

fn decode_kind(kind: MessageType) -> ExitCode {
    let encoded = encode_kind(kind);
    match decode_message(&encoded) {
        Ok(_) => write_hex(&encoded),
        Err(reason) => fail(&reason.to_string()),
    }
}

fn write_hex(bytes: &[u8]) -> ExitCode {
    let mut stdout = io::stdout().lock();
    if stdout.write_all(b"0x").is_err() {
        return ExitCode::FAILURE;
    }
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        let hi = byte >> 4;
        let lo = byte & 15;
        let Some(&hi_ch) = TABLE.get(usize::from(hi)) else {
            return ExitCode::FAILURE;
        };
        let Some(&lo_ch) = TABLE.get(usize::from(lo)) else {
            return ExitCode::FAILURE;
        };
        if stdout.write_all(&[hi_ch, lo_ch]).is_err() {
            return ExitCode::FAILURE;
        }
    }
    if stdout.flush().is_err() {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn fail(message: &str) -> ExitCode {
    let _ = writeln!(io::stderr(), "{message}");
    ExitCode::FAILURE
}
