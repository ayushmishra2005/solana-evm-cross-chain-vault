//! One deterministic commitment over the whole reported leg state.
//!
//! The bytes are laid out by hand in a fixed order with fixed widths, so the
//! hub can rebuild them without knowing anything about Rust or Borsh.

use anchor_lang::prelude::*;

use crate::errors::RemoteLegError;
use crate::hashing::keccak256;
use crate::strategy::RemotePosition;

/// Domain tag of the committed state layout.
pub const REMOTE_STATE_DOMAIN: &[u8] = b"SOLEVM_REMOTE_STATE_V1";

/// Width of the committed fields, without the domain tag.
pub const REMOTE_STATE_BODY_LEN: usize = 179;

/// Width of the whole hash preimage.
pub const REMOTE_STATE_PREIMAGE_LEN: usize = REMOTE_STATE_DOMAIN.len() + REMOTE_STATE_BODY_LEN;

/// Everything the commitment covers.
#[derive(Clone, Copy, Debug)]
pub struct RemoteStateView<'a> {
    pub remote_config: &'a Pubkey,
    pub config_version: u64,
    pub frozen: bool,
    pub position: &'a RemotePosition,
    pub custody_balance: u64,
    pub adapter_principal: u64,
}

/// Appends fixed width fields without ever running past the buffer.
struct Writer<'a> {
    buffer: &'a mut [u8],
    offset: usize,
}

impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) -> Result<()> {
        let end = self
            .offset
            .checked_add(bytes.len())
            .ok_or(RemoteLegError::ArithmeticOverflow)?;
        let slot = self
            .buffer
            .get_mut(self.offset..end)
            .ok_or(RemoteLegError::InvalidRemoteState)?;
        slot.copy_from_slice(bytes);
        self.offset = end;
        Ok(())
    }
}

/// Writes the canonical preimage and returns the width used.
pub fn write_preimage(state: &RemoteStateView, out: &mut [u8]) -> Result<usize> {
    let position = state.position;
    let mut writer = Writer {
        buffer: out,
        offset: 0,
    };

    writer.put(REMOTE_STATE_DOMAIN)?;
    writer.put(state.remote_config.as_ref())?;
    writer.put(&state.config_version.to_be_bytes())?;
    writer.put(&[u8::from(state.frozen)])?;
    writer.put(&position.attributed_principal.to_be_bytes())?;
    writer.put(&position.deployed_principal.to_be_bytes())?;
    writer.put(&position.recalled_custody.to_be_bytes())?;
    writer.put(&position.unattributed_custody.to_be_bytes())?;
    writer.put(&position.cumulative_realized_loss.to_be_bytes())?;
    writer.put(&position.active_transfer_id)?;
    writer.put(&[position.active_transfer_kind.to_u8()])?;
    writer.put(&[position.active_transfer_status.to_u8()])?;
    writer.put(&position.active_transfer_sequence.to_be_bytes())?;
    writer.put(&position.latest_completed_transfer_id)?;
    writer.put(&position.latest_completion_at.to_be_bytes())?;
    writer.put(&state.custody_balance.to_be_bytes())?;
    writer.put(&state.adapter_principal.to_be_bytes())?;

    Ok(writer.offset)
}

/// Keccak-256 over the domain tag and every committed field.
pub fn commitment(state: &RemoteStateView) -> Result<[u8; 32]> {
    let mut preimage = [0u8; REMOTE_STATE_PREIMAGE_LEN];
    let width = write_preimage(state, &mut preimage)?;
    let slot = preimage
        .get(..width)
        .ok_or(RemoteLegError::InvalidRemoteState)?;
    Ok(keccak256(slot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::{TransferKind, TransferStatus};

    fn position() -> RemotePosition {
        RemotePosition {
            state_version: 1,
            bump: 253,
            attributed_principal: 11,
            deployed_principal: 22,
            recalled_custody: 33,
            unattributed_custody: 44,
            cumulative_realized_loss: 55,
            active_transfer_id: [0x6A; 32],
            active_transfer_kind: TransferKind::Recall,
            active_transfer_sequence: 66,
            active_transfer_status: TransferStatus::Pending,
            latest_completed_transfer_id: [0x7B; 32],
            latest_completion_at: 77,
            initialized_at: 88,
        }
    }

    fn view<'a>(config: &'a Pubkey, position: &'a RemotePosition) -> RemoteStateView<'a> {
        RemoteStateView {
            remote_config: config,
            config_version: 4,
            frozen: false,
            position,
            custody_balance: 88,
            adapter_principal: 22,
        }
    }

    #[test]
    fn the_preimage_uses_the_documented_width() {
        let config = Pubkey::new_unique();
        let held = position();
        let mut buffer = [0u8; REMOTE_STATE_PREIMAGE_LEN];

        let width = write_preimage(&view(&config, &held), &mut buffer).expect("the state fits");
        assert_eq!(width, REMOTE_STATE_PREIMAGE_LEN);
        assert_eq!(REMOTE_STATE_BODY_LEN, 179);
    }

    #[test]
    fn the_preimage_starts_with_the_domain_tag_and_the_configuration() {
        let config = Pubkey::new_unique();
        let held = position();
        let mut buffer = [0u8; REMOTE_STATE_PREIMAGE_LEN];
        write_preimage(&view(&config, &held), &mut buffer).expect("the state fits");

        let tag = REMOTE_STATE_DOMAIN.len();
        assert_eq!(&buffer[..tag], REMOTE_STATE_DOMAIN);
        assert_eq!(&buffer[tag..tag + 32], config.as_ref());
    }

    /// Rebuilds the bytes by hand, the way an outside reader would.
    fn rebuilt(config: &Pubkey, held: &RemotePosition, state: &RemoteStateView) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(REMOTE_STATE_DOMAIN);
        bytes.extend_from_slice(config.as_ref());
        bytes.extend_from_slice(&state.config_version.to_be_bytes());
        bytes.push(u8::from(state.frozen));
        bytes.extend_from_slice(&held.attributed_principal.to_be_bytes());
        bytes.extend_from_slice(&held.deployed_principal.to_be_bytes());
        bytes.extend_from_slice(&held.recalled_custody.to_be_bytes());
        bytes.extend_from_slice(&held.unattributed_custody.to_be_bytes());
        bytes.extend_from_slice(&held.cumulative_realized_loss.to_be_bytes());
        bytes.extend_from_slice(&held.active_transfer_id);
        bytes.push(held.active_transfer_kind.to_u8());
        bytes.push(held.active_transfer_status.to_u8());
        bytes.extend_from_slice(&held.active_transfer_sequence.to_be_bytes());
        bytes.extend_from_slice(&held.latest_completed_transfer_id);
        bytes.extend_from_slice(&held.latest_completion_at.to_be_bytes());
        bytes.extend_from_slice(&state.custody_balance.to_be_bytes());
        bytes.extend_from_slice(&state.adapter_principal.to_be_bytes());
        bytes
    }

    #[test]
    fn an_independent_reconstruction_reaches_the_same_hash() {
        let config = Pubkey::new_unique();
        let held = position();
        let state = view(&config, &held);

        let expected = keccak256(&rebuilt(&config, &held, &state));
        assert_eq!(commitment(&state).expect("the state commits"), expected);
    }

    #[test]
    fn the_same_state_always_commits_to_the_same_value() {
        let config = Pubkey::new_unique();
        let held = position();
        let first = commitment(&view(&config, &held)).expect("the state commits");
        let second = commitment(&view(&config, &held)).expect("the state commits");
        assert_eq!(first, second);
    }

    #[track_caller]
    fn assert_changes(change: impl FnOnce(&mut RemotePosition, &mut RemoteStateView)) {
        let config = Pubkey::new_unique();
        let held = position();
        let base = commitment(&view(&config, &held)).expect("the state commits");

        let mut changed = position();
        let mut state = view(&config, &held);
        change(&mut changed, &mut state);
        state.position = &changed;

        assert_ne!(base, commitment(&state).expect("the state commits"));
    }

    #[test]
    fn a_different_configuration_version_changes_the_commitment() {
        assert_changes(|_, state| state.config_version = 5);
    }

    #[test]
    fn freezing_changes_the_commitment() {
        assert_changes(|_, state| state.frozen = true);
    }

    #[test]
    fn every_asset_bucket_changes_the_commitment() {
        assert_changes(|held, _| held.attributed_principal += 1);
        assert_changes(|held, _| held.deployed_principal += 1);
        assert_changes(|held, _| held.recalled_custody += 1);
        assert_changes(|held, _| held.unattributed_custody += 1);
        assert_changes(|held, _| held.cumulative_realized_loss += 1);
    }

    #[test]
    fn the_open_transfer_identity_changes_the_commitment() {
        assert_changes(|held, _| held.active_transfer_id = [0x6B; 32]);
        assert_changes(|held, _| held.active_transfer_kind = TransferKind::Allocate);
        assert_changes(|held, _| held.active_transfer_status = TransferStatus::Complete);
        assert_changes(|held, _| held.active_transfer_sequence += 1);
    }

    #[test]
    fn the_latest_completed_transfer_changes_the_commitment() {
        assert_changes(|held, _| held.latest_completed_transfer_id = [0x7C; 32]);
        assert_changes(|held, _| held.latest_completion_at += 1);
    }

    #[test]
    fn the_custody_balance_changes_the_commitment() {
        assert_changes(|_, state| state.custody_balance += 1);
    }

    #[test]
    fn the_adapter_principal_changes_the_commitment() {
        assert_changes(|_, state| state.adapter_principal += 1);
    }

    #[test]
    fn a_different_configuration_address_changes_the_commitment() {
        let held = position();
        let first = Pubkey::new_unique();
        let second = Pubkey::new_unique();
        assert_ne!(
            commitment(&view(&first, &held)).expect("the state commits"),
            commitment(&view(&second, &held)).expect("the state commits")
        );
    }

    #[test]
    fn a_buffer_that_is_too_small_refuses_to_write() {
        let config = Pubkey::new_unique();
        let held = position();
        let mut buffer = [0u8; REMOTE_STATE_PREIMAGE_LEN - 1];
        assert_eq!(
            write_preimage(&view(&config, &held), &mut buffer),
            Err(RemoteLegError::InvalidRemoteState.into())
        );
    }

    #[test]
    fn the_committed_value_is_never_zero_for_real_state() {
        let config = Pubkey::new_unique();
        let held = position();
        assert_ne!(
            commitment(&view(&config, &held)).expect("commits"),
            [0u8; 32]
        );
    }
}
