// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Exact planar sign predicates for binary64 coordinates.
//!
//! The decision is the sign of the exact rational determinant of the submitted
//! binary64 values (F06 section 2, F04 P-1). A rounding-error filter decides
//! the clear cases; every other case runs a fixed-capacity sign-magnitude
//! integer evaluation. No epsilon is ever used for a sign, and
//! the integer type is fixed-size: its capacity is sized from the 2^32 magnitude
//! bound enforced on inputs, so an overflow is reported as uncertain, never wrapped.

use super::Point2;
use core::cmp::Ordering;

/// An exact sign could not be decided (non-finite or out-of-range coordinate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Uncertain;

const LIMBS: usize = 80;
const OPERAND_LIMBS: usize = 40;
const U: f64 = 1.0 / 9_007_199_254_740_992.0;
const FILTER_FLOOR: f64 = 1e-290;
const MAGNITUDE_BOUND: f64 = 4_294_967_296.0;

/// Sign-magnitude integer in units of 2^-1074; zero is never negative.
#[derive(Clone, Copy)]
struct Int {
    neg: bool,
    mag: [u32; LIMBS],
}

impl Int {
    const ZERO: Self = Self {
        neg: false,
        mag: [0; LIMBS],
    };

    fn is_zero(&self) -> bool {
        self.mag.iter().all(|l| *l == 0)
    }

    fn from_f64(value: f64) -> Option<Self> {
        if !value.is_finite() || value.abs() >= MAGNITUDE_BOUND {
            return None;
        }
        let bits = value.to_bits();
        let neg = bits >> 63 != 0;
        let exponent = (bits >> 52) & 0x7ff;
        let fraction = bits & ((1u64 << 52) - 1);
        let (mantissa, shift) = if exponent == 0 {
            (fraction, 0u64)
        } else {
            (fraction | (1u64 << 52), exponent - 1)
        };
        let mut out = Self::ZERO;
        let first = usize::try_from(shift / 32).ok()?;
        let wide = u128::from(mantissa) << (shift % 32);
        for (k, part) in [wide, wide >> 32, wide >> 64].into_iter().enumerate() {
            *out.mag.get_mut(first.checked_add(k)?)? = u32::try_from(part & 0xffff_ffff).ok()?;
        }
        out.neg = neg && !out.is_zero();
        Some(out)
    }

    fn cmp_mag(a: &[u32; LIMBS], b: &[u32; LIMBS]) -> Ordering {
        for (x, y) in a.iter().zip(b.iter()).rev() {
            match x.cmp(y) {
                Ordering::Equal => {}
                other => return other,
            }
        }
        Ordering::Equal
    }

    fn add_mag(a: &[u32; LIMBS], b: &[u32; LIMBS]) -> Option<[u32; LIMBS]> {
        let mut out = [0u32; LIMBS];
        let mut carry = 0u64;
        for ((o, x), y) in out.iter_mut().zip(a.iter()).zip(b.iter()) {
            let sum = u64::from(*x) + u64::from(*y) + carry;
            *o = u32::try_from(sum & 0xffff_ffff).ok()?;
            carry = sum >> 32;
        }
        if carry == 0 { Some(out) } else { None }
    }

    /// Requires `a >= b` in magnitude.
    fn sub_mag(a: &[u32; LIMBS], b: &[u32; LIMBS]) -> Option<[u32; LIMBS]> {
        let mut out = [0u32; LIMBS];
        let mut borrow = 0i64;
        for ((o, x), y) in out.iter_mut().zip(a.iter()).zip(b.iter()) {
            let mut diff = i64::from(*x) - i64::from(*y) - borrow;
            if diff < 0 {
                diff += 1 << 32;
                borrow = 1;
            } else {
                borrow = 0;
            }
            *o = u32::try_from(diff).ok()?;
        }
        if borrow == 0 { Some(out) } else { None }
    }

    fn negated(mut self) -> Self {
        if !self.is_zero() {
            self.neg = !self.neg;
        }
        self
    }

    fn add(&self, other: &Self) -> Option<Self> {
        if self.neg == other.neg {
            let mag = Self::add_mag(&self.mag, &other.mag)?;
            return Some(Self { neg: self.neg, mag });
        }
        let (neg, mag) = match Self::cmp_mag(&self.mag, &other.mag) {
            Ordering::Equal => return Some(Self::ZERO),
            Ordering::Greater => (self.neg, Self::sub_mag(&self.mag, &other.mag)?),
            Ordering::Less => (other.neg, Self::sub_mag(&other.mag, &self.mag)?),
        };
        Some(Self { neg, mag })
    }

    fn sub(&self, other: &Self) -> Option<Self> {
        self.add(&other.negated())
    }

    fn mul(&self, other: &Self) -> Option<Self> {
        let wide = |x: &Self| x.mag.iter().skip(OPERAND_LIMBS).any(|l| *l != 0);
        if wide(self) || wide(other) {
            return None;
        }
        let mut out = [0u32; LIMBS];
        for (i, a) in self.mag.iter().take(OPERAND_LIMBS).enumerate() {
            if *a == 0 {
                continue;
            }
            let mut carry = 0u64;
            for (j, b) in other.mag.iter().take(OPERAND_LIMBS).enumerate() {
                let slot = out.get_mut(i.checked_add(j)?)?;
                let cur = u64::from(*slot) + u64::from(*a) * u64::from(*b) + carry;
                *slot = u32::try_from(cur & 0xffff_ffff).ok()?;
                carry = cur >> 32;
            }
            // Rows below `i` have not reached this limb yet, so it is still zero.
            *out.get_mut(i.checked_add(OPERAND_LIMBS)?)? = u32::try_from(carry).ok()?;
        }
        let mut product = Self {
            neg: false,
            mag: out,
        };
        product.neg = (self.neg != other.neg) && !product.is_zero();
        Some(product)
    }

    fn sign(&self) -> Ordering {
        if self.is_zero() {
            Ordering::Equal
        } else if self.neg {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }
}

fn cross_exact(ax: &Int, ay: &Int, bx: &Int, by: &Int, cx: &Int, cy: &Int) -> Option<Int> {
    let left = bx.sub(ax)?.mul(&cy.sub(ay)?)?;
    let right = by.sub(ay)?.mul(&cx.sub(ax)?)?;
    left.sub(&right)
}

/// Exact sign of `(b - a) x (c - a)`: positive when `c` is left of `a -> b`.
pub(crate) fn orient2d(a: Point2, b: Point2, c: Point2) -> Result<Ordering, Uncertain> {
    // Filter: each operand difference, product and the final subtraction
    // contributes one relative rounding error u, so |error| <= 3.01u(|l|+|r|) <= 4u(|l|+|r|).
    let left = (b.x - a.x) * (c.y - a.y);
    let right = (b.y - a.y) * (c.x - a.x);
    let det = left - right;
    let magnitude = left.abs() + right.abs();
    if det.is_finite() && magnitude > FILTER_FLOOR && det.abs() > 4.0 * U * magnitude {
        return Ok(if det > 0.0 {
            Ordering::Greater
        } else {
            Ordering::Less
        });
    }
    orient2d_exact(a, b, c)
}

pub(crate) fn orient2d_exact(a: Point2, b: Point2, c: Point2) -> Result<Ordering, Uncertain> {
    let convert = |v: f64| Int::from_f64(v).ok_or(Uncertain);
    let det = cross_exact(
        &convert(a.x)?,
        &convert(a.y)?,
        &convert(b.x)?,
        &convert(b.y)?,
        &convert(c.x)?,
        &convert(c.y)?,
    )
    .ok_or(Uncertain)?;
    Ok(det.sign())
}

/// Exact sign of the shoelace area of a closed polygon (positive: counter-clockwise).
pub(crate) fn signed_area_sign(points: &[Point2]) -> Result<Ordering, Uncertain> {
    let Some(first) = points.first() else {
        return Ok(Ordering::Equal);
    };
    let convert = |v: f64| Int::from_f64(v).ok_or(Uncertain);
    let (ox, oy) = (convert(first.x)?, convert(first.y)?);
    let mut sum = Int::ZERO;
    for (p, q) in points.iter().skip(1).zip(points.iter().skip(2)) {
        let term = cross_exact(
            &ox,
            &oy,
            &convert(p.x)?,
            &convert(p.y)?,
            &convert(q.x)?,
            &convert(q.y)?,
        )
        .ok_or(Uncertain)?;
        sum = sum.add(&term).ok_or(Uncertain)?;
    }
    Ok(sum.sign())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point2 {
        Point2 { x, y }
    }

    // Scaled integer coordinates k * 2^-30 evaluate exactly in i128.
    fn reference(a: (i64, i64), b: (i64, i64), c: (i64, i64)) -> Ordering {
        let det = i128::from(b.0 - a.0) * i128::from(c.1 - a.1)
            - i128::from(b.1 - a.1) * i128::from(c.0 - a.0);
        det.cmp(&0)
    }

    #[test]
    fn exact_matches_integer_reference_on_a_seeded_grid() {
        let scale = 1.0 / 1_073_741_824.0;
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            i64::try_from(state % 2001).unwrap() - 1000
        };
        for _ in 0..2000 {
            let (a, b, c) = ((next(), next()), (next(), next()), (next(), next()));
            let pt = |q: (i64, i64)| p(q.0 as f64 * scale, q.1 as f64 * scale);
            let want = reference(a, b, c);
            assert_eq!(orient2d_exact(pt(a), pt(b), pt(c)), Ok(want));
            assert_eq!(orient2d(pt(a), pt(b), pt(c)), Ok(want));
        }
    }

    #[test]
    fn near_collinear_points_are_decided_exactly() {
        // c is one ulp off the line y = x, below the filter's error bound.
        let a = p(0.0, 0.0);
        let b = p(1.0, 1.0);
        let hi = 1e9f64;
        let up = f64::from_bits(hi.to_bits() + 1);
        assert_eq!(orient2d(a, b, p(0.5, 0.5)), Ok(Ordering::Equal));
        assert_eq!(orient2d(a, b, p(hi, hi)), Ok(Ordering::Equal));
        assert_eq!(orient2d(a, b, p(hi, up)), Ok(Ordering::Greater));
        assert_eq!(orient2d(a, b, p(up, hi)), Ok(Ordering::Less));
        // Subnormal coordinates exercise the exact fallback below the filter floor.
        let tiny = f64::from_bits(1);
        assert_eq!(
            orient2d(p(0.0, 0.0), p(tiny, 0.0), p(0.0, tiny)),
            Ok(Ordering::Greater)
        );
        assert_eq!(
            orient2d(p(0.0, 0.0), p(tiny, 0.0), p(0.0, -tiny)),
            Ok(Ordering::Less)
        );
    }

    #[test]
    fn out_of_range_and_non_finite_are_uncertain() {
        for bad in [f64::NAN, f64::INFINITY, 5e9] {
            assert_eq!(
                orient2d_exact(p(0.0, 0.0), p(1.0, 0.0), p(bad, 1.0)),
                Err(Uncertain)
            );
        }
        assert_eq!(
            orient2d(p(0.0, 0.0), p(1.0, 0.0), p(f64::NAN, 1.0)),
            Err(Uncertain)
        );
    }

    #[test]
    fn area_sign_follows_winding_and_collapses_to_zero() {
        let square = [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)];
        assert_eq!(signed_area_sign(&square), Ok(Ordering::Greater));
        let reversed = [square[3], square[2], square[1], square[0]];
        assert_eq!(signed_area_sign(&reversed), Ok(Ordering::Less));
        let flat = [p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)];
        assert_eq!(signed_area_sign(&flat), Ok(Ordering::Equal));
        assert_eq!(signed_area_sign(&[]), Ok(Ordering::Equal));
    }
}
