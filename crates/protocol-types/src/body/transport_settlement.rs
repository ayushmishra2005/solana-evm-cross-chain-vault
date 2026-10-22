use crate::codec::{
    read_array, read_u8, read_u64, read_u128, write_bytes, write_u8, write_u64, write_u128,
};
use crate::error::{AmountField, DecodeError, EncodeError, IdentifierField, ValidationError};
use crate::identifier::{AssetAmount, ConfigVersion, TransferId};
use crate::layout;

/// Which terminal transport outcome one settlement reports.
///
/// The discriminant is on the wire, so it must never be reordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SettlementKind {
    /// An allocation ended after its deadline. Only what arrived is credited.
    Closed,
    /// Unattributed custody was credited against outstanding impairment.
    Recovered,
}

impl SettlementKind {
    /// Every kind, in discriminant order.
    pub const ALL: [Self; 2] = [Self::Closed, Self::Recovered];

    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Closed => 1,
            Self::Recovered => 2,
        }
    }

    /// Rejects zero and every kind this build does not know.
    pub const fn from_u8(value: u8) -> Result<Self, DecodeError> {
        match value {
            1 => Ok(Self::Closed),
            2 => Ok(Self::Recovered),
            other => Err(DecodeError::UnknownSettlementKind(other)),
        }
    }
}

/// The one terminal notice for a closed allocation or for a recovery.
///
/// For a closed allocation the claim is its authorized amount. For a recovery
/// it is the pooled impairment outstanding before the recovery. Impairment is
/// pooled because custody is fungible, so a settlement never says which
/// earlier transfer an asset belonged to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportSettlementBody {
    pub transfer_id: TransferId,
    pub kind: SettlementKind,
    pub claim_amount: AssetAmount,
    pub credited_amount: AssetAmount,
    /// Pooled impairment the leg still carries after this settlement.
    pub impaired_outstanding: AssetAmount,
    pub config_version: ConfigVersion,
}

impl TransportSettlementBody {
    /// The part of the claim this settlement did not credit.
    #[must_use]
    pub const fn shortfall(&self) -> AssetAmount {
        AssetAmount::new(
            self.claim_amount
                .get()
                .saturating_sub(self.credited_amount.get()),
        )
    }

    pub(crate) fn encode_into(&self, out: &mut [u8]) -> Result<(), EncodeError> {
        write_bytes(
            out,
            layout::SETTLEMENT_TRANSFER_ID_OFFSET,
            self.transfer_id.as_bytes(),
        )?;
        write_u8(out, layout::SETTLEMENT_KIND_OFFSET, self.kind.to_u8())?;
        write_u128(
            out,
            layout::SETTLEMENT_CLAIM_AMOUNT_OFFSET,
            self.claim_amount.get(),
        )?;
        write_u128(
            out,
            layout::SETTLEMENT_CREDITED_AMOUNT_OFFSET,
            self.credited_amount.get(),
        )?;
        write_u128(
            out,
            layout::SETTLEMENT_IMPAIRED_OUTSTANDING_OFFSET,
            self.impaired_outstanding.get(),
        )?;
        write_u64(
            out,
            layout::SETTLEMENT_CONFIG_VERSION_OFFSET,
            self.config_version.get(),
        )
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            transfer_id: TransferId::new(read_array(bytes, layout::SETTLEMENT_TRANSFER_ID_OFFSET)?),
            kind: SettlementKind::from_u8(read_u8(bytes, layout::SETTLEMENT_KIND_OFFSET)?)?,
            claim_amount: AssetAmount::new(read_u128(
                bytes,
                layout::SETTLEMENT_CLAIM_AMOUNT_OFFSET,
            )?),
            credited_amount: AssetAmount::new(read_u128(
                bytes,
                layout::SETTLEMENT_CREDITED_AMOUNT_OFFSET,
            )?),
            impaired_outstanding: AssetAmount::new(read_u128(
                bytes,
                layout::SETTLEMENT_IMPAIRED_OUTSTANDING_OFFSET,
            )?),
            config_version: ConfigVersion::new(read_u64(
                bytes,
                layout::SETTLEMENT_CONFIG_VERSION_OFFSET,
            )?),
        })
    }

    /// A closed allocation adds its shortfall to the pool. A recovery leaves
    /// exactly the uncredited part of the pool behind.
    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        if self.transfer_id.is_zero() {
            return Err(ValidationError::ZeroIdentifier(IdentifierField::Transfer));
        }
        if self.claim_amount.is_zero() {
            return Err(ValidationError::ZeroAmount(AmountField::ClaimAmount));
        }
        if self.credited_amount > self.claim_amount {
            return Err(ValidationError::CreditAboveClaim);
        }
        match self.kind {
            SettlementKind::Closed => {
                if self.impaired_outstanding < self.shortfall() {
                    return Err(ValidationError::ImpairmentBelowShortfall);
                }
            }
            SettlementKind::Recovered => {
                if self.impaired_outstanding != self.shortfall() {
                    return Err(ValidationError::RecoveryImpairmentMismatch);
                }
            }
        }
        if self.config_version.is_zero() {
            return Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigVersion,
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn sample() -> Self {
        Self {
            transfer_id: TransferId::new([0x66; 32]),
            kind: SettlementKind::Closed,
            claim_amount: AssetAmount::new(1_000_000),
            credited_amount: AssetAmount::new(600_000),
            impaired_outstanding: AssetAmount::new(400_000),
            config_version: ConfigVersion::new(4),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(body: &TransportSettlementBody) -> [u8; layout::TRANSPORT_SETTLEMENT_BODY_LEN] {
        let mut bytes = [0u8; layout::TRANSPORT_SETTLEMENT_BODY_LEN];
        assert_eq!(body.encode_into(&mut bytes), Ok(()));
        bytes
    }

    fn recovered(claim: u128, credited: u128, outstanding: u128) -> TransportSettlementBody {
        TransportSettlementBody {
            kind: SettlementKind::Recovered,
            claim_amount: AssetAmount::new(claim),
            credited_amount: AssetAmount::new(credited),
            impaired_outstanding: AssetAmount::new(outstanding),
            ..TransportSettlementBody::sample()
        }
    }

    #[test]
    fn a_sample_body_survives_the_round_trip() {
        let body = TransportSettlementBody::sample();
        assert_eq!(TransportSettlementBody::decode(&encoded(&body)), Ok(body));
    }

    #[test]
    fn the_kind_sits_at_its_declared_offset() {
        let bytes = encoded(&TransportSettlementBody::sample());
        assert_eq!(bytes.get(layout::SETTLEMENT_KIND_OFFSET), Some(&1));
        let bytes = encoded(&recovered(10, 4, 6));
        assert_eq!(bytes.get(layout::SETTLEMENT_KIND_OFFSET), Some(&2));
    }

    #[test]
    fn every_kind_keeps_its_wire_number() {
        for (index, kind) in SettlementKind::ALL.iter().enumerate() {
            assert_eq!(usize::from(kind.to_u8()), index.saturating_add(1));
            assert_eq!(SettlementKind::from_u8(kind.to_u8()), Ok(*kind));
        }
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        for value in [0u8, 3, 255] {
            assert_eq!(
                SettlementKind::from_u8(value),
                Err(DecodeError::UnknownSettlementKind(value))
            );
        }
    }

    #[test]
    fn a_sample_body_is_valid() {
        assert_eq!(TransportSettlementBody::sample().validate(), Ok(()));
        assert_eq!(recovered(10, 4, 6).validate(), Ok(()));
    }

    #[test]
    fn a_zero_transfer_id_is_refused() {
        let body = TransportSettlementBody {
            transfer_id: TransferId::new([0; 32]),
            ..TransportSettlementBody::sample()
        };
        assert_eq!(
            body.validate(),
            Err(ValidationError::ZeroIdentifier(IdentifierField::Transfer))
        );
    }

    #[test]
    fn a_zero_claim_is_refused() {
        let body = TransportSettlementBody {
            claim_amount: AssetAmount::new(0),
            credited_amount: AssetAmount::new(0),
            impaired_outstanding: AssetAmount::new(0),
            ..TransportSettlementBody::sample()
        };
        assert_eq!(
            body.validate(),
            Err(ValidationError::ZeroAmount(AmountField::ClaimAmount))
        );
    }

    #[test]
    fn a_credit_above_the_claim_is_refused() {
        let body = TransportSettlementBody {
            credited_amount: AssetAmount::new(1_000_001),
            ..TransportSettlementBody::sample()
        };
        assert_eq!(body.validate(), Err(ValidationError::CreditAboveClaim));
    }

    #[test]
    fn a_closed_allocation_cannot_carry_less_impairment_than_its_shortfall() {
        let body = TransportSettlementBody {
            impaired_outstanding: AssetAmount::new(399_999),
            ..TransportSettlementBody::sample()
        };
        assert_eq!(
            body.validate(),
            Err(ValidationError::ImpairmentBelowShortfall)
        );
    }

    #[test]
    fn a_closed_allocation_may_join_earlier_impairment() {
        let body = TransportSettlementBody {
            impaired_outstanding: AssetAmount::new(900_000),
            ..TransportSettlementBody::sample()
        };
        assert_eq!(body.validate(), Ok(()));
    }

    #[test]
    fn a_full_close_carries_no_shortfall() {
        let body = TransportSettlementBody {
            credited_amount: AssetAmount::new(1_000_000),
            impaired_outstanding: AssetAmount::new(0),
            ..TransportSettlementBody::sample()
        };
        assert_eq!(body.shortfall(), AssetAmount::new(0));
        assert_eq!(body.validate(), Ok(()));
    }

    #[test]
    fn a_recovery_leaves_exactly_the_uncredited_impairment() {
        assert_eq!(
            recovered(10, 4, 7).validate(),
            Err(ValidationError::RecoveryImpairmentMismatch)
        );
        assert_eq!(
            recovered(10, 4, 5).validate(),
            Err(ValidationError::RecoveryImpairmentMismatch)
        );
        assert_eq!(recovered(10, 0, 10).validate(), Ok(()));
        assert_eq!(recovered(10, 10, 0).validate(), Ok(()));
    }

    #[test]
    fn a_zero_config_version_is_refused() {
        let body = TransportSettlementBody {
            config_version: ConfigVersion::new(0),
            ..TransportSettlementBody::sample()
        };
        assert_eq!(
            body.validate(),
            Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigVersion
            ))
        );
    }

    #[test]
    fn decoding_reads_every_field_from_its_offset() {
        let mut bytes = encoded(&TransportSettlementBody::sample());
        if let Some(kind) = bytes.get_mut(layout::SETTLEMENT_KIND_OFFSET) {
            *kind = 2;
        }
        let decoded = TransportSettlementBody::decode(&bytes);
        assert_eq!(decoded.map(|body| body.kind), Ok(SettlementKind::Recovered));
    }
}
