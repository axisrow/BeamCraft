// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Fixed-capacity signed integers for exact geometric predicates.
//!
//! Every finite binary64 value is an integer multiple of `2^-1074`; with the F06
//! coordinate cap `|x| <= 4e9 < 2^32` it is an integer below `2^1106` after that
//! scaling. Differences, degree-three determinants and the 8192-term volume sums
//! therefore fit in 72 limbs (4608 bits). There is no heap use, recursion or
//! unsafe code; an operation that would exceed the capacity fails closed.

use crate::enclosure::{Enclosure, scale_pow2};

/// Number of 64-bit limbs; 4608 bits cover every sum the kernel forms.
pub(crate) const LIMBS: usize = 72;

/// A computation exceeded the fixed capacity (never reachable under the caps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overflow;

/// Sign-and-magnitude integer; `mag[len..]` is always zero and `len` is minimal.
#[derive(Clone)]
pub struct Big {
    neg: bool,
    len: usize,
    mag: [u64; LIMBS],
}

impl Big {
    /// The integer zero.
    pub const ZERO: Self = Self {
        neg: false,
        len: 0,
        mag: [0; LIMBS],
    };

    /// `m * 2^shift` for a signed 64-bit mantissa.
    pub(crate) fn from_i64_shifted(m: i64, shift: u32) -> Result<Self, Overflow> {
        if m == 0 {
            return Ok(Self::ZERO);
        }
        let limb = usize::try_from(shift / 64).map_err(|_| Overflow)?;
        let bit = shift % 64;
        let wide = u128::from(m.unsigned_abs()) << bit;
        let mut out = Self::ZERO;
        out.neg = m < 0;
        *out.mag.get_mut(limb).ok_or(Overflow)? = wide as u64;
        let high = (wide >> 64) as u64;
        if high != 0 {
            *out.mag.get_mut(limb + 1).ok_or(Overflow)? = high;
        }
        out.normalize();
        Ok(out)
    }

    fn normalize(&mut self) {
        self.len = LIMBS;
        while self.len > 0 && self.mag.get(self.len - 1).copied().unwrap_or(0) == 0 {
            self.len -= 1;
        }
        if self.len == 0 {
            self.neg = false;
        }
    }

    /// Returns -1, 0 or 1.
    pub const fn sign(&self) -> i8 {
        if self.len == 0 {
            0
        } else if self.neg {
            -1
        } else {
            1
        }
    }

    /// Negated copy.
    pub fn negated(&self) -> Self {
        let mut out = self.clone();
        if out.len != 0 {
            out.neg = !out.neg;
        }
        out
    }

    fn limbs(&self) -> &[u64] {
        self.mag.get(..self.len).unwrap_or(&[])
    }

    /// Shared with the steel crate.
    pub fn cmp_mag(&self, other: &Self) -> core::cmp::Ordering {
        self.len.cmp(&other.len).then_with(|| {
            self.limbs()
                .iter()
                .rev()
                .zip(other.limbs().iter().rev())
                .map(|(a, b)| a.cmp(b))
                .find(|o| o.is_ne())
                .unwrap_or(core::cmp::Ordering::Equal)
        })
    }

    fn add_mag(&self, other: &Self, neg: bool) -> Result<Self, Overflow> {
        let mut out = Self::ZERO;
        let mut carry = false;
        let longest = self.len.max(other.len);
        for i in 0..longest {
            let a = self.mag.get(i).copied().unwrap_or(0);
            let b = other.mag.get(i).copied().unwrap_or(0);
            let (s1, c1) = a.overflowing_add(b);
            let (s2, c2) = s1.overflowing_add(u64::from(carry));
            *out.mag.get_mut(i).ok_or(Overflow)? = s2;
            carry = c1 || c2;
        }
        if carry {
            *out.mag.get_mut(longest).ok_or(Overflow)? = 1;
        }
        out.neg = neg;
        out.normalize();
        Ok(out)
    }

    /// `self` magnitude minus `other` magnitude; requires `|self| >= |other|`.
    fn sub_mag(&self, other: &Self, neg: bool) -> Self {
        let mut out = Self::ZERO;
        let mut borrow = false;
        for i in 0..self.len {
            let a = self.mag.get(i).copied().unwrap_or(0);
            let b = other.mag.get(i).copied().unwrap_or(0);
            let (d1, b1) = a.overflowing_sub(b);
            let (d2, b2) = d1.overflowing_sub(u64::from(borrow));
            if let Some(slot) = out.mag.get_mut(i) {
                *slot = d2;
            }
            borrow = b1 || b2;
        }
        out.neg = neg;
        out.normalize();
        out
    }

    /// Exact sum.
    pub fn add(&self, other: &Self) -> Result<Self, Overflow> {
        if self.len == 0 {
            return Ok(other.clone());
        }
        if other.len == 0 {
            return Ok(self.clone());
        }
        if self.neg == other.neg {
            return self.add_mag(other, self.neg);
        }
        Ok(match self.cmp_mag(other) {
            core::cmp::Ordering::Equal => Self::ZERO,
            core::cmp::Ordering::Greater => self.sub_mag(other, self.neg),
            core::cmp::Ordering::Less => other.sub_mag(self, other.neg),
        })
    }

    /// Exact difference.
    pub fn sub(&self, other: &Self) -> Result<Self, Overflow> {
        self.add(&other.negated())
    }

    /// Exact product by schoolbook multiplication.
    pub fn mul(&self, other: &Self) -> Result<Self, Overflow> {
        if self.len == 0 || other.len == 0 {
            return Ok(Self::ZERO);
        }
        let mut out = Self::ZERO;
        for (i, &a) in self.limbs().iter().enumerate() {
            let mut carry: u128 = 0;
            for (j, &b) in other.limbs().iter().enumerate() {
                let slot = out.mag.get_mut(i + j).ok_or(Overflow)?;
                let t = u128::from(a) * u128::from(b) + u128::from(*slot) + carry;
                *slot = t as u64;
                carry = t >> 64;
            }
            let mut k = i + other.len;
            while carry != 0 {
                let slot = out.mag.get_mut(k).ok_or(Overflow)?;
                let t = u128::from(*slot) + carry;
                *slot = t as u64;
                carry = t >> 64;
                k += 1;
            }
        }
        out.neg = self.neg != other.neg;
        out.normalize();
        Ok(out)
    }

    /// Top 64 significant bits, the discarded shift and whether lower bits were lost.
    fn top_bits(&self) -> (u64, u32, bool) {
        let Some(&high) = self.limbs().last() else {
            return (0, 0, false);
        };
        let bits = 64 * (self.len - 1) + (64 - high.leading_zeros() as usize);
        if bits <= 64 {
            return (high, 0, false);
        }
        let shift = bits - 64;
        let (li, bo) = (shift / 64, shift % 64);
        let lo = self.mag.get(li).copied().unwrap_or(0);
        let mut top = lo >> bo;
        if bo > 0 {
            top |= self.mag.get(li + 1).copied().unwrap_or(0) << (64 - bo);
        }
        let low_mask = if bo == 0 { 0 } else { (1u64 << bo) - 1 };
        let sticky = lo & low_mask != 0
            || self
                .mag
                .get(..li)
                .is_some_and(|below| below.iter().any(|l| *l != 0));
        (top, u32::try_from(shift).unwrap_or(u32::MAX), sticky)
    }

    /// Like [`Self::enclose`], but a value too small for the normal range gets a
    /// valid outward bound `[0, 2^top]` (or its negative) instead of `None`, where
    /// `2^top` (at least `MIN_POSITIVE`) exceeds `|value|`. Normal values always
    /// take the exact enclosure. Overflow is still `None`.
    pub fn enclose_clamped(&self, pow2: i32) -> Option<Enclosure> {
        if let Some(e) = self.enclose(pow2) {
            return Some(e);
        }
        let bits = i32::try_from(64 * self.len.checked_sub(1)?).ok()? + 64
            - i32::try_from(self.limbs().last()?.leading_zeros()).ok()?;
        // |value| < 2^top.
        let top = bits.checked_add(pow2)?;
        if top > 1023 {
            return None;
        }
        let bound = scale_pow2(1.0, top.max(-1022))?;
        Some(if self.neg {
            Enclosure {
                lo: -bound,
                hi: 0.0,
            }
        } else {
            Enclosure { lo: 0.0, hi: bound }
        })
    }

    /// Encloses `self * 2^pow2` in binary64, or `None` when the result is
    /// not a normal finite value (overflow or loss to underflow).
    pub fn enclose(&self, pow2: i32) -> Option<Enclosure> {
        if self.len == 0 {
            return Some(Enclosure::point(0.0));
        }
        let (top, shift, sticky) = self.top_bits();
        let approx = top as f64;
        let exact = !sticky && approx as u128 == u128::from(top);
        let (lo, hi) = if exact {
            (approx, approx)
        } else {
            (approx.next_down(), approx.next_up())
        };
        let total = i32::try_from(shift).ok()?.checked_add(pow2)?;
        let (lo, hi) = (scale_pow2(lo, total)?, scale_pow2(hi, total)?);
        Some(if self.neg {
            Enclosure { lo: -hi, hi: -lo }
        } else {
            Enclosure { lo, hi }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(v: i128) -> Big {
        Big::from_i64_shifted(i64::try_from(v).unwrap(), 0).unwrap()
    }

    #[test]
    fn small_arithmetic_matches_i128() {
        let values: [i128; 9] = [
            0,
            1,
            -1,
            7,
            -9,
            1 << 40,
            -(1 << 62),
            123_456_789,
            -987_654_321,
        ];
        for &a in &values {
            for &b in &values {
                let (x, y) = (big(a), big(b));
                assert_eq!(x.add(&y).unwrap().sign(), (a + b).signum() as i8);
                assert_eq!(x.sub(&y).unwrap().sign(), (a - b).signum() as i8);
                assert_eq!(x.mul(&y).unwrap().sign(), (a * b).signum() as i8);
                let product = x.mul(&y).unwrap().enclose(0).unwrap();
                assert!(product.lo <= (a * b) as f64 && (a * b) as f64 <= product.hi);
            }
        }
    }

    #[test]
    fn wide_values_round_trip_through_enclosures() {
        let two_1000 = Big::from_i64_shifted(1, 1000).unwrap();
        let e = two_1000.enclose(-1000).unwrap();
        assert_eq!((e.lo, e.hi), (1.0, 1.0));
        let plus_one = two_1000.add(&Big::from_i64_shifted(1, 0).unwrap()).unwrap();
        let e = plus_one.enclose(-1000).unwrap();
        assert!(e.lo <= 1.0 && e.hi > 1.0);
        let diff = plus_one.sub(&two_1000).unwrap();
        assert_eq!(diff.enclose(0).unwrap().lo, 1.0);
        assert!(Big::from_i64_shifted(1, 1024).unwrap().enclose(0).is_none());
        assert!(Big::from_i64_shifted(1, 4700).is_err());
    }

    #[test]
    fn clamped_enclosures_bound_underflowing_values_outward() {
        let tiny = Big::from_i64_shifted(3, 0).unwrap();
        // 3 * 2^-1100 is below the normal range: no exact enclosure, a clamped one.
        assert!(tiny.enclose(-1100).is_none());
        let clamped = tiny.enclose_clamped(-1100).unwrap();
        assert_eq!((clamped.lo, clamped.hi), (0.0, f64::MIN_POSITIVE));
        let negative = tiny.negated().enclose_clamped(-1100).unwrap();
        assert_eq!((negative.lo, negative.hi), (-f64::MIN_POSITIVE, 0.0));
        assert_eq!(tiny.enclose_clamped(0), tiny.enclose(0));
        assert!(
            Big::from_i64_shifted(1, 1100)
                .unwrap()
                .enclose_clamped(0)
                .is_none()
        );
        assert_eq!(
            Big::ZERO.enclose_clamped(-5000),
            Some(Enclosure::point(0.0))
        );
    }

    #[test]
    fn clamping_never_excludes_a_value_near_the_normal_boundary() {
        // Every power of two from 2^-1030 to 2^-990 must stay inside its enclosure;
        // 2^-1010 is normal and used to be clamped to [0, 2^-1022].
        for k in -1030..=-990i32 {
            let value = 2f64.powi(k);
            let e = Big::from_i64_shifted(1, 0)
                .unwrap()
                .enclose_clamped(k)
                .unwrap();
            assert!(e.lo <= value && value <= e.hi, "2^{k}: {e:?}");
            let three = Big::from_i64_shifted(3, 0)
                .unwrap()
                .enclose_clamped(k)
                .unwrap();
            assert!(
                three.lo <= 3.0 * value && 3.0 * value <= three.hi,
                "3*2^{k}: {three:?}"
            );
            let neg = Big::from_i64_shifted(-3, 0)
                .unwrap()
                .enclose_clamped(k)
                .unwrap();
            assert!(
                neg.lo <= -3.0 * value && -3.0 * value <= neg.hi,
                "-3*2^{k}: {neg:?}"
            );
        }
        // Normal values keep their tight enclosure.
        let e = Big::from_i64_shifted(1, 0)
            .unwrap()
            .enclose_clamped(-1010)
            .unwrap();
        assert_eq!((e.lo, e.hi), (2f64.powi(-1010), 2f64.powi(-1010)));
        assert!(e.lo > f64::MIN_POSITIVE);
        // Only a value proven below the normal range is widened to [0, 2^-1022].
        let below = Big::from_i64_shifted(1, 0)
            .unwrap()
            .enclose_clamped(-1023)
            .unwrap();
        assert_eq!((below.lo, below.hi), (0.0, f64::MIN_POSITIVE));
    }

    #[test]
    fn capacity_overflow_fails_closed() {
        let wide = Big::from_i64_shifted(1, 4000).unwrap();
        assert!(wide.mul(&wide).is_err());
        assert_eq!(wide.sub(&wide).unwrap().sign(), 0);
    }
}
