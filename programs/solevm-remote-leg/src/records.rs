//! Safe manual creation of the fixed size records this program owns.
//!
//! Anchor would look the address up before the handler runs, which would let a
//! missing record answer before the watermark does. Creating by hand keeps the
//! documented validation order.

use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Allocate, Assign, Transfer};

use crate::errors::RemoteLegError;

/// Rejects anything that is not an empty system owned account.
///
/// Lamports alone are allowed, so a stranger cannot block a valid message.
pub fn check_available(
    info: &AccountInfo,
    already_used: RemoteLegError,
    invalid: RemoteLegError,
) -> Result<()> {
    if info.owner == &crate::ID {
        return Err(already_used.into());
    }
    if info.owner != &system_program::ID || !info.data_is_empty() {
        return Err(invalid.into());
    }
    Ok(())
}

/// Funds, allocates and assigns the address, then writes the account.
pub fn create_and_write<'info, T>(
    target: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program_account: &AccountInfo<'info>,
    signer_seeds: &[&[u8]],
    space: usize,
    value: &T,
) -> Result<()>
where
    T: AccountSerialize,
{
    let signer = &[signer_seeds];

    let required = Rent::get()?.minimum_balance(space);
    let current = target.lamports();
    if current < required {
        let missing = required
            .checked_sub(current)
            .ok_or(RemoteLegError::ArithmeticOverflow)?;
        system_program::transfer(
            CpiContext::new(
                *system_program_account.key,
                Transfer {
                    from: payer.clone(),
                    to: target.clone(),
                },
            ),
            missing,
        )?;
    }

    let width = u64::try_from(space).map_err(|_| RemoteLegError::ArithmeticOverflow)?;
    system_program::allocate(
        CpiContext::new_with_signer(
            *system_program_account.key,
            Allocate {
                account_to_allocate: target.clone(),
            },
            signer,
        ),
        width,
    )?;
    system_program::assign(
        CpiContext::new_with_signer(
            *system_program_account.key,
            Assign {
                account_to_assign: target.clone(),
            },
            signer,
        ),
        &crate::ID,
    )?;

    let mut data = target.try_borrow_mut_data()?;
    let mut slot: &mut [u8] = &mut data;
    value.try_serialize(&mut slot)?;
    Ok(())
}
