use core::ops::RangeInclusive;

use crate::amount::{AccountId, AssetAmount, EpochId, ShareAmount};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestKey {
    pub epoch: EpochId,
    pub account: AccountId,
}

impl RequestKey {
    #[must_use]
    pub const fn new(epoch: EpochId, account: AccountId) -> Self {
        Self { epoch, account }
    }

    /// Every key that can belong to one epoch, in map order.
    pub(crate) fn epoch_range(epoch: EpochId) -> RangeInclusive<Self> {
        Self::new(epoch, AccountId::new(u64::MIN))..=Self::new(epoch, AccountId::new(u64::MAX))
    }
}

/// Observable position of a request in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestState {
    Pending,
    Locked,
    Claimable,
    Claimed,
    Refundable,
    Refunded,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DepositRequest {
    pub assets: AssetAmount,
    pub cancelled_assets: AssetAmount,
    pub cancelled: bool,
    pub claimed: bool,
}

impl DepositRequest {
    /// A request still owed a settlement or refund outcome.
    #[must_use]
    pub const fn is_outstanding(&self) -> bool {
        !self.cancelled && !self.claimed && !self.assets.is_zero()
    }

    /// A request that carries an amount into its epoch aggregate.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !self.cancelled && !self.assets.is_zero()
    }

    /// Cancelling empties the amount, so the two must always agree.
    #[must_use]
    pub const fn flags_agree(&self) -> bool {
        self.cancelled == self.assets.is_zero() && !(self.cancelled && self.claimed)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RedeemRequest {
    pub shares: ShareAmount,
    pub cancelled_shares: ShareAmount,
    pub cancelled: bool,
    pub claimed: bool,
}

impl RedeemRequest {
    #[must_use]
    pub const fn is_outstanding(&self) -> bool {
        !self.cancelled && !self.claimed && !self.shares.is_zero()
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        !self.cancelled && !self.shares.is_zero()
    }

    #[must_use]
    pub const fn flags_agree(&self) -> bool {
        self.cancelled == self.shares.is_zero() && !(self.cancelled && self.claimed)
    }
}
