//! Typed errors returned by the remote leg.

use anchor_lang::prelude::*;

#[error_code]
pub enum RemoteLegError {
    #[msg("signer may not perform this action")]
    Unauthorized,
    #[msg("remote leg is already initialized")]
    AlreadyInitialized,
    #[msg("remote leg is frozen")]
    Frozen,
    #[msg("remote leg is already frozen")]
    AlreadyFrozen,
    #[msg("authority key is not usable")]
    InvalidAuthority,
    #[msg("administrator and guardian must differ")]
    EqualAuthorities,
    #[msg("program account is not the expected program")]
    InvalidProgram,
    #[msg("account is owned by the wrong program")]
    InvalidAccountOwner,
    #[msg("address does not match its canonical seeds")]
    InvalidPda,
    #[msg("bump does not match the canonical bump")]
    InvalidBump,
    #[msg("mint does not match the configured asset")]
    InvalidMint,
    #[msg("mint decimals are not supported")]
    InvalidMintDecimals,
    #[msg("token program is not the supported one")]
    InvalidTokenProgram,
    #[msg("custody token account fails its policy")]
    InvalidCustodyAccount,
    #[msg("outbound escrow fails its policy")]
    InvalidOutboundEscrow,
    #[msg("source chain is not usable")]
    InvalidSourceDomain,
    #[msg("destination chain is not usable")]
    InvalidDestinationDomain,
    #[msg("application endpoint is not usable")]
    InvalidApplication,
    #[msg("deployment identifier is not usable")]
    InvalidDeployment,
    #[msg("vault identifier is not usable")]
    InvalidVault,
    #[msg("lane identifier is not usable")]
    InvalidLane,
    #[msg("configuration version is not usable")]
    InvalidConfigVersion,
    #[msg("arithmetic overflowed")]
    ArithmeticOverflow,
}
