//! The simulator's random numbers: the same seed gives the same demo.
//!
//! A generator of our own (SplitMix64) instead of a crate's, because the
//! sequence is part of the demo: an upgrade of a dependency must not change
//! who walks where.

/// A deterministic stream of random numbers.
#[derive(Debug, Clone)]
pub struct Rng {
    seed: u64,
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self { seed, state: seed }
    }

    /// A stream of its own for `label`, independent of how much of this one
    /// was used: a bot's walk does not change when its neighbour posts more.
    pub fn fork(&self, label: &str) -> Self {
        // FNV-1a over the label, mixed with the seed.
        let hash = label
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3));
        let mut mixed = Self::new(self.seed ^ hash.rotate_left(17));
        Self::new(mixed.draw())
    }

    pub fn draw(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A whole number below `bound` (0 when `bound` is 0).
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 { 0 } else { (self.draw() % bound as u64) as usize }
    }

    /// A number in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.draw() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A number in `[low, high)`.
    pub fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    pub fn chance(&mut self, probability: f64) -> bool {
        self.unit() < probability
    }

    /// One of `items`; panics on an empty slice (the pools are constants).
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_numbers() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        assert_eq!((0..50).map(|_| a.draw()).collect::<Vec<_>>(), (0..50).map(|_| b.draw()).collect::<Vec<_>>());
        assert_ne!(Rng::new(7).draw(), Rng::new(8).draw());
    }

    #[test]
    fn a_fork_does_not_depend_on_what_was_drawn_before() {
        let mut used = Rng::new(7);
        let before = used.fork("walk").draw();
        used.draw();
        assert_eq!(before, used.fork("walk").draw());
        assert_ne!(before, Rng::new(7).fork("chat").draw());
    }

    #[test]
    fn numbers_stay_in_their_ranges() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            assert!(rng.below(7) < 7);
            let x = rng.between(-2.0, 3.0);
            assert!((-2.0..3.0).contains(&x));
        }
        assert_eq!(rng.below(0), 0);
    }
}
