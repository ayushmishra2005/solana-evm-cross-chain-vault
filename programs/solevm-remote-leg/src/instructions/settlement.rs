//! Applies one explicit close or one pooled recovery, and publishes its
//! transport settlement in the same instruction.
//!
//! Publishing here puts the settlement ahead of every later report on the
//! outbound lane, so the vault never applies a report that already counts it.
//! The vault holds its cycle open until this settlement arrives, so a late
//! resolution still binds; skipping it would strand that cycle.

use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;
use protocol_types::{ResolveAction, ResolveBody, SettlementKind};

use crate::control::{
    CONSUMED_MESSAGE_SEED, ConsumedMessage, MessageClass, REPLAY_LANE_SEED, RISK_CONFIG_SEED,
    ReplayLane, RiskConfig,
};
use crate::custody;
use crate::errors::RemoteLegError;
use crate::events::{SettlementProcessed, TransportSettlementPublished};
use crate::instructions::allocate::{ConsumedAddress, TransferAddress};
use crate::instructions::publication::{self, RecordEntry, RecordSlots};
use crate::message::{self, ValidatedMessage};
use crate::outbound::{OUTBOUND_LANE_SEED, OutboundLane, ReportType};
use crate::outbound_message::{self, OutboundContext, TransportSettlementFields};
use crate::records;
use crate::state::{REMOTE_CONFIG_SEED, RemoteConfig, STATE_VERSION};
use crate::strategy::{
    REMOTE_POSITION_SEED, RemotePosition, STRATEGY_CONFIG_SEED, StrategyConfig,
    TRANSFER_RECORD_SEED, TransferKind, TransferRecord, TransferStatus,
};
use crate::transport::{TRANSPORT_LEDGER_SEED, TransportLedger};

const CLASS: MessageClass = MessageClass::Resolve;

#[derive(Accounts)]
pub struct ProcessSettlement<'info> {
    #[account(mut)]
    pub transport_verifier: Signer<'info>,

    #[account(
        seeds = [
            REMOTE_CONFIG_SEED,
            &remote_config.deployment_id,
            &remote_config.vault_id,
        ],
        bump = remote_config.bump,
        has_one = transport_verifier @ RemoteLegError::Unauthorized,
        has_one = custody_token_account @ RemoteLegError::InvalidCustodyAccount,
    )]
    pub remote_config: Box<Account<'info, RemoteConfig>>,

    #[account(
        seeds = [RISK_CONFIG_SEED, remote_config.key().as_ref()],
        bump = risk_config.bump,
    )]
    pub risk_config: Box<Account<'info, RiskConfig>>,

    #[account(
        seeds = [STRATEGY_CONFIG_SEED, remote_config.key().as_ref()],
        bump = strategy_config.bump,
    )]
    pub strategy_config: Box<Account<'info, StrategyConfig>>,

    #[account(
        mut,
        seeds = [REMOTE_POSITION_SEED, remote_config.key().as_ref()],
        bump = remote_position.bump,
    )]
    pub remote_position: Box<Account<'info, RemotePosition>>,

    #[account(
        mut,
        seeds = [
            REPLAY_LANE_SEED,
            remote_config.key().as_ref(),
            &remote_config.control_lane_id.to_le_bytes(),
        ],
        bump = control_lane.bump,
    )]
    pub control_lane: Box<Account<'info, ReplayLane>>,

    #[account(
        mut,
        seeds = [
            OUTBOUND_LANE_SEED,
            remote_config.key().as_ref(),
            &remote_config.report_lane_id.to_le_bytes(),
        ],
        bump = outbound_lane.bump,
    )]
    pub outbound_lane: Box<Account<'info, OutboundLane>>,

    /// CHECK: bound to the named transfer id. A close reads the allocation
    /// there, and a recovery creates its own record there.
    #[account(mut)]
    pub transfer_record: UncheckedAccount<'info>,

    /// CHECK: bound to the derived ledger address, and created by the first
    /// close.
    #[account(mut)]
    pub transport_ledger: UncheckedAccount<'info>,

    pub custody_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: created by hand after the watermark and commitment checks.
    #[account(mut)]
    pub consumed_message: UncheckedAccount<'info>,

    /// CHECK: the address is derived and the account is created here.
    #[account(mut)]
    pub report_record: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,

    /// Co-signs while the leg is frozen, so neither a close nor a recovery
    /// changes a frozen leg without the guardian's consent.
    #[account(address = remote_config.emergency_guardian @ RemoteLegError::Unauthorized)]
    pub emergency_guardian: Option<Signer<'info>>,
}

/// What one close or recovery settled.
struct Settlement {
    kind: SettlementKind,
    claim_amount: u128,
    credited_amount: u64,
    impaired_outstanding: u128,
}

/// The ledger as loaded, and whether it still has to be created.
struct LedgerSlot {
    ledger: TransportLedger,
    exists: bool,
}

pub fn handle_settlement(ctx: Context<ProcessSettlement>, message_bytes: Vec<u8>) -> Result<()> {
    let config_key = ctx.accounts.remote_config.key();
    check_versions(&ctx)?;

    let now = message::current_time()?;
    let ValidatedMessage {
        message,
        message_id,
        fresh,
    } = message::validate_inbound(
        &message_bytes,
        CLASS,
        &ctx.accounts.remote_config,
        &ctx.accounts.control_lane,
        now,
    )?;

    let protocol_types::Body::Resolve(body) = message.body else {
        return Err(RemoteLegError::UnsupportedMessageType.into());
    };
    require!(
        body.action != ResolveAction::Admit,
        RemoteLegError::WrongResolveInstruction
    );
    require_eq!(
        body.config_version.get(),
        ctx.accounts.remote_config.config_version,
        RemoteLegError::InvalidConfigVersion
    );

    let transfer_id = *body.transfer_id.as_bytes();
    let target = TransferAddress::derive(&config_key, &transfer_id);
    require_keys_eq!(
        ctx.accounts.transfer_record.key(),
        target.address,
        RemoteLegError::InvalidTransferRecord
    );

    let sequence = message.header.sequence.get();
    let lane_id = ctx.accounts.remote_config.control_lane_id;
    let consumed = ConsumedAddress::derive(&config_key, lane_id, sequence);
    require_keys_eq!(
        ctx.accounts.consumed_message.key(),
        consumed.address,
        RemoteLegError::InvalidConsumedMessage
    );
    records::check_available(
        &ctx.accounts.consumed_message.to_account_info(),
        RemoteLegError::ReplayDetected,
        RemoteLegError::InvalidConsumedMessage,
    )?;

    require!(
        !ctx.accounts.remote_config.frozen || ctx.accounts.emergency_guardian.is_some(),
        RemoteLegError::Frozen
    );

    let processed_at = Clock::get()?.unix_timestamp;
    let (ledger_address, ledger_bump) = TransportLedger::address(&config_key);
    let mut slot = load_ledger(
        &ctx.accounts.transport_ledger,
        ledger_address,
        ledger_bump,
        processed_at,
    )?;

    let settlement = match body.action {
        ResolveAction::Close => close(ctx.accounts, &body, &mut slot.ledger, processed_at)?,
        ResolveAction::Recover => {
            require!(slot.exists, RemoteLegError::RecoveryStateMismatch);
            recover(
                ctx.accounts,
                &body,
                target.bump,
                sequence,
                &mut slot.ledger,
                processed_at,
            )?
        }
        ResolveAction::Admit => return Err(RemoteLegError::WrongResolveInstruction.into()),
    };

    let payer = ctx.accounts.transport_verifier.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let ledger_info = ctx.accounts.transport_ledger.to_account_info();
    if slot.exists {
        records::store(&ledger_info, &slot.ledger)?;
    } else {
        records::create_and_write(
            &ledger_info,
            &payer,
            &system_program,
            &[TRANSPORT_LEDGER_SEED, config_key.as_ref(), &[ledger_bump]],
            TransportLedger::LEN,
            &slot.ledger,
        )?;
    }

    let built = outbound_message::build_transport_settlement(
        &OutboundContext {
            config: &ctx.accounts.remote_config,
            lane: &ctx.accounts.outbound_lane,
            max_report_age: ctx.accounts.risk_config.max_report_age,
            now,
        },
        &TransportSettlementFields {
            transfer_id,
            kind: settlement.kind,
            claim_amount: settlement.claim_amount,
            credited_amount: settlement.credited_amount,
            impaired_outstanding: settlement.impaired_outstanding,
            config_version: ctx.accounts.remote_config.config_version,
        },
    )?;
    let report_lane_id = ctx.accounts.outbound_lane.lane_id;
    let report_sequence = ctx.accounts.outbound_lane.next_sequence;
    publication::create_report_record(
        &RecordSlots {
            report_record: &ctx.accounts.report_record.to_account_info(),
            payer: &payer,
            system_program: &system_program,
        },
        &RecordEntry {
            remote_config: &config_key,
            lane_id: report_lane_id,
            sequence: report_sequence,
            report_type: ReportType::TransportSettlement,
            built: &built,
            related_transfer_id: transfer_id,
            epoch_id: 0,
            published_at: processed_at,
        },
    )?;
    ctx.accounts.outbound_lane.advance(
        ReportType::TransportSettlement,
        built.next_commitment,
        processed_at,
    )?;

    records::create_and_write(
        &ctx.accounts.consumed_message.to_account_info(),
        &payer,
        &system_program,
        &[
            CONSUMED_MESSAGE_SEED,
            config_key.as_ref(),
            &consumed.lane,
            &consumed.sequence,
            &[consumed.bump],
        ],
        ConsumedMessage::LEN,
        &ConsumedMessage {
            state_version: STATE_VERSION,
            bump: consumed.bump,
            message_class: CLASS,
            lane_id,
            sequence,
            message_id,
        },
    )?;

    let lane = &mut ctx.accounts.control_lane;
    lane.highest_consumed_sequence = sequence;
    lane.message_commitment = message::next_commitment(&lane.message_commitment, &message_id);
    lane.last_accepted_at = processed_at;
    let lane_commitment = lane.message_commitment;

    emit!(TransportSettlementPublished {
        remote_config: config_key,
        report_record: ctx.accounts.report_record.key(),
        transfer_id,
        lane_id: report_lane_id,
        sequence: report_sequence,
        message_id: built.message_id,
        body_hash: built.body_hash,
        previous_commitment: built.previous_commitment,
        next_commitment: built.next_commitment,
        encoded_len: built.encoded_len,
        message_bytes: built.encoded.as_bytes().to_vec(),
        published_at: processed_at,
    });
    emit!(SettlementProcessed {
        remote_config: config_key,
        transfer_record: target.address,
        transfer_id,
        target_sequence: body.target_sequence.get(),
        kind: settlement.kind.to_u8(),
        claim_amount: settlement.claim_amount,
        credited_amount: settlement.credited_amount,
        impaired_outstanding: settlement.impaired_outstanding,
        arrived_late: !fresh,
        guardian_cosigned: ctx.accounts.emergency_guardian.is_some(),
        message_id,
        sequence,
        lane_commitment,
        processed_at,
    });

    Ok(())
}

fn load_ledger(
    account: &UncheckedAccount,
    address: Pubkey,
    bump: u8,
    now: i64,
) -> Result<LedgerSlot> {
    require_keys_eq!(
        account.key(),
        address,
        RemoteLegError::InvalidTransportLedger
    );
    let info = account.to_account_info();
    if info.owner == &crate::ID {
        let ledger: TransportLedger = records::load(&info, RemoteLegError::InvalidTransportLedger)?;
        require_eq!(
            ledger.state_version,
            STATE_VERSION,
            RemoteLegError::InvalidStateVersion
        );
        ledger.check_balanced()?;
        return Ok(LedgerSlot {
            ledger,
            exists: true,
        });
    }
    records::check_available(
        &info,
        RemoteLegError::InvalidTransportLedger,
        RemoteLegError::InvalidTransportLedger,
    )?;
    Ok(LedgerSlot {
        ledger: TransportLedger::empty(bump, now),
        exists: false,
    })
}

/// Ends the named allocation and credits exactly what custody explains.
///
/// A pending allocation first takes any custody that already arrived, as an
/// attribution would. A quarantined one was never admitted, so it takes
/// nothing; what arrived for it stays unattributed for a later recovery.
fn close(
    accounts: &mut ProcessSettlement,
    body: &ResolveBody,
    ledger: &mut TransportLedger,
    now: i64,
) -> Result<Settlement> {
    let info = accounts.transfer_record.to_account_info();
    let mut record: TransferRecord = records::load(&info, RemoteLegError::InvalidTransferRecord)?;
    require_eq!(
        record.state_version,
        STATE_VERSION,
        RemoteLegError::InvalidStateVersion
    );
    require!(
        record.transfer_kind == TransferKind::Allocate,
        RemoteLegError::InvalidTransferKind
    );
    require!(
        record.transfer_id == *body.transfer_id.as_bytes()
            && record.message_sequence == body.target_sequence.get(),
        RemoteLegError::InvalidTransferRecord
    );
    record.check_allocation_shape()?;

    let position = &mut accounts.remote_position;
    let held = position.holds_allocation(&record.transfer_id, record.message_sequence);
    match record.status {
        TransferStatus::Pending => {
            require!(
                held && position.active_transfer_status == TransferStatus::Pending,
                RemoteLegError::InvalidTransferRecord
            );
            let custody_amount = accounts.custody_token_account.amount;
            custody::reconcile(position, custody_amount)?;
            let attributed_now = record
                .outstanding_allocation()?
                .min(position.unattributed_custody);
            position.unattributed_custody = position
                .unattributed_custody
                .checked_sub(attributed_now)
                .ok_or(RemoteLegError::ArithmeticOverflow)?;
            position.attributed_principal = position
                .attributed_principal
                .checked_add(attributed_now)
                .ok_or(RemoteLegError::ArithmeticOverflow)?;
            record.attributed_amount = record
                .attributed_amount
                .checked_add(attributed_now)
                .ok_or(RemoteLegError::ArithmeticOverflow)?;
            custody::check_identity(position, custody_amount)?;
            position.close_transfer();
            record.completed_at = now;
        }
        TransferStatus::Quarantined => {
            require!(
                held && position.active_transfer_status == TransferStatus::Quarantined,
                RemoteLegError::InvalidTransferRecord
            );
            require_eq!(
                record.attributed_amount,
                0,
                RemoteLegError::InvalidTransferRecord
            );
            position.close_transfer();
            record.completed_at = now;
        }
        TransferStatus::Complete => {
            // The vault asked to close before it applied the completion, and
            // it applies no report until this settlement, so the completion
            // still settles exactly once, here, in full.
            position.check_no_active_transfer()?;
            require!(
                position.latest_completed_transfer_id == record.transfer_id,
                RemoteLegError::InvalidTransferRecord
            );
            require_eq!(
                record.attributed_amount,
                record.authorized_amount,
                RemoteLegError::InvalidTransferRecord
            );
        }
        TransferStatus::None | TransferStatus::Closed => {
            return Err(RemoteLegError::TransferNotClosable.into());
        }
    }
    require_gte!(
        record.authorized_amount,
        record.attributed_amount,
        RemoteLegError::AttributionExceedsAuthorization
    );

    record.status = TransferStatus::Closed;
    records::store(&info, &record)?;
    ledger.record_close(
        record.authorized_amount,
        record.attributed_amount,
        record.message_sequence,
    )?;

    Ok(Settlement {
        kind: SettlementKind::Closed,
        claim_amount: u128::from(record.authorized_amount),
        credited_amount: record.attributed_amount,
        impaired_outstanding: ledger.impaired_outstanding,
    })
}

/// Credits verified unattributed custody against the impaired pool.
///
/// The credit is capped by the pool and by the principal ceiling, measured
/// against everything the leg already holds or owes back.
fn recover(
    accounts: &mut ProcessSettlement,
    body: &ResolveBody,
    record_bump: u8,
    sequence: u64,
    ledger: &mut TransportLedger,
    now: i64,
) -> Result<Settlement> {
    require!(
        ledger.impaired_outstanding > 0
            && body.target_sequence.get() == ledger.last_impaired_sequence,
        RemoteLegError::RecoveryStateMismatch
    );
    accounts.remote_position.check_no_active_transfer()?;
    let info = accounts.transfer_record.to_account_info();
    records::check_available(
        &info,
        RemoteLegError::TransferAlreadyExists,
        RemoteLegError::InvalidTransferRecord,
    )?;

    let position = &mut accounts.remote_position;
    let custody_amount = accounts.custody_token_account.amount;
    custody::reconcile(position, custody_amount)?;
    let headroom = accounts
        .strategy_config
        .max_remote_principal
        .saturating_sub(position.committed_principal()?);
    let pool = u64::try_from(ledger.impaired_outstanding).unwrap_or(u64::MAX);
    let credited = position.unattributed_custody.min(pool).min(headroom);
    position.unattributed_custody = position
        .unattributed_custody
        .checked_sub(credited)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    position.attributed_principal = position
        .attributed_principal
        .checked_add(credited)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    custody::check_identity(position, custody_amount)?;

    let claim_amount = ledger.impaired_outstanding;
    ledger.record_recovery(credited)?;

    let transfer_id = *body.transfer_id.as_bytes();
    let config_key = accounts.remote_config.key();
    records::create_and_write(
        &info,
        &accounts.transport_verifier.to_account_info(),
        &accounts.system_program.to_account_info(),
        &[
            TRANSFER_RECORD_SEED,
            config_key.as_ref(),
            &transfer_id,
            &[record_bump],
        ],
        TransferRecord::LEN,
        &TransferRecord::new_recovery(record_bump, transfer_id, sequence, credited, now),
    )?;

    Ok(Settlement {
        kind: SettlementKind::Recovered,
        claim_amount,
        credited_amount: credited,
        impaired_outstanding: ledger.impaired_outstanding,
    })
}

fn check_versions(ctx: &Context<ProcessSettlement>) -> Result<()> {
    for version in [
        ctx.accounts.remote_config.state_version,
        ctx.accounts.risk_config.state_version,
        ctx.accounts.strategy_config.state_version,
        ctx.accounts.remote_position.state_version,
        ctx.accounts.control_lane.state_version,
        ctx.accounts.outbound_lane.state_version,
    ] {
        require_eq!(version, STATE_VERSION, RemoteLegError::InvalidStateVersion);
    }
    require_eq!(
        ctx.accounts.control_lane.lane_id,
        ctx.accounts.remote_config.control_lane_id,
        RemoteLegError::InvalidLane
    );
    require_eq!(
        ctx.accounts.outbound_lane.lane_id,
        ctx.accounts.remote_config.report_lane_id,
        RemoteLegError::InvalidOutboundLane
    );
    Ok(())
}
