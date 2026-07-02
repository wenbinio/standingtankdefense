//! Deterministic primitives for the Standing Tank Defense simulation core.
//!
//! INVARIANTS (see `CLAUDE.md` + `docs/05-data-model.md` §5.6):
//! - ZERO external dependencies. Bit-identical across platforms/compilers.
//! - No floats, no wall-clock, no `std` randomness, no hashing of pointers.
//! - All arithmetic is checked/wrapping *explicitly*; never relies on release
//!   wrapping (the workspace sets `overflow-checks = true` even in release).
//!
//! Provides:
//! - [`Fixed`]: Q47.16 fixed-point number (i64 raw, 16 fractional bits).
//! - [`Rng`]: SplitMix64 deterministic PRNG with purpose-stream derivation.
//! - [`Checksum`]: FNV-1a 64-bit accumulator for `state_checksum`.

// ============================ Fixed-point ============================

/// Q47.16 signed fixed-point. Range ~[-1.4e14, 1.4e14], precision 1/65536.
/// Use for positions, rates, and multipliers. HP/gold/damage stay `i64`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Default)]
pub struct Fixed(pub i64);

impl Fixed {
    pub const FRAC_BITS: u32 = 16;
    pub const ONE: Fixed = Fixed(1 << Self::FRAC_BITS);
    pub const ZERO: Fixed = Fixed(0);

    #[inline]
    pub const fn from_int(i: i64) -> Fixed {
        Fixed(i << Self::FRAC_BITS)
    }
    #[inline]
    pub const fn from_raw(raw: i64) -> Fixed {
        Fixed(raw)
    }
    #[inline]
    pub const fn raw(self) -> i64 {
        self.0
    }
    /// Fixed `num/den` (deterministic, truncated toward negative infinity).
    #[inline]
    pub fn from_ratio(num: i64, den: i64) -> Fixed {
        Fixed((((num as i128) << Self::FRAC_BITS) / den as i128) as i64)
    }
    /// Floor toward negative infinity (arithmetic shift). Deterministic.
    #[inline]
    pub const fn floor_to_int(self) -> i64 {
        self.0 >> Self::FRAC_BITS
    }
    // `mul`/`div` intentionally shadow the `std::ops` names: they are the sim's
    // ONLY sanctioned Fixed×Fixed operators (saturating, Q-format-aware), and the
    // explicit method calls keep every checksum-path multiply/divide greppable.
    // Renaming or moving them behind `impl Mul/Div` would churn every hot-path
    // call site for zero behavior change.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn mul(self, o: Fixed) -> Fixed {
        // SATURATING at the i64 boundary. The i128 product never overflows; only the
        // final narrowing can. Clamping (vs wrapping/panicking) keeps the result
        // monotonic and DETERMINISTIC — a pure function of the integer inputs, no
        // floats, identical on every platform. This only ever engages on extreme
        // builds whose multiplier already exceeds the representable range (e.g. a bot
        // stacking hundreds of multiplicative damage mods); for all in-range values
        // it is bit-identical to the plain narrowing, so no normal run / checksum
        // changes — it just turns an out-of-range panic into a saturated ceiling.
        Fixed(sat_i128_to_i64(
            (self.0 as i128 * o.0 as i128) >> Self::FRAC_BITS,
        ))
    }
    // See `mul` for why this shadows `std::ops::Div::div` on purpose.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn div(self, o: Fixed) -> Fixed {
        Fixed(sat_i128_to_i64(
            ((self.0 as i128) << Self::FRAC_BITS) / o.0 as i128,
        ))
    }
    /// Multiply an `i64` magnitude (e.g. damage) by this multiplier, flooring.
    /// Saturating at the i64 boundary (see [`Fixed::mul`]).
    #[inline]
    pub fn scale_i64(self, v: i64) -> i64 {
        sat_i128_to_i64((v as i128 * self.0 as i128) >> Self::FRAC_BITS)
    }
    /// Deterministic integer square root of a non-negative Fixed.
    /// Returns floor(sqrt(self)) as a Fixed. Panics on negative input.
    pub fn sqrt(self) -> Fixed {
        assert!(self.0 >= 0, "Fixed::sqrt of negative");
        // sqrt(x) in Q.f: sqrt(raw << f) since result needs the frac shift back.
        // value = raw / 2^f ; sqrt(value) = sqrt(raw) / 2^(f/2).
        // Compute isqrt(raw << FRAC_BITS) to get result in raw units.
        let n: u128 = (self.0 as u128) << Self::FRAC_BITS;
        Fixed(isqrt_u128(n) as i64)
    }
}

// All Fixed arithmetic SATURATES at the i64 boundary instead of wrapping/panicking.
// This is a pure-integer, platform-stable, deterministic clamp: for every in-range
// value it is bit-identical to plain arithmetic (so no normal run or checksum is
// affected), and it only engages on extreme builds whose accumulated value already
// exceeds the representable range — turning a would-be overflow panic into a stable
// saturated ceiling. (Needed because the bot can legitimately stack many additive /
// multiplicative damage modifiers; the sim must not crash on such builds.)
impl core::ops::Add for Fixed {
    type Output = Fixed;
    #[inline]
    fn add(self, o: Fixed) -> Fixed {
        Fixed(self.0.saturating_add(o.0))
    }
}
impl core::ops::Sub for Fixed {
    type Output = Fixed;
    #[inline]
    fn sub(self, o: Fixed) -> Fixed {
        Fixed(self.0.saturating_sub(o.0))
    }
}
impl core::ops::Neg for Fixed {
    type Output = Fixed;
    #[inline]
    fn neg(self) -> Fixed {
        Fixed(self.0.saturating_neg())
    }
}
impl core::ops::AddAssign for Fixed {
    #[inline]
    fn add_assign(&mut self, o: Fixed) {
        self.0 = self.0.saturating_add(o.0);
    }
}
impl core::ops::SubAssign for Fixed {
    #[inline]
    fn sub_assign(&mut self, o: Fixed) {
        self.0 = self.0.saturating_sub(o.0);
    }
}

/// Clamp an i128 to the i64 range (saturating narrowing) — the shared helper for
/// all Fixed multiply/divide/scale paths.
#[inline]
fn sat_i128_to_i64(v: i128) -> i64 {
    if v > i64::MAX as i128 {
        i64::MAX
    } else if v < i64::MIN as i128 {
        i64::MIN
    } else {
        v as i64
    }
}

/// Floor integer square root of a u128.
fn isqrt_u128(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    // Newton's method with an integer initial guess.
    // `(bits + 1) / 2` == `bits.div_ceil(2)` for all non-negative values — a
    // bit-identical rewrite (checksum-path safe), just clearer.
    let mut x = 1u128 << (128 - n.leading_zeros()).div_ceil(2);
    loop {
        let y = (x + n / x) >> 1;
        if y >= x {
            return x;
        }
        x = y;
    }
}

// ============================ PRNG ============================

/// SplitMix64 — a small, fast, fully deterministic PRNG. State is a single u64,
/// so it serializes trivially into snapshots (`docs/05 §5.6`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rng {
    state: u64,
}

impl Rng {
    #[inline]
    pub const fn from_seed(seed: u64) -> Rng {
        Rng { state: seed }
    }
    #[inline]
    pub const fn state(self) -> u64 {
        self.state
    }

    /// Derive an independent stream from a master seed + identifying coords.
    /// `stream = splitmix( hash(master, player, purpose, round) )`.
    pub fn derive(master: u64, player: u32, purpose: u32, round: u32) -> Rng {
        let mut s = master;
        for v in [player as u64, purpose as u64, round as u64] {
            s ^= v.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut r = Rng { state: s };
            s = r.next_u64();
        }
        Rng { state: s }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Unbiased integer in `[0, n)` (Lemire's method). `below(0) == 0`.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let mut x = self.next_u32() as u64;
        let mut m = x * n as u64;
        let mut l = m as u32;
        if l < n {
            let t = n.wrapping_neg() % n;
            while l < t {
                x = self.next_u32() as u64;
                m = x * n as u64;
                l = m as u32;
            }
        }
        (m >> 32) as u32
    }
    /// True with probability `num/den`.
    #[inline]
    pub fn chance(&mut self, num: u32, den: u32) -> bool {
        self.below(den) < num
    }
}

// ============================ Checksum ============================

/// FNV-1a 64-bit accumulator. Feed every authoritative field each tick-batch,
/// in a *fixed order*, to produce `state_checksum` (`docs/04 §4.4.4`).
#[derive(Clone, Copy, Debug)]
pub struct Checksum(u64);

impl Default for Checksum {
    fn default() -> Self {
        Self::new()
    }
}

impl Checksum {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    #[inline]
    pub const fn new() -> Checksum {
        Checksum(Self::OFFSET)
    }
    #[inline]
    pub fn write_u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }
    #[inline]
    pub fn write_i64(&mut self, v: i64) {
        self.write_u64(v as u64);
    }
    #[inline]
    pub fn write_u32(&mut self, v: u32) {
        self.write_u64(v as u64);
    }
    #[inline]
    pub fn write_fixed(&mut self, v: Fixed) {
        self.write_i64(v.0);
    }
    #[inline]
    pub fn finish(self) -> u64 {
        self.0
    }
}

// ============================ Tests ============================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_roundtrip_and_ops() {
        assert_eq!(Fixed::from_int(5).floor_to_int(), 5);
        assert_eq!((Fixed::from_int(3) + Fixed::from_int(4)).floor_to_int(), 7);
        assert_eq!(
            Fixed::from_int(6)
                .mul(Fixed::from_ratio(1, 2))
                .floor_to_int(),
            3
        );
        assert_eq!(
            Fixed::from_int(10).div(Fixed::from_int(4)),
            Fixed::from_ratio(10, 4)
        );
        assert_eq!(Fixed::ONE.scale_i64(1000), 1000);
        assert_eq!(Fixed::from_ratio(3, 2).scale_i64(1000), 1500);
    }

    #[test]
    fn fixed_sqrt_is_floor_exact_for_squares() {
        for k in [0i64, 1, 2, 9, 16, 100, 144, 1000, 65536] {
            let s = Fixed::from_int(k).sqrt();
            // floor(sqrt(k)) check
            let expect = (k as f64).sqrt().floor() as i64;
            assert_eq!(s.floor_to_int(), expect, "sqrt({k})");
        }
    }

    #[test]
    fn rng_is_reproducible() {
        let mut a = Rng::from_seed(12345);
        let mut b = Rng::from_seed(12345);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn rng_streams_are_independent() {
        let m = 0xDEAD_BEEF_CAFE_F00D;
        let mut spawn = Rng::derive(m, 3, 0, 7);
        let mut shop = Rng::derive(m, 3, 2, 7);
        // Different purposes must not produce identical sequences.
        let s1: Vec<u64> = (0..8).map(|_| spawn.next_u64()).collect();
        let s2: Vec<u64> = (0..8).map(|_| shop.next_u64()).collect();
        assert_ne!(s1, s2);
    }

    #[test]
    fn below_is_in_range() {
        let mut r = Rng::from_seed(99);
        for _ in 0..10_000 {
            assert!(r.below(6) < 6);
        }
        assert_eq!(r.below(0), 0);
    }

    #[test]
    fn checksum_order_sensitive() {
        let mut a = Checksum::new();
        a.write_u64(1);
        a.write_u64(2);
        let mut b = Checksum::new();
        b.write_u64(2);
        b.write_u64(1);
        assert_ne!(a.finish(), b.finish());
    }
}
