/// SplitMix64. Chosen because the algorithm is short enough to audit and gives
/// the same stream on every platform.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const MIX_A: u64 = 0xBF58_476D_1CE4_E5B9;
const MIX_B: u64 = 0x94D0_49BB_1331_11EB;

impl Rng {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GAMMA);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(MIX_A);
        z = (z ^ (z >> 27)).wrapping_mul(MIX_B);
        z ^ (z >> 31)
    }

    /// Uniform value in `0..bound`. Returns zero when the bound is zero.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        self.next_u64() % bound
    }

    /// Uniform value in `low..=high`. Swaps the ends when they arrive reversed.
    pub fn between(&mut self, low: u64, high: u64) -> u64 {
        let (low, high) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
        low.wrapping_add(self.below(high.wrapping_sub(low).wrapping_add(1)))
    }

    pub fn between_u128(&mut self, low: u128, high: u128) -> u128 {
        let (low, high) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
        let span = high.wrapping_sub(low).wrapping_add(1);
        if span == 0 {
            return low;
        }
        let draw = u128::from(self.next_u64()) << 64 | u128::from(self.next_u64());
        low.wrapping_add(draw % span)
    }

    /// True with probability `numerator / denominator`.
    pub fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        if denominator == 0 {
            return false;
        }
        self.below(denominator) < numerator
    }

    pub fn pick<T: Copy>(&mut self, options: &[T]) -> Option<T> {
        if options.is_empty() {
            return None;
        }
        let index = usize::try_from(self.below(options.len() as u64)).unwrap_or(0);
        options.get(index).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_repeats_the_same_stream() {
        let mut first = Rng::new(42);
        let mut second = Rng::new(42);
        for _ in 0..256 {
            assert_eq!(first.next_u64(), second.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut first = Rng::new(1);
        let mut second = Rng::new(2);
        let left: Vec<u64> = (0..64).map(|_| first.next_u64()).collect();
        let right: Vec<u64> = (0..64).map(|_| second.next_u64()).collect();
        assert_ne!(left, right);
    }

    #[test]
    fn bounded_draws_stay_inside_their_range() {
        let mut rng = Rng::new(7);
        for _ in 0..1024 {
            let value = rng.between(10, 20);
            assert!((10..=20).contains(&value));
            assert!(rng.below(5) < 5);
        }
    }

    #[test]
    fn a_single_value_range_returns_that_value() {
        let mut rng = Rng::new(9);
        assert_eq!(rng.between(3, 3), 3);
        assert_eq!(rng.between_u128(11, 11), 11);
    }
}
