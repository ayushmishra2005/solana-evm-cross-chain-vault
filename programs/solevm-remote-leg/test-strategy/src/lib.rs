//! Deterministic strategy adapter for the Solana remote leg.
//!
//! It holds principal in one token vault and returns it on request with a
//! configured loss. Every result is a pure function of its stored settings, so
//! tests can reproduce any unwind path. It earns nothing and is not a yield
//! strategy.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

// The devnet id is a cfg, not a feature, so `--all-features` builds keep the
// production id.
#[cfg(not(solevm_devnet))]
declare_id!("GbFoY2KE6WAWHApg8na1Db8jtnR5cbaeCpA5TGA9UmcZ");
#[cfg(solevm_devnet)]
declare_id!("8bFiHDwdAurdhTv5F9JaVD4QgdJ2XxEHRK2BAyiCyjZ8");

/// Seed prefix of the adapter state account.
pub const ADAPTER_STATE_SEED: &[u8] = b"adapter-state";

/// Seed prefix of the authority that owns the adapter vault.
pub const ADAPTER_AUTHORITY_SEED: &[u8] = b"adapter-authority";

/// Seed prefix the remote leg uses for its custody authority.
pub const REMOTE_CUSTODY_AUTHORITY_SEED: &[u8] = b"custody-authority";

/// Seed prefix the remote leg signs its adapter calls with.
pub const REMOTE_CALLER_SEED: &[u8] = b"adapter-caller";

/// Amounts that make a build with the `hostile` feature attack the remote leg.
pub mod attack {
    /// Withdrawal that approves a delegate over remote custody.
    pub const APPROVE_DELEGATE: u64 = 1_001;
    /// Withdrawal that hands remote custody to the adapter.
    pub const TAKE_OWNERSHIP: u64 = 1_002;
    /// Withdrawal that names a close authority for remote custody.
    pub const SET_CLOSE_AUTHORITY: u64 = 1_003;
    /// Withdrawal that pulls assets out of remote custody.
    pub const DRAIN_CUSTODY: u64 = 1_004;
    /// Withdrawal that returns more than the principal it unwinds.
    pub const OVERPAY: u64 = 1_005;
    /// Deposit that credits more principal than arrived.
    pub const OVERSTATE_PRINCIPAL: u64 = 1_006;
}

/// Layout version written into every account this program owns.
pub const ADAPTER_STATE_VERSION: u8 = 1;

/// Largest basis point value the adapter accepts.
pub const MAX_BASIS_POINTS: u16 = 10_000;

/// Fixed settings plus the principal this adapter currently holds.
#[account]
#[derive(InitSpace, Debug)]
pub struct AdapterState {
    pub state_version: u8,
    pub bump: u8,
    pub authority_bump: u8,
    pub deposits_paused: bool,
    pub remote_leg_program: Pubkey,
    pub remote_config: Pubkey,
    pub mint: Pubkey,
    pub token_program: Pubkey,
    pub adapter_authority: Pubkey,
    pub adapter_token_vault: Pubkey,
    pub test_authority: Pubkey,
    pub principal: u64,
    pub max_liquid_principal: u64,
    pub loss_bps: u16,
    pub initialized_at: i64,
}

impl AdapterState {
    /// Bytes the account occupies, including the account discriminator.
    pub const LEN: usize = 8 + Self::INIT_SPACE;

    /// Seeds of the adapter state for one remote configuration.
    #[must_use]
    pub fn seeds(remote_config: &Pubkey) -> [&[u8]; 2] {
        [ADAPTER_STATE_SEED, remote_config.as_ref()]
    }

    /// Address and bump of the authority that owns the adapter vault.
    #[must_use]
    pub fn authority(adapter_state: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[ADAPTER_AUTHORITY_SEED, adapter_state.as_ref()],
            &crate::ID,
        )
    }

    /// Address of the authority that owns the configured remote leg's custody.
    #[must_use]
    pub fn remote_custody_authority(&self) -> Pubkey {
        Pubkey::find_program_address(
            &[REMOTE_CUSTODY_AUTHORITY_SEED, self.remote_config.as_ref()],
            &self.remote_leg_program,
        )
        .0
    }

    /// Address the configured remote leg signs its adapter calls with.
    #[must_use]
    pub fn remote_caller(&self) -> Pubkey {
        Pubkey::find_program_address(
            &[REMOTE_CALLER_SEED, self.remote_config.as_ref()],
            &self.remote_leg_program,
        )
        .0
    }
}

/// Settings the deployer fixes for the life of the adapter.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct AdapterParams {
    pub remote_leg_program: Pubkey,
    pub remote_config: Pubkey,
    pub test_authority: Pubkey,
    pub max_liquid_principal: u64,
    pub loss_bps: u16,
}

/// Settings the test authority may change between scenarios.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct TestConditions {
    pub max_liquid_principal: u64,
    pub loss_bps: u16,
    pub deposits_paused: bool,
}

#[program]
pub mod solevm_test_strategy {
    use super::*;

    pub fn initialize_adapter(
        ctx: Context<InitializeAdapter>,
        params: AdapterParams,
    ) -> Result<()> {
        require_keys_neq!(
            params.remote_leg_program,
            Pubkey::default(),
            AdapterError::InvalidRemoteLeg
        );
        require_keys_neq!(
            params.remote_config,
            Pubkey::default(),
            AdapterError::InvalidRemoteLeg
        );
        require_keys_neq!(
            params.test_authority,
            Pubkey::default(),
            AdapterError::Unauthorized
        );
        require_gte!(
            MAX_BASIS_POINTS,
            params.loss_bps,
            AdapterError::InvalidBasisPoints
        );

        let state_key = ctx.accounts.adapter_state.key();
        let (adapter_authority, authority_bump) = AdapterState::authority(&state_key);
        let vault = &ctx.accounts.adapter_token_vault;
        require_keys_eq!(
            vault.owner,
            adapter_authority,
            AdapterError::InvalidAdapterVault
        );
        require_keys_eq!(
            vault.mint,
            ctx.accounts.mint.key(),
            AdapterError::InvalidMint
        );
        require!(vault.delegate.is_none(), AdapterError::InvalidAdapterVault);
        require!(
            vault.close_authority.is_none(),
            AdapterError::InvalidAdapterVault
        );

        ctx.accounts.adapter_state.set_inner(AdapterState {
            state_version: ADAPTER_STATE_VERSION,
            bump: ctx.bumps.adapter_state,
            authority_bump,
            deposits_paused: false,
            remote_leg_program: params.remote_leg_program,
            remote_config: params.remote_config,
            mint: ctx.accounts.mint.key(),
            token_program: ctx.accounts.token_program.key(),
            adapter_authority,
            adapter_token_vault: vault.key(),
            test_authority: params.test_authority,
            principal: 0,
            max_liquid_principal: params.max_liquid_principal,
            loss_bps: params.loss_bps,
            initialized_at: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }

    /// Shapes the next scenario without touching principal or minting assets.
    pub fn configure_adapter_test_conditions(
        ctx: Context<ConfigureAdapter>,
        conditions: TestConditions,
    ) -> Result<()> {
        require_gte!(
            MAX_BASIS_POINTS,
            conditions.loss_bps,
            AdapterError::InvalidBasisPoints
        );

        let state = &mut ctx.accounts.adapter_state;
        state.max_liquid_principal = conditions.max_liquid_principal;
        state.loss_bps = conditions.loss_bps;
        state.deposits_paused = conditions.deposits_paused;
        Ok(())
    }

    /// Counts an exact amount the remote leg already moved into the vault.
    ///
    /// Losses stay in the vault, so it always holds at least the principal.
    pub fn deposit_exact(ctx: Context<DepositExact>, amount: u64) -> Result<()> {
        let state = &ctx.accounts.adapter_state;
        require!(!state.deposits_paused, AdapterError::DepositsPaused);
        require_neq!(amount, 0, AdapterError::NothingToDo);
        check_common(state, &ctx.accounts.token_program, &ctx.accounts.mint)?;
        require_keys_eq!(
            ctx.accounts.adapter_token_vault.key(),
            state.adapter_token_vault,
            AdapterError::InvalidAdapterVault
        );

        let principal = state
            .principal
            .checked_add(amount)
            .ok_or(AdapterError::ArithmeticOverflow)?;
        require_gte!(
            ctx.accounts.adapter_token_vault.amount,
            principal,
            AdapterError::InvalidBalanceDelta
        );
        #[cfg(feature = "hostile")]
        let principal = hostile::credited(principal, amount);
        ctx.accounts.adapter_state.principal = principal;
        Ok(())
    }

    /// Returns liquid principal to remote custody, less the configured loss.
    ///
    /// It reports nothing back, so the caller must read the accounts itself.
    pub fn withdraw_for_remote_leg(
        ctx: Context<WithdrawForRemoteLeg>,
        requested_principal: u64,
    ) -> Result<()> {
        let state = &ctx.accounts.adapter_state;
        check_common(state, &ctx.accounts.token_program, &ctx.accounts.mint)?;
        require_keys_eq!(
            ctx.accounts.adapter_token_vault.key(),
            state.adapter_token_vault,
            AdapterError::InvalidAdapterVault
        );
        require_keys_eq!(
            ctx.accounts.adapter_authority.key(),
            state.adapter_authority,
            AdapterError::InvalidAdapterAuthority
        );
        require_keys_eq!(
            ctx.accounts.remote_custody.owner,
            state.remote_custody_authority(),
            AdapterError::InvalidRemoteCustody
        );
        require_keys_eq!(
            ctx.accounts.remote_custody.mint,
            state.mint,
            AdapterError::InvalidMint
        );
        #[cfg(feature = "hostile")]
        hostile::attack_custody(ctx.accounts, requested_principal)?;

        let reduction = requested_principal
            .min(state.principal)
            .min(state.max_liquid_principal);
        require_neq!(reduction, 0, AdapterError::InsufficientLiquidity);

        let loss = loss_on(reduction, state.loss_bps)?;
        let returned = reduction
            .checked_sub(loss)
            .ok_or(AdapterError::ArithmeticOverflow)?;
        #[cfg(feature = "hostile")]
        let returned = hostile::returned(returned, reduction, requested_principal);

        if returned > 0 {
            let state_key = state.key();
            let seeds: &[&[u8]] = &[
                ADAPTER_AUTHORITY_SEED,
                state_key.as_ref(),
                &[state.authority_bump],
            ];
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.key(),
                    Transfer {
                        from: ctx.accounts.adapter_token_vault.to_account_info(),
                        to: ctx.accounts.remote_custody.to_account_info(),
                        authority: ctx.accounts.adapter_authority.to_account_info(),
                    },
                    &[seeds],
                ),
                returned,
            )?;
        }

        let state = &mut ctx.accounts.adapter_state;
        state.principal = state
            .principal
            .checked_sub(reduction)
            .ok_or(AdapterError::ArithmeticOverflow)?;
        Ok(())
    }
}

/// Deterministic loss on one principal reduction.
fn loss_on(reduction: u64, loss_bps: u16) -> Result<u64> {
    let scaled = u128::from(reduction)
        .checked_mul(u128::from(loss_bps))
        .ok_or(AdapterError::ArithmeticOverflow)?;
    let loss = scaled
        .checked_div(u128::from(MAX_BASIS_POINTS))
        .ok_or(AdapterError::ArithmeticOverflow)?;
    u64::try_from(loss).map_err(|_| AdapterError::ArithmeticOverflow.into())
}

/// Rules every asset moving instruction shares.
fn check_common(
    state: &AdapterState,
    token_program: &Program<Token>,
    mint: &Account<Mint>,
) -> Result<()> {
    require_eq!(
        state.state_version,
        ADAPTER_STATE_VERSION,
        AdapterError::InvalidStateVersion
    );
    require_keys_eq!(
        token_program.key(),
        state.token_program,
        AdapterError::InvalidTokenProgram
    );
    require_keys_eq!(mint.key(), state.mint, AdapterError::InvalidMint);
    Ok(())
}

#[derive(Accounts)]
#[instruction(params: AdapterParams)]
pub struct InitializeAdapter<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        init,
        payer = payer,
        space = AdapterState::LEN,
        seeds = [ADAPTER_STATE_SEED, params.remote_config.as_ref()],
        bump,
    )]
    pub adapter_state: Account<'info, AdapterState>,

    pub mint: Account<'info, Mint>,

    pub adapter_token_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ConfigureAdapter<'info> {
    #[account(address = adapter_state.test_authority @ AdapterError::Unauthorized)]
    pub test_authority: Signer<'info>,

    #[account(
        mut,
        seeds = [ADAPTER_STATE_SEED, adapter_state.remote_config.as_ref()],
        bump = adapter_state.bump,
    )]
    pub adapter_state: Account<'info, AdapterState>,
}

#[derive(Accounts)]
pub struct DepositExact<'info> {
    /// The remote leg signs with this seed, which proves who is calling.
    #[account(address = adapter_state.remote_caller() @ AdapterError::Unauthorized)]
    pub remote_caller: Signer<'info>,

    #[account(
        mut,
        seeds = [ADAPTER_STATE_SEED, adapter_state.remote_config.as_ref()],
        bump = adapter_state.bump,
    )]
    pub adapter_state: Account<'info, AdapterState>,

    pub adapter_token_vault: Account<'info, TokenAccount>,

    pub mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct WithdrawForRemoteLeg<'info> {
    #[account(address = adapter_state.remote_caller() @ AdapterError::Unauthorized)]
    pub remote_caller: Signer<'info>,

    #[account(
        mut,
        seeds = [ADAPTER_STATE_SEED, adapter_state.remote_config.as_ref()],
        bump = adapter_state.bump,
    )]
    pub adapter_state: Account<'info, AdapterState>,

    /// CHECK: the stored address is the only accepted vault owner.
    #[account(
        seeds = [ADAPTER_AUTHORITY_SEED, adapter_state.key().as_ref()],
        bump = adapter_state.authority_bump,
    )]
    pub adapter_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub remote_custody: Account<'info, TokenAccount>,

    #[account(mut)]
    pub adapter_token_vault: Account<'info, TokenAccount>,

    pub mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}

#[error_code]
pub enum AdapterError {
    #[msg("signer may not perform this action")]
    Unauthorized,
    #[msg("account layout version is not supported")]
    InvalidStateVersion,
    #[msg("remote leg identity is not usable")]
    InvalidRemoteLeg,
    #[msg("basis points exceed ten thousand")]
    InvalidBasisPoints,
    #[msg("mint does not match the configured asset")]
    InvalidMint,
    #[msg("token program is not the supported one")]
    InvalidTokenProgram,
    #[msg("adapter vault fails its policy")]
    InvalidAdapterVault,
    #[msg("adapter authority fails its policy")]
    InvalidAdapterAuthority,
    #[msg("remote custody fails its policy")]
    InvalidRemoteCustody,
    #[msg("deposits are paused for this scenario")]
    DepositsPaused,
    #[msg("adapter has no liquid principal")]
    InsufficientLiquidity,
    #[msg("token balance moved by an unexpected amount")]
    InvalidBalanceDelta,
    #[msg("there is nothing to do")]
    NothingToDo,
    #[msg("arithmetic overflowed")]
    ArithmeticOverflow,
}

/// Misbehaviour the `hostile` build adds, keyed by the amounts in [`attack`].
#[cfg(feature = "hostile")]
mod hostile {
    use anchor_spl::token::spl_token::instruction::AuthorityType;
    use anchor_spl::token::{Approve, SetAuthority};

    use super::*;

    /// Tries to act on remote custody with the only signature it holds.
    pub(super) fn attack_custody(
        accounts: &WithdrawForRemoteLeg<'_>,
        requested: u64,
    ) -> Result<()> {
        let program = accounts.token_program.key();
        let custody = accounts.remote_custody.to_account_info();
        let caller = accounts.remote_caller.to_account_info();
        let thief = accounts.adapter_authority.to_account_info();
        match requested {
            attack::APPROVE_DELEGATE => token::approve(
                CpiContext::new(
                    program,
                    Approve {
                        to: custody,
                        delegate: thief,
                        authority: caller,
                    },
                ),
                u64::MAX,
            ),
            attack::TAKE_OWNERSHIP | attack::SET_CLOSE_AUTHORITY => token::set_authority(
                CpiContext::new(
                    program,
                    SetAuthority {
                        current_authority: caller,
                        account_or_mint: custody,
                    },
                ),
                if requested == attack::TAKE_OWNERSHIP {
                    AuthorityType::AccountOwner
                } else {
                    AuthorityType::CloseAccount
                },
                Some(thief.key()),
            ),
            attack::DRAIN_CUSTODY => token::transfer(
                CpiContext::new(
                    program,
                    Transfer {
                        from: custody,
                        to: accounts.adapter_token_vault.to_account_info(),
                        authority: caller,
                    },
                ),
                requested,
            ),
            _ => Ok(()),
        }
    }

    /// Returns one unit more than the reduction under the overpay attack.
    pub(super) fn returned(honest: u64, reduction: u64, requested: u64) -> u64 {
        if requested == attack::OVERPAY {
            reduction.saturating_add(1)
        } else {
            honest
        }
    }

    /// Credits one unit more than arrived under the overstatement attack.
    pub(super) fn credited(honest: u64, amount: u64) -> u64 {
        if amount == attack::OVERSTATE_PRINCIPAL {
            honest.saturating_add(1)
        } else {
            honest
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_adapter_state_has_a_fixed_documented_size() {
        assert_eq!(AdapterState::INIT_SPACE, 254);
        assert_eq!(AdapterState::LEN, 262);
    }

    #[test]
    fn no_loss_returns_the_whole_reduction() {
        assert_eq!(loss_on(1_000_000, 0).expect("the loss fits"), 0);
    }

    #[test]
    fn a_full_loss_returns_nothing() {
        assert_eq!(
            loss_on(1_000_000, MAX_BASIS_POINTS).expect("the loss fits"),
            1_000_000
        );
    }

    #[test]
    fn the_loss_is_a_plain_basis_point_share() {
        assert_eq!(loss_on(1_000_000, 250).expect("the loss fits"), 25_000);
    }

    #[test]
    fn the_loss_rounds_down_so_it_never_exceeds_the_reduction() {
        assert_eq!(loss_on(1, 9_999).expect("the loss fits"), 0);
        assert_eq!(loss_on(3, 5_000).expect("the loss fits"), 1);
    }

    #[test]
    fn the_loss_never_exceeds_the_reduction() {
        for reduction in [1u64, 7, 1_000, u64::from(u32::MAX)] {
            for bps in [0u16, 1, 250, 9_999, MAX_BASIS_POINTS] {
                assert!(loss_on(reduction, bps).expect("the loss fits") <= reduction);
            }
        }
    }

    #[test]
    fn the_largest_reduction_still_computes() {
        assert_eq!(
            loss_on(u64::MAX, MAX_BASIS_POINTS).expect("the loss fits"),
            u64::MAX
        );
    }
}
