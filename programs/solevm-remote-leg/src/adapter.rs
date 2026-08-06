//! The call interface every strategy adapter must expose.
//!
//! The leg builds these calls itself and only ever sends them to the adapter
//! stored in `StrategyConfig`. Nothing an adapter returns is trusted, so the
//! only value read back is the principal it keeps in its own account.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke_signed;

use crate::errors::RemoteLegError;

/// Selector of `deposit_exact`.
pub const DEPOSIT_EXACT: [u8; 8] = [103, 205, 71, 100, 215, 98, 254, 121];

/// Selector of `withdraw_for_remote_leg`.
pub const WITHDRAW_FOR_REMOTE_LEG: [u8; 8] = [140, 25, 238, 22, 233, 57, 60, 133];

/// Offset of the adapter principal inside its state account.
pub const PRINCIPAL_OFFSET: usize = 236;

/// Width of the adapter principal field.
pub const PRINCIPAL_LEN: usize = 8;

/// Accounts every adapter call shares, in their fixed order.
pub struct AdapterCall<'info> {
    pub adapter_program: AccountInfo<'info>,
    pub adapter_state: AccountInfo<'info>,
    pub adapter_authority: AccountInfo<'info>,
    pub adapter_token_vault: AccountInfo<'info>,
    pub custody_authority: AccountInfo<'info>,
    pub custody_token_account: AccountInfo<'info>,
    pub asset_mint: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
}

impl<'info> AdapterCall<'info> {
    /// Asks the adapter to take an exact amount out of custody.
    pub fn deposit_exact(&self, amount: u64, signer_seeds: &[&[u8]]) -> Result<()> {
        let metas = vec![
            AccountMeta::new_readonly(*self.custody_authority.key, true),
            AccountMeta::new(*self.adapter_state.key, false),
            AccountMeta::new(*self.custody_token_account.key, false),
            AccountMeta::new(*self.adapter_token_vault.key, false),
            AccountMeta::new_readonly(*self.asset_mint.key, false),
            AccountMeta::new_readonly(*self.token_program.key, false),
        ];
        let infos = [
            self.custody_authority.clone(),
            self.adapter_state.clone(),
            self.custody_token_account.clone(),
            self.adapter_token_vault.clone(),
            self.asset_mint.clone(),
            self.token_program.clone(),
        ];
        self.invoke(DEPOSIT_EXACT, amount, metas, &infos, signer_seeds)
    }

    /// Asks the adapter to return principal to custody.
    pub fn withdraw(&self, requested_principal: u64, signer_seeds: &[&[u8]]) -> Result<()> {
        let metas = vec![
            AccountMeta::new_readonly(*self.custody_authority.key, true),
            AccountMeta::new(*self.adapter_state.key, false),
            AccountMeta::new_readonly(*self.adapter_authority.key, false),
            AccountMeta::new(*self.custody_token_account.key, false),
            AccountMeta::new(*self.adapter_token_vault.key, false),
            AccountMeta::new_readonly(*self.asset_mint.key, false),
            AccountMeta::new_readonly(*self.token_program.key, false),
        ];
        let infos = [
            self.custody_authority.clone(),
            self.adapter_state.clone(),
            self.adapter_authority.clone(),
            self.custody_token_account.clone(),
            self.adapter_token_vault.clone(),
            self.asset_mint.clone(),
            self.token_program.clone(),
        ];
        self.invoke(
            WITHDRAW_FOR_REMOTE_LEG,
            requested_principal,
            metas,
            &infos,
            signer_seeds,
        )
    }

    fn invoke(
        &self,
        selector: [u8; 8],
        amount: u64,
        metas: Vec<AccountMeta>,
        infos: &[AccountInfo<'info>],
        signer_seeds: &[&[u8]],
    ) -> Result<()> {
        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&selector);
        data.extend_from_slice(&amount.to_le_bytes());

        let instruction = Instruction {
            program_id: *self.adapter_program.key,
            accounts: metas,
            data,
        };
        invoke_signed(&instruction, infos, &[signer_seeds])?;
        Ok(())
    }
}

/// Reads the principal the adapter records in its own account.
pub fn read_principal(adapter_state: &AccountInfo) -> Result<u64> {
    let data = adapter_state.try_borrow_data()?;
    let slot = data
        .get(PRINCIPAL_OFFSET..PRINCIPAL_OFFSET + PRINCIPAL_LEN)
        .ok_or(RemoteLegError::InvalidAdapterState)?;
    let bytes: [u8; PRINCIPAL_LEN] = slot
        .try_into()
        .map_err(|_| RemoteLegError::InvalidAdapterState)?;
    Ok(u64::from_le_bytes(bytes))
}
