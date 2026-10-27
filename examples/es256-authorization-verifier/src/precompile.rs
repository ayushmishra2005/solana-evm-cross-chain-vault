//! Reads the secp256r1 verification that must immediately precede `consume`.
//!
//! The native program accepts many layouts: up to eight signatures, offsets
//! into other instructions, overlapping slices and trailing bytes. This
//! program accepts exactly one, so the bytes it parses are the bytes the
//! native program hashed and verified.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{Instruction, get_stack_height};
use solana_instructions_sysvar::{load_current_index_checked, load_instruction_at_checked};

use crate::errors::VerifierError;
use crate::state::COMPRESSED_PUBLIC_KEY_LEN;

/// Instruction-data index meaning "this instruction" to the native program.
pub const CURRENT_INSTRUCTION: u16 = u16::MAX;
pub const SIGNATURE_LEN: usize = 64;
pub const PUBLIC_KEY_OFFSET: usize = 16;
pub const SIGNATURE_OFFSET: usize = PUBLIC_KEY_OFFSET + COMPRESSED_PUBLIC_KEY_LEN;
pub const MESSAGE_OFFSET: usize = SIGNATURE_OFFSET + SIGNATURE_LEN;
/// Upper bound on the JWS signing input; the largest accepted transaction
/// stays under the 1,232-byte packet limit.
pub const MAX_SIGNING_INPUT_LEN: usize = 512;

const TRANSACTION_LEVEL_STACK_HEIGHT: usize = 1;

/// The fields a valid precompile instruction pins down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedSignature<'a> {
    pub public_key: &'a [u8],
    pub signing_input: &'a [u8],
}

/// The 16-byte header of the only layout accepted: one signature, zero
/// padding, then key, signature and message in this instruction's own data.
pub fn canonical_header(signing_input_len: u16) -> [u8; PUBLIC_KEY_OFFSET] {
    let mut header = [0u8; PUBLIC_KEY_OFFSET];
    let fields: [u16; 7] = [
        SIGNATURE_OFFSET as u16,
        CURRENT_INSTRUCTION,
        PUBLIC_KEY_OFFSET as u16,
        CURRENT_INSTRUCTION,
        MESSAGE_OFFSET as u16,
        signing_input_len,
        CURRENT_INSTRUCTION,
    ];
    let (count, offsets) = header.split_at_mut(2);
    count.copy_from_slice(&[1, 0]);
    for (slot, field) in offsets.as_chunks_mut::<2>().0.iter_mut().zip(fields) {
        *slot = field.to_le_bytes();
    }
    header
}

/// Parses precompile instruction data in the canonical layout.
pub fn parse_precompile_data(data: &[u8]) -> Result<VerifiedSignature<'_>> {
    let signing_input = data
        .get(MESSAGE_OFFSET..)
        .ok_or(VerifierError::InvalidPrecompileInstruction)?;
    require!(
        !signing_input.is_empty() && signing_input.len() <= MAX_SIGNING_INPUT_LEN,
        VerifierError::InvalidSigningInputLength
    );
    let signing_input_len =
        u16::try_from(signing_input.len()).map_err(|_| VerifierError::InvalidSigningInputLength)?;
    let header = data
        .get(..PUBLIC_KEY_OFFSET)
        .ok_or(VerifierError::InvalidPrecompileInstruction)?;
    require!(
        header == canonical_header(signing_input_len),
        VerifierError::InvalidPrecompileInstruction
    );
    let public_key = data
        .get(PUBLIC_KEY_OFFSET..SIGNATURE_OFFSET)
        .ok_or(VerifierError::InvalidPrecompileInstruction)?;
    Ok(VerifiedSignature {
        public_key,
        signing_input,
    })
}

/// Returns the secp256r1 instruction that verified the signature this call
/// consumes, after checking the transaction around it.
///
/// Accepted shape: zero or more compute-budget instructions, the precompile,
/// then this instruction. Later instructions may follow, but none may target
/// this program or the precompile, so one transaction consumes at most one
/// authorization.
pub fn load_verified_precompile(instructions_sysvar: &AccountInfo) -> Result<Instruction> {
    require!(
        get_stack_height() == TRANSACTION_LEVEL_STACK_HEIGHT,
        VerifierError::NotTopLevel
    );
    let current = usize::from(load_current_index_checked(instructions_sysvar)?);
    let this = load_instruction_at_checked(current, instructions_sysvar)?;
    require!(this.program_id == crate::ID, VerifierError::NotTopLevel);

    let precompile_index = current
        .checked_sub(1)
        .ok_or(VerifierError::MissingPrecompile)?;
    let count = instruction_count(instructions_sysvar)?;
    for index in 0..count {
        if index == precompile_index || index == current {
            continue;
        }
        let program_id = load_instruction_at_checked(index, instructions_sysvar)?.program_id;
        let allowed = if index < precompile_index {
            program_id == solana_sdk_ids::compute_budget::ID
        } else {
            program_id != crate::ID && program_id != solana_sdk_ids::secp256r1_program::ID
        };
        require!(allowed, VerifierError::InvalidInstructionOrder);
    }

    let precompile = load_instruction_at_checked(precompile_index, instructions_sysvar)?;
    require!(
        precompile.program_id == solana_sdk_ids::secp256r1_program::ID,
        VerifierError::MissingPrecompile
    );
    require!(
        precompile.accounts.is_empty(),
        VerifierError::InvalidPrecompileInstruction
    );
    Ok(precompile)
}

/// The sysvar starts with its instruction count as a little-endian u16.
fn instruction_count(instructions_sysvar: &AccountInfo) -> Result<usize> {
    require_keys_eq!(
        *instructions_sysvar.key,
        solana_instructions_sysvar::ID,
        VerifierError::InvalidInstructionOrder
    );
    let data = instructions_sysvar.try_borrow_data()?;
    let count = data
        .get(..2)
        .and_then(|bytes| <[u8; 2]>::try_from(bytes).ok())
        .ok_or(VerifierError::InvalidInstructionOrder)?;
    Ok(usize::from(u16::from_le_bytes(count)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn instruction_data(signing_input: &[u8]) -> Vec<u8> {
        let mut data = canonical_header(signing_input.len() as u16).to_vec();
        data.extend_from_slice(&[0x02; COMPRESSED_PUBLIC_KEY_LEN]);
        data.extend_from_slice(&[0x11; SIGNATURE_LEN]);
        data.extend_from_slice(signing_input);
        data
    }

    #[test]
    fn accepts_the_canonical_layout() {
        let data = instruction_data(b"a.b");
        let verified = parse_precompile_data(&data).unwrap();
        assert_eq!(verified.signing_input, b"a.b");
        assert_eq!(verified.public_key, &[0x02; COMPRESSED_PUBLIC_KEY_LEN]);
    }

    #[test]
    fn rejects_trailing_bytes_the_native_program_would_ignore() {
        let mut data = instruction_data(b"a.b");
        data.push(0);
        assert!(parse_precompile_data(&data).is_err());
    }

    #[test]
    fn rejects_other_signature_counts_and_references() {
        for (index, value) in [(0, 2), (1, 1), (4, 0), (5, 0)] {
            let mut data = instruction_data(b"a.b");
            data[index] = value;
            assert!(parse_precompile_data(&data).is_err(), "byte {index}");
        }
    }

    #[test]
    fn rejects_every_change_to_the_header() {
        let canonical = instruction_data(b"a.b");
        for index in 0..PUBLIC_KEY_OFFSET {
            for value in 0..=255u8 {
                let mut data = canonical.clone();
                data[index] = value;
                assert_eq!(
                    parse_precompile_data(&data).is_ok(),
                    value == canonical[index],
                    "byte {index} = {value:#04x}"
                );
            }
        }
    }

    #[test]
    fn every_truncation_fails_without_panicking() {
        let data = instruction_data(b"a.b");
        for length in 0..data.len() {
            assert!(parse_precompile_data(&data[..length]).is_err(), "{length}");
        }
    }

    #[test]
    fn signing_input_length_bounds_are_inclusive() {
        assert!(parse_precompile_data(&instruction_data(&[b'a'; MAX_SIGNING_INPUT_LEN])).is_ok());
        let mut oversized = canonical_header(513).to_vec();
        oversized.extend_from_slice(&[0x02; COMPRESSED_PUBLIC_KEY_LEN]);
        oversized.extend_from_slice(&[0x11; SIGNATURE_LEN]);
        oversized.extend_from_slice(&[b'a'; 513]);
        assert!(parse_precompile_data(&oversized).is_err());
        let mut empty = canonical_header(0).to_vec();
        empty.extend_from_slice(&[0x02; COMPRESSED_PUBLIC_KEY_LEN]);
        empty.extend_from_slice(&[0x11; SIGNATURE_LEN]);
        assert!(parse_precompile_data(&empty).is_err());
    }
}
