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
    #[msg("control state is already initialized")]
    ControlStateAlreadyInitialized,
    #[msg("account layout version is not supported")]
    InvalidStateVersion,
    #[msg("risk configuration fails its policy")]
    InvalidRiskConfig,
    #[msg("basis points exceed ten thousand")]
    InvalidBasisPoints,
    #[msg("report age is not usable")]
    InvalidReportAge,
    #[msg("configuration commitment is not usable")]
    InvalidConfigCommitment,
    #[msg("message bytes are not canonical")]
    InvalidMessage,
    #[msg("message is larger than the protocol maximum")]
    MessageTooLarge,
    #[msg("message type is not accepted here")]
    UnsupportedMessageType,
    #[msg("protocol version is not supported")]
    InvalidProtocolVersion,
    #[msg("schema version is not supported")]
    InvalidSchemaVersion,
    #[msg("message timestamp is not usable")]
    InvalidTimestamp,
    #[msg("message has expired")]
    MessageExpired,
    #[msg("sequence is not the expected next one")]
    InvalidSequence,
    #[msg("sequence is below the replay watermark")]
    SequenceBelowWatermark,
    #[msg("previous commitment does not match the lane")]
    InvalidPreviousCommitment,
    #[msg("message was already consumed")]
    ReplayDetected,
    #[msg("consumed message record fails its policy")]
    InvalidConsumedMessage,
    #[msg("configuration is not effective yet")]
    ConfigNotEffective,
    #[msg("watermark value is not usable")]
    InvalidWatermark,
    #[msg("watermark would break the mandatory lag")]
    WatermarkLagViolation,
    #[msg("record may not be closed yet")]
    RecordNotClosable,
    #[msg("rent destination is not the administrator")]
    InvalidRentDestination,
    #[msg("strategy state is already initialized")]
    StrategyStateAlreadyInitialized,
    #[msg("strategy configuration fails its policy")]
    InvalidStrategyConfig,
    #[msg("remote position fails its policy")]
    InvalidRemotePosition,
    #[msg("adapter program is not the configured one")]
    InvalidAdapterProgram,
    #[msg("adapter state fails its policy")]
    InvalidAdapterState,
    #[msg("adapter authority fails its policy")]
    InvalidAdapterAuthority,
    #[msg("adapter vault fails its policy")]
    InvalidAdapterVault,
    #[msg("transfer record fails its policy")]
    InvalidTransferRecord,
    #[msg("transfer kind is not the expected one")]
    InvalidTransferKind,
    #[msg("transfer status is not the expected one")]
    InvalidTransferStatus,
    #[msg("transfer id already has a record")]
    TransferAlreadyExists,
    #[msg("transfer id has no record")]
    TransferNotFound,
    #[msg("another transfer cycle is still unresolved")]
    UnresolvedCycle,
    #[msg("there is no active transfer of this kind")]
    NoActiveTransfer,
    #[msg("allocation attribution is not complete")]
    AllocationIncomplete,
    #[msg("attribution would exceed the authorized amount")]
    AttributionExceedsAuthorization,
    #[msg("no custody assets are waiting for attribution")]
    NoAttributableAssets,
    #[msg("allocation exceeds the permitted remote principal")]
    RemoteAllocationLimitExceeded,
    #[msg("expected source balance is not usable")]
    InvalidExpectedSourceBalance,
    #[msg("minimum destination amount is not usable")]
    InvalidMinimumDestinationAmount,
    #[msg("attributed custody principal is too small")]
    InsufficientAttributedCustody,
    #[msg("remote principal is smaller than the request")]
    InsufficientRemotePrincipal,
    #[msg("strategy returned no liquid principal")]
    InsufficientStrategyLiquidity,
    #[msg("token balance moved by an unexpected amount")]
    InvalidBalanceDelta,
    #[msg("adapter principal moved by an unexpected amount")]
    InvalidPrincipalDelta,
    #[msg("realized loss is not usable")]
    InvalidRealizedLoss,
    #[msg("recall amount is not usable")]
    InvalidRecallAmount,
    #[msg("minimum return amount is not usable")]
    InvalidMinimumReturn,
    #[msg("no recalled custody is waiting to be sent")]
    NoRecalledCustody,
    #[msg("an unresolved transfer blocks the watermark")]
    FinancialObligationBlocksWatermark,
    #[msg("custody holds less than the accounted total")]
    AccountingDeficit,
    #[msg("amount does not fit the token amount type")]
    AmountTooLarge,
}
