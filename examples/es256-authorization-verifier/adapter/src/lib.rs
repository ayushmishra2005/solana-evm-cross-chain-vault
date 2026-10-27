//! Host-side helpers that turn an ES256 compact JWS into the secp256r1
//! precompile instruction the verifier expects.
//!
//! Nothing here is trusted by the program. The adapter only reshapes public
//! data: it compresses the JWK point, keeps the signing input byte for byte,
//! and rewrites a high-S signature to its low-S twin, which verifies the same
//! message under the same key.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use openssl::bn::{BigNum, BigNumContext};
use openssl::ec::{EcGroup, EcKey, EcPoint, PointConversionForm};
use openssl::ecdsa::EcdsaSig;
use openssl::nid::Nid;
use openssl::pkey::Private;
use solana_instruction::Instruction;
use solana_secp256r1_program::{
    COMPRESSED_PUBKEY_SERIALIZED_SIZE, DATA_START, FIELD_SIZE, SECP256R1_HALF_ORDER,
    SECP256R1_ORDER, SIGNATURE_SERIALIZED_SIZE,
};

pub const COMPRESSED_PUBLIC_KEY_LEN: usize = COMPRESSED_PUBKEY_SERIALIZED_SIZE;
pub const SIGNATURE_LEN: usize = SIGNATURE_SERIALIZED_SIZE;
pub const PUBLIC_KEY_OFFSET: usize = DATA_START;
pub const SIGNATURE_OFFSET: usize = PUBLIC_KEY_OFFSET + COMPRESSED_PUBLIC_KEY_LEN;
pub const MESSAGE_OFFSET: usize = SIGNATURE_OFFSET + SIGNATURE_LEN;
/// Matches the verifier's limit on the signing input.
pub const MAX_SIGNING_INPUT_LEN: usize = 512;

const CURRENT_INSTRUCTION: u16 = u16::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterError {
    InvalidBase64Url,
    InvalidCompactJws,
    InvalidCoordinate,
    PointNotOnCurve,
    ScalarOutOfRange,
    InvalidSigningInputLength,
    Crypto,
}

impl From<openssl::error::ErrorStack> for AdapterError {
    fn from(_: openssl::error::ErrorStack) -> Self {
        Self::Crypto
    }
}

pub type AdapterResult<T> = Result<T, AdapterError>;

pub fn base64url_encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Canonical unpadded Base64URL; padding and non-zero trailing bits fail.
pub fn base64url_decode(text: &str) -> AdapterResult<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| AdapterError::InvalidBase64Url)
}

/// `ASCII(BASE64URL(header) "." BASE64URL(payload))`, the bytes ES256 signs.
pub fn signing_input(header_json: &[u8], payload_json: &[u8]) -> Vec<u8> {
    format!(
        "{}.{}",
        base64url_encode(header_json),
        base64url_encode(payload_json)
    )
    .into_bytes()
}

/// Splits a compact JWS into its signing input, unchanged, and its raw
/// `r || s` signature.
pub fn split_compact_jws(token: &str) -> AdapterResult<(Vec<u8>, [u8; SIGNATURE_LEN])> {
    let (signing_input, signature) = token
        .rsplit_once('.')
        .ok_or(AdapterError::InvalidCompactJws)?;
    if signing_input.matches('.').count() != 1 {
        return Err(AdapterError::InvalidCompactJws);
    }
    let signature = <[u8; SIGNATURE_LEN]>::try_from(base64url_decode(signature)?)
        .map_err(|_| AdapterError::InvalidCompactJws)?;
    Ok((signing_input.as_bytes().to_vec(), signature))
}

/// Converts a P-256 JWK's `x` and `y` into a 33-byte compressed SEC1 point,
/// refusing anything that is not a point on the curve.
pub fn compress_jwk_public_key(x: &str, y: &str) -> AdapterResult<[u8; COMPRESSED_PUBLIC_KEY_LEN]> {
    let x = base64url_decode(x)?;
    let y = base64url_decode(y)?;
    if x.len() != FIELD_SIZE || y.len() != FIELD_SIZE {
        return Err(AdapterError::InvalidCoordinate);
    }
    let mut uncompressed = vec![0x04];
    uncompressed.extend_from_slice(&x);
    uncompressed.extend_from_slice(&y);

    let group = p256_group()?;
    let mut context = BigNumContext::new()?;
    let point = EcPoint::from_bytes(&group, &uncompressed, &mut context)
        .map_err(|_| AdapterError::PointNotOnCurve)?;
    if !point.is_on_curve(&group, &mut context)? {
        return Err(AdapterError::PointNotOnCurve);
    }
    compressed(&group, &point)
}

/// Rejects scalars outside `1..n` and returns the low-S form the native
/// precompile requires. Already-low signatures are returned unchanged.
pub fn normalize_signature(signature: &[u8; SIGNATURE_LEN]) -> AdapterResult<[u8; SIGNATURE_LEN]> {
    let (r, s) = signature.split_at(FIELD_SIZE);
    let r = BigNum::from_slice(r)?;
    let s = BigNum::from_slice(s)?;
    let order = BigNum::from_slice(&SECP256R1_ORDER)?;
    let half_order = BigNum::from_slice(&SECP256R1_HALF_ORDER)?;
    let one = BigNum::from_u32(1)?;
    if r < one || r >= order || s < one || s >= order {
        return Err(AdapterError::ScalarOutOfRange);
    }
    if s <= half_order {
        return Ok(*signature);
    }
    let mut low_s = BigNum::new()?;
    low_s.checked_sub(&order, &s)?;
    raw_signature(&r, &low_s)
}

/// Whether the precompile would accept this `s` without normalization.
pub fn is_low_s(signature: &[u8; SIGNATURE_LEN]) -> AdapterResult<bool> {
    let (_, s) = signature.split_at(FIELD_SIZE);
    Ok(BigNum::from_slice(s)? <= BigNum::from_slice(&SECP256R1_HALF_ORDER)?)
}

/// The only precompile layout the verifier accepts: one signature, with key,
/// signature and signing input in this instruction's own data.
pub fn secp256r1_instruction(
    public_key: &[u8; COMPRESSED_PUBLIC_KEY_LEN],
    signature: &[u8; SIGNATURE_LEN],
    signing_input: &[u8],
) -> AdapterResult<Instruction> {
    if signing_input.is_empty() || signing_input.len() > MAX_SIGNING_INPUT_LEN {
        return Err(AdapterError::InvalidSigningInputLength);
    }
    let signing_input_len =
        u16::try_from(signing_input.len()).map_err(|_| AdapterError::InvalidSigningInputLength)?;
    let offsets: [u16; 7] = [
        SIGNATURE_OFFSET as u16,
        CURRENT_INSTRUCTION,
        PUBLIC_KEY_OFFSET as u16,
        CURRENT_INSTRUCTION,
        MESSAGE_OFFSET as u16,
        signing_input_len,
        CURRENT_INSTRUCTION,
    ];
    let mut data = Vec::with_capacity(MESSAGE_OFFSET.saturating_add(signing_input.len()));
    data.extend_from_slice(&[1, 0]);
    for field in offsets {
        data.extend_from_slice(&field.to_le_bytes());
    }
    data.extend_from_slice(public_key);
    data.extend_from_slice(signature);
    data.extend_from_slice(signing_input);
    Ok(Instruction {
        program_id: solana_sdk_ids::secp256r1_program::ID,
        accounts: Vec::new(),
        data,
    })
}

/// A throwaway P-256 key generated in memory, for tests and demonstrations.
/// It is never written to disk.
pub struct SyntheticSigner {
    key: EcKey<Private>,
}

impl std::fmt::Debug for SyntheticSigner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SyntheticSigner")
    }
}

impl SyntheticSigner {
    pub fn generate() -> AdapterResult<Self> {
        let group = p256_group()?;
        Ok(Self {
            key: EcKey::generate(&group)?,
        })
    }

    pub fn public_key(&self) -> AdapterResult<[u8; COMPRESSED_PUBLIC_KEY_LEN]> {
        compressed(&p256_group()?, self.key.public_key())
    }

    /// Raw ES256 `r || s` over the signing input. OpenSSL does not normalize
    /// `s`, so roughly half of these need [`normalize_signature`].
    pub fn sign(&self, signing_input: &[u8]) -> AdapterResult<[u8; SIGNATURE_LEN]> {
        let digest = openssl::sha::sha256(signing_input);
        let signature = EcdsaSig::sign(&digest, &self.key)?;
        raw_signature(signature.r(), signature.s())
    }

    /// A compact JWS signed by this key, with a low-S signature.
    pub fn compact_jws(&self, header_json: &[u8], payload_json: &[u8]) -> AdapterResult<String> {
        let input = signing_input(header_json, payload_json);
        let signature = normalize_signature(&self.sign(&input)?)?;
        let input = String::from_utf8(input).map_err(|_| AdapterError::InvalidCompactJws)?;
        Ok(format!("{input}.{}", base64url_encode(&signature)))
    }
}

/// The high-S twin `(r, n - s)` of a signature, which ECDSA also accepts.
pub fn high_s_twin(signature: &[u8; SIGNATURE_LEN]) -> AdapterResult<[u8; SIGNATURE_LEN]> {
    let (r, s) = signature.split_at(FIELD_SIZE);
    let r = BigNum::from_slice(r)?;
    let s = BigNum::from_slice(s)?;
    let order = BigNum::from_slice(&SECP256R1_ORDER)?;
    let mut twin = BigNum::new()?;
    twin.checked_sub(&order, &s)?;
    raw_signature(&r, &twin)
}

fn p256_group() -> AdapterResult<EcGroup> {
    Ok(EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?)
}

fn compressed(
    group: &EcGroup,
    point: &openssl::ec::EcPointRef,
) -> AdapterResult<[u8; COMPRESSED_PUBLIC_KEY_LEN]> {
    let mut context = BigNumContext::new()?;
    let bytes = point.to_bytes(group, PointConversionForm::COMPRESSED, &mut context)?;
    <[u8; COMPRESSED_PUBLIC_KEY_LEN]>::try_from(bytes).map_err(|_| AdapterError::PointNotOnCurve)
}

fn raw_signature(
    r: &openssl::bn::BigNumRef,
    s: &openssl::bn::BigNumRef,
) -> AdapterResult<[u8; SIGNATURE_LEN]> {
    let field = i32::try_from(FIELD_SIZE).map_err(|_| AdapterError::Crypto)?;
    let mut signature = r.to_vec_padded(field)?;
    signature.extend_from_slice(&s.to_vec_padded(field)?);
    <[u8; SIGNATURE_LEN]>::try_from(signature).map_err(|_| AdapterError::ScalarOutOfRange)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn instruction_matches_the_sdk_builder() {
        let signer = SyntheticSigner::generate().unwrap();
        let input = signing_input(br#"{"alg":"ES256"}"#, br#"{"a":1}"#);
        let signature = normalize_signature(&signer.sign(&input).unwrap()).unwrap();
        let public_key = signer.public_key().unwrap();

        let ours = secp256r1_instruction(&public_key, &signature, &input).unwrap();
        let sdk = solana_secp256r1_program::new_secp256r1_instruction_with_signature(
            &input,
            &signature,
            &public_key,
        );
        assert_eq!(ours.program_id, sdk.program_id);
        assert_eq!(ours.data, sdk.data);
        assert!(ours.accounts.is_empty());
    }

    #[test]
    fn normalization_yields_low_s_and_is_idempotent() {
        let signer = SyntheticSigner::generate().unwrap();
        let signature = signer.sign(b"a.b").unwrap();
        let low = normalize_signature(&signature).unwrap();
        let high = high_s_twin(&low).unwrap();

        assert!(is_low_s(&low).unwrap());
        assert!(!is_low_s(&high).unwrap());
        assert_eq!(normalize_signature(&high).unwrap(), low);
        assert_eq!(normalize_signature(&low).unwrap(), low);
    }

    #[test]
    fn normalization_rejects_out_of_range_scalars() {
        let mut zero_r = [0x11; SIGNATURE_LEN];
        zero_r[..FIELD_SIZE].fill(0);
        let mut order_s = [0x11; SIGNATURE_LEN];
        order_s[FIELD_SIZE..].copy_from_slice(&SECP256R1_ORDER);
        for signature in [zero_r, order_s, [0xFF; SIGNATURE_LEN]] {
            assert_eq!(
                normalize_signature(&signature),
                Err(AdapterError::ScalarOutOfRange)
            );
        }
    }

    #[test]
    fn compresses_the_published_saifuro_key() {
        let compressed = compress_jwk_public_key(
            "0sSLihPTUCksZoUJR3Eh22UcZhhOu8-nEqSa_98rE_I",
            "eXSPgBZzeT1E6HtkD8tlg8yujO37jKQhH3SDd99Xx68",
        )
        .unwrap();
        assert_eq!(compressed[0], 0x03);
        assert!(
            compress_jwk_public_key(
                "0sSLihPTUCksZoUJR3Eh22UcZhhOu8-nEqSa_98rE_I",
                "eXSPgBZzeT1E6HtkD8tlg8yujO37jKQhH3SDd99Xx6g",
            )
            .is_err()
        );
    }
}
