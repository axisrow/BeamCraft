// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Outward-rounded binary64 enclosures for derived mesh measurements.
//!
//! Rust exposes only round-to-nearest, so each inexact operation widens its
//! result by one representable step in both directions, which always brackets the
//! exact value because binary64 operations are correctly rounded (G04 section 3.3).
//! An operation known to be exact keeps a point result, so integer-valued
//! quantities stay tight. Any non-finite or underflowing result is `None`.

/// Magnitudes below this are never trusted to be exact: underflow may hide a residual.
const EXACT_FLOOR: f64 = 1e-250;

/// A closed interval `[lo, hi]` known to contain a computed quantity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Enclosure {
    /// Lower bound, rounded downward.
    pub lo: f64,
    /// Upper bound, rounded upward.
    pub hi: f64,
}

impl Enclosure {
    /// A degenerate enclosure of a single exactly known value.
    pub const fn point(value: f64) -> Self {
        Self {
            lo: value,
            hi: value,
        }
    }

    /// Validates finite endpoints with `lo <= hi`.
    pub fn new(lo: f64, hi: f64) -> Option<Self> {
        (lo.is_finite() && hi.is_finite() && lo <= hi).then_some(Self { lo, hi })
    }

    /// Returns true when `value` lies inside the closed interval.
    pub fn contains(self, value: f64) -> bool {
        self.lo <= value && value <= self.hi
    }

    /// Returns true when the two closed intervals share a point.
    pub fn overlaps(self, other: Self) -> bool {
        self.lo <= other.hi && other.lo <= self.hi
    }

    /// Returns `hi - lo` rounded upward; infinite only for non-finite input.
    pub fn width(self) -> f64 {
        (self.hi - self.lo).next_up()
    }

    /// Returns the centre, used only for reporting.
    pub fn midpoint(self) -> f64 {
        self.lo * 0.5 + self.hi * 0.5
    }

    /// Shared with the steel crate.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, other: Self) -> Option<Self> {
        finite_pair(
            add_pair(self.lo, other.lo)?.0,
            add_pair(self.hi, other.hi)?.1,
        )
    }

    pub(crate) fn mul(self, other: Self) -> Option<Self> {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for a in [self.lo, self.hi] {
            for b in [other.lo, other.hi] {
                let (down, up) = mul_pair(a, b)?;
                lo = lo.min(down);
                hi = hi.max(up);
            }
        }
        let nonnegative = self.lo >= 0.0 && other.lo >= 0.0;
        // A product of nonnegative intervals is nonnegative even when underflow widened it.
        finite_pair(if nonnegative { lo.max(0.0) } else { lo }, hi)
    }

    /// Quotient by an interval that excludes zero.
    pub(crate) fn div(self, other: Self) -> Option<Self> {
        if other.lo <= 0.0 && other.hi >= 0.0 {
            return None;
        }
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for a in [self.lo, self.hi] {
            for b in [other.lo, other.hi] {
                let (down, up) = div_pair(a, b)?;
                lo = lo.min(down);
                hi = hi.max(up);
            }
        }
        finite_pair(lo, hi)
    }

    /// Square root of a nonnegative interval.
    pub(crate) fn sqrt(self) -> Option<Self> {
        if self.lo < 0.0 {
            return None;
        }
        finite_pair(sqrt_pair(self.lo)?.0, sqrt_pair(self.hi)?.1)
    }

    /// Multiplication by an exact power of two, never underflowing silently.
    pub(crate) fn scaled_pow2(self, k: i32) -> Option<Self> {
        finite_pair(scale_pow2(self.lo, k)?, scale_pow2(self.hi, k)?)
    }
}

fn finite_pair(lo: f64, hi: f64) -> Option<Enclosure> {
    Enclosure::new(lo, hi)
}

/// Multiplies by `2^k` exactly, or returns `None` on overflow or inexact underflow.
pub(crate) fn scale_pow2(x: f64, k: i32) -> Option<f64> {
    if x == 0.0 {
        return Some(0.0);
    }
    let mut r = x;
    let mut left = k;
    while left != 0 {
        let step = left.clamp(-1000, 1000);
        r *= f64::from_bits(u64::try_from(1023 + step).ok()? << 52);
        left -= step;
        if !r.is_finite() {
            return None;
        }
    }
    (r.abs() >= f64::MIN_POSITIVE).then_some(r)
}

fn add_pair(a: f64, b: f64) -> Option<(f64, f64)> {
    let s = a + b;
    if !s.is_finite() {
        return None;
    }
    // Knuth TwoSum: the error term is zero exactly when the sum is exact.
    let bb = s - a;
    let err = (a - (s - bb)) + (b - bb);
    Some(if err == 0.0 {
        (s, s)
    } else {
        (s.next_down(), s.next_up())
    })
}

fn mul_pair(a: f64, b: f64) -> Option<(f64, f64)> {
    let p = a * b;
    if !p.is_finite() {
        return None;
    }
    let trusted = a == 0.0 || b == 0.0 || p.abs() >= EXACT_FLOOR;
    Some(if trusted && a.mul_add(b, -p) == 0.0 {
        (p, p)
    } else {
        (p.next_down(), p.next_up())
    })
}

fn div_pair(a: f64, b: f64) -> Option<(f64, f64)> {
    let q = a / b;
    if !q.is_finite() {
        return None;
    }
    // The fused residual of an inexact quotient can itself underflow to zero, so a
    // zero residual proves exactness only when numerator and quotient are well above
    // the underflow range (the residual is then at least about |a| * 2^-104).
    let trusted = a == 0.0 || (a.abs() >= EXACT_FLOOR && q.abs() >= EXACT_FLOOR);
    Some(if trusted && q.mul_add(b, -a) == 0.0 {
        (q, q)
    } else {
        (q.next_down(), q.next_up())
    })
}

fn sqrt_pair(a: f64) -> Option<(f64, f64)> {
    let s = a.sqrt();
    if !s.is_finite() {
        return None;
    }
    let trusted = a == 0.0 || a >= EXACT_FLOOR;
    Some(if trusted && s.mul_add(s, -a) == 0.0 {
        (s, s)
    } else {
        (s.next_down().max(0.0), s.next_up())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_operations_stay_points() {
        let a = Enclosure::point(6.0);
        let b = Enclosure::point(3.0);
        assert_eq!(a.div(b), Some(Enclosure::point(2.0)));
        assert_eq!(a.mul(b), Some(Enclosure::point(18.0)));
        assert_eq!(a.add(b), Some(Enclosure::point(9.0)));
        assert_eq!(Enclosure::point(49.0).sqrt(), Some(Enclosure::point(7.0)));
    }

    #[test]
    fn inexact_operations_bracket_the_exact_value() {
        let third = Enclosure::point(1.0).div(Enclosure::point(3.0)).unwrap();
        assert!(third.lo < third.hi);
        // 3 * 1/3 must bracket 1 even though the rounded product is exactly 1.
        let back = third.mul(Enclosure::point(3.0)).unwrap();
        assert!(back.contains(1.0));
        let root2 = Enclosure::point(2.0).sqrt().unwrap();
        assert!(root2.lo * root2.lo <= 2.0 && 2.0 <= root2.hi * root2.hi);
    }

    #[test]
    fn a_zero_residual_near_underflow_is_not_proof_of_an_exact_quotient() {
        // 1/3 of MIN_POSITIVE: the fused residual underflows to zero, yet q is inexact.
        let a = f64::MIN_POSITIVE;
        let q = Enclosure::point(a).div(Enclosure::point(3.0 * a)).unwrap();
        assert!(q.lo < q.hi, "{q:?}");
        // The exact 1/3 lies between the neighbours of the rounded quotient.
        let rounded = a / (3.0 * a);
        assert!(q.lo < rounded && rounded < q.hi);
        // Tiny numerators are widened too, even when the rounded quotient is large.
        let tiny = Enclosure::point(1e-300)
            .div(Enclosure::point(3.0e-300))
            .unwrap();
        assert!(tiny.lo < tiny.hi);
        // A large exactly divisible case is still a point.
        assert_eq!(
            Enclosure::point(6.0).div(Enclosure::point(3.0)),
            Some(Enclosure::point(2.0))
        );
    }

    #[test]
    fn invalid_requests_are_rejected() {
        // Squares of nonnegative intervals never dip below zero, even through underflow.
        let tiny = Enclosure::new(0.0, f64::MIN_POSITIVE).unwrap();
        let square = tiny.mul(tiny).unwrap();
        assert!(square.lo >= 0.0 && square.hi > 0.0);
        assert!(square.sqrt().is_some());
        let zero = Enclosure::new(-1.0, 1.0).unwrap();
        assert!(Enclosure::point(1.0).div(zero).is_none());
        assert!(zero.sqrt().is_none());
        assert!(Enclosure::new(2.0, 1.0).is_none());
        assert!(Enclosure::new(f64::NAN, 1.0).is_none());
        assert!(scale_pow2(1.0, 2000).is_none());
        assert!(scale_pow2(1.0, -1100).is_none());
        assert_eq!(scale_pow2(3.0, 10), Some(3072.0));
    }
}
