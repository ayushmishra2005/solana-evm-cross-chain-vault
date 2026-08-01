use crate::error::Rejection;

/// Raw amount of the underlying asset, in the smallest asset unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetAmount(u128);

impl AssetAmount {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(raw: u128) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u128 {
        self.0
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub fn checked_add(self, other: Self) -> Result<Self, Rejection> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(Rejection::ArithmeticOverflow)
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, Rejection> {
        self.0
            .checked_sub(other.0)
            .map(Self)
            .ok_or(Rejection::ArithmeticUnderflow)
    }
}

/// Raw amount of vault shares, in the smallest share unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShareAmount(u128);

impl ShareAmount {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(raw: u128) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u128 {
        self.0
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub fn checked_add(self, other: Self) -> Result<Self, Rejection> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(Rejection::ArithmeticOverflow)
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, Rejection> {
        self.0
            .checked_sub(other.0)
            .map(Self)
            .ok_or(Rejection::ArithmeticUnderflow)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EpochId(u64);

impl EpochId {
    pub const GENESIS: Self = Self(0);

    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Result<Self, Rejection> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(Rejection::ArithmeticOverflow)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountId(u64);

impl AccountId {
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(u64);

impl Timestamp {
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    pub fn checked_add_seconds(self, seconds: u64) -> Result<Self, Rejection> {
        self.0
            .checked_add(seconds)
            .map(Self)
            .ok_or(Rejection::ArithmeticOverflow)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigVersion(u32);

impl ConfigVersion {
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}
