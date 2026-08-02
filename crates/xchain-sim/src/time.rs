//! Simulated time.
//!
//! The simulator never reads a system clock. Every deadline and every delivery
//! is expressed in ticks that only move when a caller asks them to.

/// A point on the simulated timeline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(u64);

impl Tick {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the later tick, or `None` when the sum would wrap.
    #[must_use]
    pub fn checked_add(self, ticks: u64) -> Option<Self> {
        self.0.checked_add(ticks).map(Self)
    }
}

impl From<u64> for Tick {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<Tick> for u64 {
    fn from(value: Tick) -> Self {
        value.0
    }
}

impl core::fmt::Display for Tick {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "t{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_compare_in_timeline_order() {
        assert!(Tick::new(1) < Tick::new(2));
        assert_eq!(Tick::ZERO, Tick::new(0));
    }

    #[test]
    fn adding_past_the_limit_returns_nothing() {
        assert_eq!(Tick::new(u64::MAX).checked_add(1), None);
        assert_eq!(Tick::new(1).checked_add(2), Some(Tick::new(3)));
    }

    #[test]
    fn a_tick_converts_both_ways() {
        assert_eq!(u64::from(Tick::from(9u64)), 9);
    }

    #[test]
    fn a_tick_prints_its_number() {
        extern crate alloc;
        use alloc::string::ToString;
        assert_eq!(Tick::new(12).to_string(), "t12");
    }
}
