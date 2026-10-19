// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// Rust guideline compliant 2026-05-18

//! A tiny deterministic xorshift64* pseudo-random number generator.
//!
//! It is not cryptographically secure; it only drives visual effects.

/// Seed used in place of 0, which is a fixed point of xorshift.
const ZERO_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// Output multiplier of xorshift64* (Vigna, 2014).
const MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

/// Deterministic xorshift64* generator.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Creates a generator from `seed`; a seed of 0 maps to a fixed nonzero value.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let state = if seed == 0 { ZERO_SEED } else { seed };
        Self { state }
    }

    /// Returns the next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(MULTIPLIER)
    }

    /// Returns a value in `0..n`, or 0 when `n == 0`.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        // Lemire's multiply-shift reduction on the high 32 bits.
        let scaled = ((self.next_u64() >> 32) * u64::from(n)) >> 32;
        u32::try_from(scaled).unwrap_or(0)
    }

    /// Returns a value in `[0, 1)` with 16 bits of resolution.
    pub(crate) fn unit(&mut self) -> f32 {
        let bits = u16::try_from(self.below(1 << 16)).unwrap_or(0);
        f32::from(bits) / 65_536.0
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn zero_seed_is_usable() {
        let mut rng = Rng::new(0);
        assert_ne!(rng.next_u64(), 0);
        assert_ne!(rng.next_u64(), rng.next_u64());
    }

    #[test]
    fn below_stays_in_range() {
        let mut rng = Rng::new(42);
        assert_eq!(rng.below(0), 0);
        for n in [1, 2, 3, 7, 100, u32::MAX] {
            for _ in 0..1000 {
                assert!(rng.below(n) < n);
            }
        }
    }

    #[test]
    fn unit_in_range_and_deterministic() {
        let mut a = Rng::new(5);
        let mut b = Rng::new(5);
        for _ in 0..1000 {
            let u = a.unit();
            assert!((0.0..1.0).contains(&u));
            assert_eq!(u.to_bits(), b.unit().to_bits());
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }
}
