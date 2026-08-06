//! Deterministic strategy adapter for the Solana remote leg.
//!
//! It holds principal in one token vault and returns it on request with a
//! configured loss. Every result is a pure function of its stored settings, so
//! tests can reproduce any unwind path. It earns nothing and is not a yield
//! strategy.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("GbFoY2KE6WAWHApg8na1Db8jtnR5cbaeCpA5TGA9UmcZ");

/// Seed prefix of the adapter state account.
pub const ADAPTER_STATE_SEED: &[u8] = b"adapter-state";

/// Seed prefix of the authority that owns the adapter vault.
pub const ADAPTER_AUTHORITY_SEED: &[u8] = b"adapter-authority";

/// Seed prefix the remote leg uses for its custody authority.
pub const REMOTE_CUSTODY_AUTHORITY_SEED: &[u8] = b"custody-authority";

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

    /// Address of the custody authority the configured remote leg signs with.
    #[must_use]
    pub fn remote_custody_authority(&self) -> Pubkey {
        Pubkey::find_program_address(
            &[REMOTE_CUSTODY_AUTHORITY_SEED, self.remote_config.as_ref()],
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

    /// Takes assets from remote custody and counts only what arrived.
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
        require_keys_eq!(
            ctx.accounts.remote_custody.owner,
            ctx.accounts.remote_custody_authority.key(),
            AdapterError::InvalidRemoteCustody
        );
        require_keys_eq!(
            ctx.accounts.remote_custody.mint,
            state.mint,
            AdapterError::InvalidMint
        );

        let before = ctx.accounts.adapter_token_vault.amount;
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.remote_custody.to_account_info(),
                    to: ctx.accounts.adapter_token_vault.to_account_info(),
                    authority: ctx.accounts.remote_custody_authority.to_account_info(),
                },
            ),
            amount,
        )?;

        ctx.accounts.adapter_token_vault.reload()?;
        let received = ctx
            .accounts
            .adapter_token_vault
            .amount
            .checked_sub(before)
            .ok_or(AdapterError::InvalidBalanceDelta)?;
        require_eq!(received, amount, AdapterError::InvalidBalanceDelta);

        let state = &mut ctx.accounts.adapter_state;
        state.principal = state
            .principal
            .checked_add(received)
            .ok_or(AdapterError::ArithmeticOverflow)?;
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
            ctx.accounts.remote_custody_authority.key(),
            AdapterError::InvalidRemoteCustody
        );
        require_keys_eq!(
            ctx.accounts.remote_custody.mint,
            state.mint,
            AdapterError::InvalidMint
        );

        let reduction = requested_principal
            .min(state.principal)
            .min(state.max_liquid_principal);
        require_neq!(reduction, 0, AdapterError::InsufficientLiquidity);

        let loss = loss_on(reduction, state.loss_bps)?;
        let returned = reduction
            .checked_sub(loss)
            .ok_or(AdapterError::ArithmeticOverflow)?;

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
    #[account(address = adapter_state.remote_custody_authority() @ AdapterError::Unauthorized)]
    pub remote_custody_authority: Signer<'info>,

    #[account(
        mut,
        seeds = [ADAPTER_STATE_SEED, adapter_state.remote_config.as_ref()],
        bump = adapter_state.bump,
    )]
    pub adapter_state: Account<'info, AdapterState>,

    #[account(mut)]
    pub remote_custody: Account<'info, TokenAccount>,

    #[account(mut)]
    pub adapter_token_vault: Account<'info, TokenAccount>,

    pub mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct WithdrawForRemoteLeg<'info> {
    #[account(address = adapter_state.remote_custody_authority() @ AdapterError::Unauthorized)]
    pub remote_custody_authority: Signer<'info>,

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
