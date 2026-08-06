//! Fixed size state for the strategy adapter, the position and transfers.

use anchor_lang::prelude::*;

use crate::control::MessageClass;
use crate::errors::RemoteLegError;

/// Seed prefix of the immutable adapter configuration.
pub const STRATEGY_CONFIG_SEED: &[u8] = b"strategy-config";

/// Seed prefix of the position account.
pub const REMOTE_POSITION_SEED: &[u8] = b"remote-position";

/// Seed prefix of one transfer record.
pub const TRANSFER_RECORD_SEED: &[u8] = b"transfer-record";

/// Bytes held back for later fields of the same layout version.
pub const STRATEGY_CONFIG_RESERVED: usize = 32;

/// Which side of the protocol one transfer moves assets for.
///
/// The discriminant is stored, so it must never be reordered.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum TransferKind {
    None = 0,
    Allocate = 1,
    Recall = 2,
}

impl TransferKind {
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// The replay lane a transfer of this kind arrives through.
    #[must_use]
    pub const fn message_class(self) -> Option<MessageClass> {
        match self {
            Self::None => None,
            Self::Allocate => Some(MessageClass::Allocate),
            Self::Recall => Some(MessageClass::Recall),
        }
    }
}

/// Where one transfer stands in its lifecycle.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum TransferStatus {
    None = 0,
    Pending = 1,
    Complete = 2,
}

impl TransferStatus {
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }
}

/// Adapter identity fixed once, so no caller may redirect assets.
#[account]
#[derive(InitSpace, Debug)]
pub struct StrategyConfig {
    pub state_version: u8,
    pub bump: u8,
    pub adapter_program: Pubkey,
    pub adapter_state: Pubkey,
    pub adapter_authority: Pubkey,
    pub adapter_token_vault: Pubkey,
    pub max_remote_principal: u64,
    pub initialized_at: i64,
    pub reserved: [u8; STRATEGY_CONFIG_RESERVED],
}

impl StrategyConfig {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Seeds of the strategy configuration for one remote leg.
    #[must_use]
    pub fn seeds(remote_config: &Pubkey) -> [&[u8]; 2] {
        [STRATEGY_CONFIG_SEED, remote_config.as_ref()]
    }
}

/// Every asset bucket the leg tracks, plus the one open transfer.
#[account]
#[derive(InitSpace, Debug)]
pub struct RemotePosition {
    pub state_version: u8,
    pub bump: u8,
    pub attributed_principal: u64,
    pub deployed_principal: u64,
    pub recalled_custody: u64,
    pub unattributed_custody: u64,
    pub cumulative_realized_loss: u64,
    pub active_transfer_id: [u8; 32],
    pub active_transfer_kind: TransferKind,
    pub active_transfer_sequence: u64,
    pub active_transfer_status: TransferStatus,
    pub latest_completed_transfer_id: [u8; 32],
    pub latest_completion_at: i64,
    pub initialized_at: i64,
}

impl RemotePosition {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Seeds of the position for one remote leg.
    #[must_use]
    pub fn seeds(remote_config: &Pubkey) -> [&[u8]; 2] {
        [REMOTE_POSITION_SEED, remote_config.as_ref()]
    }

    /// True while one allocation or recall is still unresolved.
    #[must_use]
    pub fn has_active_transfer(&self) -> bool {
        self.active_transfer_kind != TransferKind::None
    }

    /// Rejects a new cycle while another one is open.
    pub fn check_no_active_transfer(&self) -> Result<()> {
        require!(!self.has_active_transfer(), RemoteLegError::UnresolvedCycle);
        Ok(())
    }

    /// Custody tokens the leg has already explained.
    pub fn accounted_custody(&self) -> Result<u64> {
        self.attributed_principal
            .checked_add(self.recalled_custody)
            .and_then(|total| total.checked_add(self.unattributed_custody))
            .ok_or_else(|| RemoteLegError::ArithmeticOverflow.into())
    }

    /// Principal the vault has accepted on this chain.
    pub fn accepted_principal(&self) -> Result<u64> {
        self.attributed_principal
            .checked_add(self.deployed_principal)
            .ok_or_else(|| RemoteLegError::ArithmeticOverflow.into())
    }

    /// Opens one cycle for a transfer that just arrived.
    pub fn open_transfer(&mut self, kind: TransferKind, transfer_id: [u8; 32], sequence: u64) {
        self.active_transfer_kind = kind;
        self.active_transfer_id = transfer_id;
        self.active_transfer_sequence = sequence;
        self.active_transfer_status = TransferStatus::Pending;
    }

    /// Closes the open cycle and remembers which transfer finished.
    pub fn complete_transfer(&mut self, transfer_id: [u8; 32], completed_at: i64) {
        self.active_transfer_kind = TransferKind::None;
        self.active_transfer_id = [0u8; 32];
        self.active_transfer_sequence = 0;
        self.active_transfer_status = TransferStatus::None;
        self.latest_completed_transfer_id = transfer_id;
        self.latest_completion_at = completed_at;
    }
}

/// One allocation or recall, from arrival to completion.
///
/// Both kinds share this shape. Fields the kind does not use stay zero.
#[account]
#[derive(InitSpace, Debug)]
pub struct TransferRecord {
    pub state_version: u8,
    pub bump: u8,
    pub transfer_kind: TransferKind,
    pub message_class: MessageClass,
    pub status: TransferStatus,
    pub transfer_id: [u8; 32],
    pub message_sequence: u64,
    pub authorized_amount: u64,
    pub attributed_amount: u64,
    pub requested_recall_amount: u64,
    pub minimum_amount: u64,
    pub custody_principal_reserved: u64,
    pub strategy_principal_resolved: u64,
    pub assets_withdrawn: u64,
    pub assets_sent: u64,
    pub realized_loss: u64,
    /// Reported by the source chain, never verified here.
    pub expected_source_balance: u128,
    pub created_at: i64,
    pub completed_at: i64,
}

impl TransferRecord {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Seeds of one record, given the transfer id from the message.
    #[must_use]
    pub fn seeds<'a>(remote_config: &'a Pubkey, transfer_id: &'a [u8; 32]) -> [&'a [u8]; 3] {
        [TRANSFER_RECORD_SEED, remote_config.as_ref(), transfer_id]
    }

    /// A record for an allocation that has not been attributed yet.
    #[must_use]
    pub fn new_allocation(
        bump: u8,
        transfer_id: [u8; 32],
        message_sequence: u64,
        authorized_amount: u64,
        minimum_amount: u64,
        expected_source_balance: u128,
        created_at: i64,
    ) -> Self {
        Self {
            state_version: crate::state::STATE_VERSION,
            bump,
            transfer_kind: TransferKind::Allocate,
            message_class: MessageClass::Allocate,
            status: TransferStatus::Pending,
            transfer_id,
            message_sequence,
            authorized_amount,
            attributed_amount: 0,
            requested_recall_amount: 0,
            minimum_amount,
            custody_principal_reserved: 0,
            strategy_principal_resolved: 0,
            assets_withdrawn: 0,
            assets_sent: 0,
            realized_loss: 0,
            expected_source_balance,
            created_at,
            completed_at: 0,
        }
    }

    /// A record for a recall that has not been unwound yet.
    #[must_use]
    pub fn new_recall(
        bump: u8,
        transfer_id: [u8; 32],
        message_sequence: u64,
        requested_recall_amount: u64,
        minimum_amount: u64,
        custody_principal_reserved: u64,
        created_at: i64,
    ) -> Self {
        Self {
            state_version: crate::state::STATE_VERSION,
            bump,
            transfer_kind: TransferKind::Recall,
            message_class: MessageClass::Recall,
            status: TransferStatus::Pending,
            transfer_id,
            message_sequence,
            authorized_amount: 0,
            attributed_amount: 0,
            requested_recall_amount,
            minimum_amount,
            custody_principal_reserved,
            strategy_principal_resolved: 0,
            assets_withdrawn: 0,
            assets_sent: 0,
            realized_loss: 0,
            expected_source_balance: 0,
            created_at,
            completed_at: 0,
        }
    }

    /// Rejects a record that is not the open transfer of the wanted kind.
    pub fn check_active(&self, kind: TransferKind, position: &RemotePosition) -> Result<()> {
        require_eq!(
            self.state_version,
            crate::state::STATE_VERSION,
            RemoteLegError::InvalidStateVersion
        );
        require!(
            self.transfer_kind == kind,
            RemoteLegError::InvalidTransferKind
        );
        require!(
            self.status == TransferStatus::Pending,
            RemoteLegError::InvalidTransferStatus
        );
        require!(
            position.active_transfer_kind == kind,
            RemoteLegError::NoActiveTransfer
        );
        require!(
            position.active_transfer_id == self.transfer_id,
            RemoteLegError::InvalidTransferRecord
        );
        Ok(())
    }

    /// Fields an allocation never uses must stay at zero.
    pub fn check_allocation_shape(&self) -> Result<()> {
        let unused = [
            self.requested_recall_amount,
            self.custody_principal_reserved,
            self.strategy_principal_resolved,
            self.assets_withdrawn,
            self.assets_sent,
            self.realized_loss,
        ];
        require!(
            unused.iter().all(|value| *value == 0),
            RemoteLegError::InvalidTransferRecord
        );
        Ok(())
    }

    /// Fields a recall never uses must stay at zero.
    pub fn check_recall_shape(&self) -> Result<()> {
        require!(
            self.authorized_amount == 0
                && self.attributed_amount == 0
                && self.expected_source_balance == 0,
            RemoteLegError::InvalidTransferRecord
        );
        Ok(())
    }

    /// Amount of the authorization that is still waiting for assets.
    pub fn outstanding_allocation(&self) -> Result<u64> {
        self.authorized_amount
            .checked_sub(self.attributed_amount)
            .ok_or_else(|| RemoteLegError::AttributionExceedsAuthorization.into())
    }

    /// Recall principal that is neither reserved from custody nor withdrawn.
    pub fn unresolved_recall_principal(&self) -> Result<u64> {
        let resolved = self
            .custody_principal_reserved
            .checked_add(self.strategy_principal_resolved)
            .ok_or(RemoteLegError::ArithmeticOverflow)?;
        self.requested_recall_amount
            .checked_sub(resolved)
            .ok_or_else(|| RemoteLegError::InvalidRecallAmount.into())
    }
}

/// Closes a recall once sent assets plus realized loss match the request.
///
/// A total loss settles the request without a send, so both the withdrawal and
/// the send path apply this rule.
pub fn settle_recall(
    record: &mut TransferRecord,
    position: &mut RemotePosition,
    now: i64,
) -> Result<bool> {
    let settled = record
        .assets_sent
        .checked_add(record.realized_loss)
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    require_gte!(
        record.requested_recall_amount,
        settled,
        RemoteLegError::InvalidRecallAmount
    );
    if settled != record.requested_recall_amount {
        return Ok(false);
    }

    record.status = TransferStatus::Complete;
    record.completed_at = now;
    position.complete_transfer(record.transfer_id, now);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_strategy_account_has_a_fixed_documented_size() {
        assert_eq!(StrategyConfig::INIT_SPACE, 178);
        assert_eq!(StrategyConfig::LEN, 186);
        assert_eq!(RemotePosition::INIT_SPACE, 132);
        assert_eq!(RemotePosition::LEN, 140);
        assert_eq!(TransferRecord::INIT_SPACE, 149);
        assert_eq!(TransferRecord::LEN, 157);
    }

    #[test]
    fn the_transfer_kind_byte_never_changes() {
        assert_eq!(TransferKind::None.to_u8(), 0);
        assert_eq!(TransferKind::Allocate.to_u8(), 1);
        assert_eq!(TransferKind::Recall.to_u8(), 2);
    }

    #[test]
    fn the_transfer_status_byte_never_changes() {
        assert_eq!(TransferStatus::None.to_u8(), 0);
        assert_eq!(TransferStatus::Pending.to_u8(), 1);
        assert_eq!(TransferStatus::Complete.to_u8(), 2);
    }

    #[test]
    fn each_transfer_kind_maps_to_its_replay_lane() {
        assert_eq!(TransferKind::None.message_class(), None);
        assert_eq!(
            TransferKind::Allocate.message_class(),
            Some(MessageClass::Allocate)
        );
        assert_eq!(
            TransferKind::Recall.message_class(),
            Some(MessageClass::Recall)
        );
    }

    #[test]
    fn every_transfer_record_field_keeps_its_documented_offset() {
        let record = TransferRecord::new_allocation(7, [0xC1; 32], 9, 500, 400, 12_345, 77);
        let mut bytes = Vec::new();
        record.serialize(&mut bytes).expect("record encodes");
        assert_eq!(bytes.len(), TransferRecord::INIT_SPACE);

        #[track_caller]
        fn field(bytes: &[u8], offset: usize, expected: &[u8]) {
            assert_eq!(
                bytes.get(offset..offset + expected.len()),
                Some(expected),
                "field at offset {offset} moved"
            );
        }

        field(&bytes, 0, &[crate::state::STATE_VERSION]);
        field(&bytes, 1, &[7]);
        field(&bytes, 2, &[TransferKind::Allocate.to_u8()]);
        field(&bytes, 3, &[MessageClass::Allocate.to_u8()]);
        field(&bytes, 4, &[TransferStatus::Pending.to_u8()]);
        field(&bytes, 5, &[0xC1; 32]);
        field(&bytes, 37, &9u64.to_le_bytes());
        field(&bytes, 45, &500u64.to_le_bytes());
        field(&bytes, 53, &0u64.to_le_bytes());
        field(&bytes, 61, &0u64.to_le_bytes());
        field(&bytes, 69, &400u64.to_le_bytes());
        field(&bytes, 77, &0u64.to_le_bytes());
        field(&bytes, 85, &0u64.to_le_bytes());
        field(&bytes, 93, &0u64.to_le_bytes());
        field(&bytes, 101, &0u64.to_le_bytes());
        field(&bytes, 109, &0u64.to_le_bytes());
        field(&bytes, 117, &12_345u128.to_le_bytes());
        field(&bytes, 133, &77i64.to_le_bytes());
        field(&bytes, 141, &0i64.to_le_bytes());
        assert_eq!(149, TransferRecord::INIT_SPACE);
    }

    #[test]
    fn a_new_allocation_leaves_every_recall_field_at_zero() {
        let record = TransferRecord::new_allocation(1, [1u8; 32], 2, 100, 90, 7, 5);
        assert!(record.check_allocation_shape().is_ok());
        assert_eq!(
            record.outstanding_allocation().expect("the call succeeds"),
            100
        );
    }

    #[test]
    fn a_new_recall_leaves_every_allocation_field_at_zero() {
        let record = TransferRecord::new_recall(1, [1u8; 32], 2, 100, 90, 40, 5);
        assert!(record.check_recall_shape().is_ok());
        assert_eq!(
            record
                .unresolved_recall_principal()
                .expect("the call succeeds"),
            60
        );
    }

    #[test]
    fn an_allocation_carrying_recall_values_is_rejected() {
        let mut record = TransferRecord::new_allocation(1, [1u8; 32], 2, 100, 90, 7, 5);
        record.assets_sent = 1;
        assert_eq!(
            record.check_allocation_shape().expect_err("the call fails"),
            Error::from(RemoteLegError::InvalidTransferRecord)
        );
    }

    #[test]
    fn a_recall_carrying_allocation_values_is_rejected() {
        let mut record = TransferRecord::new_recall(1, [1u8; 32], 2, 100, 90, 40, 5);
        record.attributed_amount = 1;
        assert_eq!(
            record.check_recall_shape().expect_err("the call fails"),
            Error::from(RemoteLegError::InvalidTransferRecord)
        );
    }

    #[test]
    fn attribution_above_the_authorization_is_rejected() {
        let mut record = TransferRecord::new_allocation(1, [1u8; 32], 2, 100, 90, 7, 5);
        record.attributed_amount = 101;
        assert_eq!(
            record.outstanding_allocation().expect_err("the call fails"),
            Error::from(RemoteLegError::AttributionExceedsAuthorization)
        );
    }

    #[test]
    fn a_fully_resolved_recall_has_no_unresolved_principal() {
        let mut record = TransferRecord::new_recall(1, [1u8; 32], 2, 100, 90, 40, 5);
        record.strategy_principal_resolved = 60;
        assert_eq!(
            record
                .unresolved_recall_principal()
                .expect("the call succeeds"),
            0
        );
    }

    #[test]
    fn resolving_more_than_the_request_is_rejected() {
        let mut record = TransferRecord::new_recall(1, [1u8; 32], 2, 100, 90, 40, 5);
        record.strategy_principal_resolved = 61;
        assert_eq!(
            record
                .unresolved_recall_principal()
                .expect_err("the call fails"),
            Error::from(RemoteLegError::InvalidRecallAmount)
        );
    }

    fn sample_position() -> RemotePosition {
        RemotePosition {
            state_version: crate::state::STATE_VERSION,
            bump: 1,
            attributed_principal: 10,
            deployed_principal: 20,
            recalled_custody: 5,
            unattributed_custody: 3,
            cumulative_realized_loss: 0,
            active_transfer_id: [0u8; 32],
            active_transfer_kind: TransferKind::None,
            active_transfer_sequence: 0,
            active_transfer_status: TransferStatus::None,
            latest_completed_transfer_id: [0u8; 32],
            latest_completion_at: 0,
            initialized_at: 0,
        }
    }

    #[test]
    fn the_accounted_custody_adds_every_custody_bucket() {
        assert_eq!(
            sample_position()
                .accounted_custody()
                .expect("the call succeeds"),
            18
        );
    }

    #[test]
    fn the_accepted_principal_leaves_out_unattributed_custody() {
        assert_eq!(
            sample_position()
                .accepted_principal()
                .expect("the call succeeds"),
            30
        );
    }

    #[test]
    fn a_free_position_accepts_a_new_cycle() {
        assert!(sample_position().check_no_active_transfer().is_ok());
    }

    #[test]
    fn a_position_with_an_open_cycle_rejects_a_new_one() {
        let mut position = sample_position();
        position.open_transfer(TransferKind::Allocate, [9u8; 32], 4);
        assert!(position.has_active_transfer());
        assert_eq!(
            position
                .check_no_active_transfer()
                .expect_err("the call fails"),
            Error::from(RemoteLegError::UnresolvedCycle)
        );
    }

    #[test]
    fn completing_a_cycle_frees_the_position_and_records_the_transfer() {
        let mut position = sample_position();
        position.open_transfer(TransferKind::Recall, [9u8; 32], 4);
        position.complete_transfer([9u8; 32], 100);

        assert!(!position.has_active_transfer());
        assert_eq!(position.active_transfer_sequence, 0);
        assert_eq!(position.active_transfer_status, TransferStatus::None);
        assert_eq!(position.latest_completed_transfer_id, [9u8; 32]);
        assert_eq!(position.latest_completion_at, 100);
    }

    #[test]
    fn a_record_is_active_only_for_its_own_kind_and_id() {
        let record = TransferRecord::new_allocation(1, [9u8; 32], 2, 100, 90, 0, 5);
        let mut position = sample_position();
        position.open_transfer(TransferKind::Allocate, [9u8; 32], 2);
        assert!(
            record
                .check_active(TransferKind::Allocate, &position)
                .is_ok()
        );

        assert_eq!(
            record
                .check_active(TransferKind::Recall, &position)
                .expect_err("the call fails"),
            Error::from(RemoteLegError::InvalidTransferKind)
        );

        position.open_transfer(TransferKind::Allocate, [8u8; 32], 2);
        assert_eq!(
            record
                .check_active(TransferKind::Allocate, &position)
                .expect_err("the call fails"),
            Error::from(RemoteLegError::InvalidTransferRecord)
        );
    }

    #[test]
    fn a_completed_record_is_no_longer_active() {
        let mut record = TransferRecord::new_allocation(1, [9u8; 32], 2, 100, 90, 0, 5);
        record.status = TransferStatus::Complete;
        let mut position = sample_position();
        position.open_transfer(TransferKind::Allocate, [9u8; 32], 2);
        assert_eq!(
            record
                .check_active(TransferKind::Allocate, &position)
                .expect_err("the call fails"),
            Error::from(RemoteLegError::InvalidTransferStatus)
        );
    }

    #[test]
    fn the_strategy_seeds_keep_their_documented_order() {
        let config = Pubkey::new_unique();
        let transfer_id = [4u8; 32];
        assert_eq!(StrategyConfig::seeds(&config)[0], STRATEGY_CONFIG_SEED);
        assert_eq!(StrategyConfig::seeds(&config)[1], config.as_ref());
        assert_eq!(RemotePosition::seeds(&config)[0], REMOTE_POSITION_SEED);
        assert_eq!(RemotePosition::seeds(&config)[1], config.as_ref());

        let seeds = TransferRecord::seeds(&config, &transfer_id);
        assert_eq!(seeds[0], TRANSFER_RECORD_SEED);
        assert_eq!(seeds[1], config.as_ref());
        assert_eq!(seeds[2], &transfer_id[..]);
    }

    #[test]
    fn two_transfer_ids_give_two_record_addresses() {
        let config = Pubkey::new_unique();
        let first =
            Pubkey::find_program_address(&TransferRecord::seeds(&config, &[1u8; 32]), &crate::ID);
        let second =
            Pubkey::find_program_address(&TransferRecord::seeds(&config, &[2u8; 32]), &crate::ID);
        assert_ne!(first.0, second.0);
    }
}
