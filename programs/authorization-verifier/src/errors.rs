use anchor_lang::prelude::*;

#[error_code]
pub enum VerifierError {
    #[msg("Signer is not authorized for this action")]
    Unauthorized,
    #[msg("Administrator and consumer must be distinct, non-default keys")]
    InvalidRoles,
    #[msg("Key id is outside the profile or the environment's namespace")]
    InvalidKeyId,
    #[msg("Public key is not a compressed SEC1 point")]
    InvalidPublicKey,
    #[msg("Key validity window is empty or negative")]
    InvalidKeyWindow,
    #[msg("Mandate fields are outside the profile")]
    InvalidMandate,
    #[msg("Mandate hash does not match the mandate id")]
    MandateHashMismatch,
    #[msg("Account is already revoked")]
    AlreadyRevoked,

    #[msg("Consume must be a top-level instruction of this program")]
    NotTopLevel,
    #[msg("Transaction instructions are not in the required order")]
    InvalidInstructionOrder,
    #[msg("The preceding instruction is not the secp256r1 precompile")]
    MissingPrecompile,
    #[msg("Precompile instruction is not in the canonical layout")]
    InvalidPrecompileInstruction,
    #[msg("Signing input length is outside the profile")]
    InvalidSigningInputLength,

    #[msg("Signing input is not two Base64URL segments")]
    MalformedSigningInput,
    #[msg("Segment is not canonical unpadded Base64URL")]
    InvalidBase64Url,
    #[msg("JSON is malformed or outside the profile grammar")]
    MalformedJson,
    #[msg("JSON value type is not supported for this field")]
    UnsupportedJsonValue,
    #[msg("JSON object repeats a field")]
    DuplicateField,
    #[msg("JSON object contains a field outside the profile")]
    UnknownField,
    #[msg("Header or claim is unsupported by this profile")]
    UnsupportedClaim,
    #[msg("A required field is missing")]
    MissingField,
    #[msg("Field length is outside the profile")]
    InvalidFieldLength,
    #[msg("Number is outside the profile")]
    InvalidNumber,
    #[msg("Algorithm must be ES256")]
    UnsupportedAlgorithm,
    #[msg("Type must be JWT when present")]
    UnsupportedType,

    #[msg("Token kid does not match the trusted key")]
    KeyIdMismatch,
    #[msg("Precompile public key does not match the trusted key")]
    PublicKeyMismatch,
    #[msg("Trusted key is revoked")]
    KeyRevoked,
    #[msg("Token lifetime or current time lies outside the key validity window")]
    OutsideKeyWindow,
    #[msg("Issuer is not accepted")]
    IssuerMismatch,
    #[msg("Verdict is not allow")]
    VerdictNotAllow,
    #[msg("Expiry must be exactly 300 seconds after issue")]
    InvalidLifetime,
    #[msg("Token was issued too far in the future")]
    IssuedInFuture,
    #[msg("Token has expired")]
    Expired,

    #[msg("Mandate is revoked")]
    MandateRevoked,
    #[msg("Signed mandate does not match the approved mandate")]
    MandateMismatch,
    #[msg("Signed amount does not match the approved mandate")]
    AmountMismatch,
    #[msg("Signed currency does not match the approved mandate")]
    CurrencyMismatch,
    #[msg("Signed recipient does not match the approved mandate")]
    RecipientMismatch,
    #[msg("Signed policy version does not match the approved mandate")]
    PolicyMismatch,
    #[msg("Signed agent does not match the approved mandate")]
    AgentMismatch,
    #[msg("Signed category does not match the approved mandate")]
    CategoryMismatch,
    #[msg("Nonce hash argument does not match the signed nonce")]
    NonceHashMismatch,

    #[msg("Deployment authority parameters are outside the profile")]
    InvalidDeploymentAuthority,
    #[msg("Signed message is not a SolEVM deployment authorization")]
    InvalidAuthorizationMessage,
    #[msg("Signed operation is not supported by this profile")]
    UnsupportedOperation,
    #[msg("Signed authorization names another deployment authority")]
    AuthorityMismatch,
    #[msg("Signed network domain does not match the deployment authority")]
    NetworkDomainMismatch,
    #[msg("Signed policy generation is not the current one")]
    PolicyGenerationMismatch,
    #[msg("Expiry must follow issue by at most 300 seconds")]
    InvalidAuthorizationLifetime,
    #[msg("Signed amount must be positive")]
    InvalidAuthorizationAmount,
    #[msg("Nonce argument does not match the signed nonce")]
    NonceMismatch,
    #[msg("Consume is not immediately followed by a deployment")]
    MissingDeploymentInstruction,
    #[msg("Following instruction is not the authorized deployment")]
    InvalidDeploymentInstruction,
    #[msg("Policy generation cannot advance further")]
    PolicyGenerationExhausted,
}
