//! Test-only program for adversarial verifier tests. Never deploy it.
//!
//! With accounts, the first is the target program and the rest are passed
//! through with their signer and writable flags; the instruction data is
//! relayed unchanged by CPI. Without accounts it accepts any data and does
//! nothing, so a test can place arbitrary bytes in a transaction under a
//! program id that is not a precompile.

use anchor_lang::solana_program::account_info::AccountInfo;
use anchor_lang::solana_program::entrypoint::ProgramResult;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke;
use anchor_lang::solana_program::pubkey::Pubkey;

#[cfg(not(feature = "no-entrypoint"))]
anchor_lang::solana_program::entrypoint!(process_instruction);

pub fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let Some((target, passed)) = accounts.split_first() else {
        return Ok(());
    };
    let instruction = Instruction {
        program_id: *target.key,
        accounts: passed
            .iter()
            .map(|account| AccountMeta {
                pubkey: *account.key,
                is_signer: account.is_signer,
                is_writable: account.is_writable,
            })
            .collect(),
        data: data.to_vec(),
    };
    invoke(&instruction, accounts)
}
