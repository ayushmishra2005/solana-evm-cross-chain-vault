//! The two independent delivery paths.

/// Control messages and asset value move on separate lanes.
///
/// Nothing in the simulator ties one to the other. A lane can be paused on its
/// own, and a message can arrive with no matching transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lane {
    Control,
    Asset,
}

impl Lane {
    pub const ALL: [Self; 2] = [Self::Control, Self::Asset];

    /// Sort weight used to break ties between lanes at the same tick.
    ///
    /// Control sorts first so an ordering is always defined.
    #[must_use]
    pub const fn priority(self) -> u8 {
        match self {
            Self::Control => 0,
            Self::Asset => 1,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Asset => "asset",
        }
    }
}

impl core::fmt::Display for Lane {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Whether a lane currently lets deliveries complete.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LaneState {
    #[default]
    Running,
    Paused,
}

impl LaneState {
    #[must_use]
    pub const fn is_paused(self) -> bool {
        matches!(self, Self::Paused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_sorts_before_asset() {
        assert!(Lane::Control.priority() < Lane::Asset.priority());
        assert!(Lane::Control < Lane::Asset);
    }

    #[test]
    fn each_lane_has_its_own_name() {
        assert_eq!(Lane::Control.name(), "control");
        assert_eq!(Lane::Asset.name(), "asset");
    }

    #[test]
    fn a_lane_starts_running() {
        assert!(!LaneState::default().is_paused());
        assert!(LaneState::Paused.is_paused());
    }
}
