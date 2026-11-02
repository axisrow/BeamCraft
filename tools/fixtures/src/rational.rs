// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Exact rational arithmetic for the G11 references, with no third-party dependency.
//!
//! A [`Rational`] is a reduced fraction of arbitrary-size integers. Every binary64 value is a
//! dyadic rational, so [`Rational::from_f64`] is exact; sums, products and quotients stay exact.
//! Nothing here rounds except [`Rational::to_scientific`], which prints a decimal for readers.
//! The arithmetic is deliberately simple (schoolbook products, binary long division, binary GCD):
//! the references are a few hundred numbers of at most a few thousand bits.

use core::cmp::Ordering;

/// An unsigned integer of any size: little-endian 32-bit limbs, no high zero limbs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Natural(Vec<u32>);

impl Natural {
    fn zero() -> Self {
        Self(Vec::new())
    }

    fn from_u64(v: u64) -> Self {
        let mut n = Self(vec![v as u32, (v >> 32) as u32]);
        n.trim();
        n
    }

    fn trim(&mut self) {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }

    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    fn bits(&self) -> u64 {
        self.0.last().map_or(0, |top| {
            self.0.len() as u64 * 32 - u64::from(top.leading_zeros())
        })
    }

    fn bit(&self, i: u64) -> bool {
        let limb = usize::try_from(i / 32).ok();
        limb.and_then(|l| self.0.get(l))
            .is_some_and(|w| (w >> (i % 32)) & 1 == 1)
    }

    fn cmp(&self, other: &Self) -> Ordering {
        self.0.len().cmp(&other.0.len()).then_with(|| {
            for (a, b) in self.0.iter().rev().zip(other.0.iter().rev()) {
                match a.cmp(b) {
                    Ordering::Equal => {}
                    unequal => return unequal,
                }
            }
            Ordering::Equal
        })
    }

    fn add(&self, other: &Self) -> Self {
        let mut out = Vec::with_capacity(self.0.len().max(other.0.len()) + 1);
        let mut carry = 0u64;
        for i in 0..self.0.len().max(other.0.len()) {
            let a = u64::from(self.0.get(i).copied().unwrap_or(0));
            let b = u64::from(other.0.get(i).copied().unwrap_or(0));
            let s = a + b + carry;
            out.push(s as u32);
            carry = s >> 32;
        }
        out.push(carry as u32);
        let mut n = Self(out);
        n.trim();
        n
    }

    /// `self - other`; the caller guarantees `self >= other`.
    fn sub(&self, other: &Self) -> Self {
        let mut out = Vec::with_capacity(self.0.len());
        let mut borrow = 0i64;
        for (i, a) in self.0.iter().enumerate() {
            let b = i64::from(other.0.get(i).copied().unwrap_or(0));
            let mut d = i64::from(*a) - b - borrow;
            borrow = 0;
            if d < 0 {
                d += 1 << 32;
                borrow = 1;
            }
            out.push(d as u32);
        }
        let mut n = Self(out);
        n.trim();
        n
    }

    fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        let mut out = vec![0u32; self.0.len() + other.0.len()];
        for (i, a) in self.0.iter().enumerate() {
            let mut carry = 0u64;
            for (j, b) in other.0.iter().enumerate() {
                if let Some(slot) = out.get_mut(i + j) {
                    let t = u64::from(*slot) + u64::from(*a) * u64::from(*b) + carry;
                    *slot = t as u32;
                    carry = t >> 32;
                }
            }
            let mut k = i + other.0.len();
            while carry != 0 {
                if let Some(slot) = out.get_mut(k) {
                    let t = u64::from(*slot) + carry;
                    *slot = t as u32;
                    carry = t >> 32;
                    k += 1;
                } else {
                    break;
                }
            }
        }
        let mut n = Self(out);
        n.trim();
        n
    }

    fn shl(&self, by: u64) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        let limbs = (by / 32) as usize;
        let bits = (by % 32) as u32;
        let mut out = vec![0u32; limbs];
        let mut carry = 0u32;
        for w in &self.0 {
            if bits == 0 {
                out.push(*w);
            } else {
                out.push((w << bits) | carry);
                carry = w >> (32 - bits);
            }
        }
        out.push(carry);
        let mut n = Self(out);
        n.trim();
        n
    }

    fn shr(&self, by: u64) -> Self {
        let limbs = usize::try_from(by / 32).unwrap_or(usize::MAX);
        let bits = (by % 32) as u32;
        let rest: Vec<u32> = self.0.iter().skip(limbs).copied().collect();
        let mut out = Vec::with_capacity(rest.len());
        for (i, w) in rest.iter().enumerate() {
            let high = rest.get(i + 1).copied().unwrap_or(0);
            out.push(if bits == 0 {
                *w
            } else {
                (w >> bits) | (high << (32 - bits))
            });
        }
        let mut n = Self(out);
        n.trim();
        n
    }

    fn trailing_zeros(&self) -> u64 {
        let mut count = 0u64;
        for w in &self.0 {
            if *w == 0 {
                count += 32;
            } else {
                return count + u64::from(w.trailing_zeros());
            }
        }
        count
    }

    /// Quotient and remainder by binary long division; `None` for a zero divisor.
    fn div_rem(&self, divisor: &Self) -> Option<(Self, Self)> {
        if divisor.is_zero() {
            return None;
        }
        if self.cmp(divisor) == Ordering::Less {
            return Some((Self::zero(), self.clone()));
        }
        let mut quotient = vec![0u32; self.0.len()];
        let mut remainder = Self::zero();
        for i in (0..self.bits()).rev() {
            remainder = remainder.shl(1);
            if self.bit(i) {
                remainder = remainder.add(&Self::from_u64(1));
            }
            if remainder.cmp(divisor) != Ordering::Less {
                remainder = remainder.sub(divisor);
                if let Some(w) = usize::try_from(i / 32)
                    .ok()
                    .and_then(|l| quotient.get_mut(l))
                {
                    *w |= 1 << (i % 32);
                }
            }
        }
        let mut q = Self(quotient);
        q.trim();
        Some((q, remainder))
    }

    /// Greatest common divisor by the binary algorithm.
    fn gcd(&self, other: &Self) -> Self {
        if self.is_zero() {
            return other.clone();
        }
        if other.is_zero() {
            return self.clone();
        }
        let (za, zb) = (self.trailing_zeros(), other.trailing_zeros());
        let shift = za.min(zb);
        let mut a = self.shr(za);
        let mut b = other.shr(zb);
        loop {
            match a.cmp(&b) {
                Ordering::Equal => return a.shl(shift),
                Ordering::Greater => {
                    a = a.sub(&b);
                    let z = a.trailing_zeros();
                    a = a.shr(z);
                }
                Ordering::Less => {
                    b = b.sub(&a);
                    let z = b.trailing_zeros();
                    b = b.shr(z);
                }
            }
        }
    }

    /// `floor(sqrt(self))` by Newton's iteration from above.
    fn isqrt(&self) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        let mut x = Self::from_u64(1).shl(self.bits() / 2 + 1);
        loop {
            let Some((q, _)) = self.div_rem(&x) else {
                return x;
            };
            let y = x.add(&q).shr(1);
            if y.cmp(&x) != Ordering::Less {
                return x;
            }
            x = y;
        }
    }

    fn to_decimal(&self) -> String {
        if self.is_zero() {
            return "0".into();
        }
        let billion = Self::from_u64(1_000_000_000);
        let mut chunks = Vec::new();
        let mut n = self.clone();
        while !n.is_zero() {
            let Some((q, r)) = n.div_rem(&billion) else {
                break;
            };
            chunks.push(r.0.first().copied().unwrap_or(0));
            n = q;
        }
        let mut out = String::new();
        for (i, c) in chunks.iter().rev().enumerate() {
            if i == 0 {
                out.push_str(&c.to_string());
            } else {
                out.push_str(&format!("{c:09}"));
            }
        }
        out
    }

    fn parse_decimal(digits: &str) -> Option<Self> {
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let ten = Self::from_u64(10);
        let mut n = Self::zero();
        for b in digits.bytes() {
            n = n.mul(&ten).add(&Self::from_u64(u64::from(b - b'0')));
        }
        Some(n)
    }

    fn pow10(exp: u32) -> Self {
        let ten = Self::from_u64(10);
        (0..exp).fold(Self::from_u64(1), |acc, _| acc.mul(&ten))
    }
}

/// An exact rational number in lowest terms with a positive denominator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rational {
    negative: bool,
    numerator: Natural,
    denominator: Natural,
}

impl Rational {
    fn reduced(negative: bool, numerator: Natural, denominator: Natural) -> Option<Self> {
        if denominator.is_zero() {
            return None;
        }
        if numerator.is_zero() {
            return Some(Self::zero());
        }
        let g = numerator.gcd(&denominator);
        let (n, _) = numerator.div_rem(&g)?;
        let (d, _) = denominator.div_rem(&g)?;
        Some(Self {
            negative,
            numerator: n,
            denominator: d,
        })
    }

    /// Zero.
    pub fn zero() -> Self {
        Self {
            negative: false,
            numerator: Natural::zero(),
            denominator: Natural::from_u64(1),
        }
    }

    /// The integer `v`.
    pub fn integer(v: i64) -> Self {
        Self {
            negative: v < 0,
            numerator: Natural::from_u64(v.unsigned_abs()),
            denominator: Natural::from_u64(1),
        }
    }

    /// The fraction `n / d`; `None` when `d` is zero.
    pub fn ratio(n: i64, d: i64) -> Option<Self> {
        Self::integer(n).div(&Self::integer(d))
    }

    /// The exact value of a finite binary64 number; `None` for NaN or an infinity.
    pub fn from_f64(x: f64) -> Option<Self> {
        if !x.is_finite() {
            return None;
        }
        let bits = x.to_bits();
        let negative = bits >> 63 == 1;
        let exponent = ((bits >> 52) & 0x7ff) as i64;
        let fraction = bits & ((1u64 << 52) - 1);
        let (mantissa, power) = if exponent == 0 {
            (fraction, -1074)
        } else {
            (fraction | (1u64 << 52), exponent - 1075)
        };
        let m = Natural::from_u64(mantissa);
        let one = Natural::from_u64(1);
        if power >= 0 {
            Self::reduced(negative, m.shl(power.unsigned_abs()), one)
        } else {
            Self::reduced(negative, m, one.shl(power.unsigned_abs()))
        }
    }

    /// Parses a plain decimal such as `-3.14159`, exactly (no exponent).
    pub fn parse_decimal(text: &str) -> Option<Self> {
        let (negative, body) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
        let digits = format!("{whole}{fraction}");
        let scale = u32::try_from(fraction.len()).ok()?;
        Self::reduced(
            negative,
            Natural::parse_decimal(&digits)?,
            Natural::pow10(scale),
        )
    }

    /// A lower bound of `sqrt(self)` within `2^-bits` of it: `floor(sqrt(self) * 2^bits) / 2^bits`.
    /// `None` for a negative value.
    pub fn sqrt_within(&self, bits: u32) -> Option<Self> {
        if self.is_negative() {
            return None;
        }
        let scaled = self.numerator.shl(2 * u64::from(bits));
        let (floor, _) = scaled.div_rem(&self.denominator)?;
        Self::reduced(
            false,
            floor.isqrt(),
            Natural::from_u64(1).shl(u64::from(bits)),
        )
    }

    /// Parses the canonical text `p/q` of [`Self::to_fraction`]; `None` for anything else.
    pub fn parse_fraction(text: &str) -> Option<Self> {
        let (negative, body) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (n, d) = body.split_once('/')?;
        Self::reduced(
            negative,
            Natural::parse_decimal(n)?,
            Natural::parse_decimal(d)?,
        )
    }

    /// Whether the value is zero.
    pub fn is_zero(&self) -> bool {
        self.numerator.is_zero()
    }

    /// Whether the value is below zero.
    pub fn is_negative(&self) -> bool {
        self.negative && !self.is_zero()
    }

    /// `-self`.
    pub fn neg(&self) -> Self {
        Self {
            negative: !self.negative && !self.is_zero(),
            ..self.clone()
        }
    }

    /// `|self|`.
    pub fn abs(&self) -> Self {
        Self {
            negative: false,
            ..self.clone()
        }
    }

    /// `self + other`.
    pub fn add(&self, other: &Self) -> Self {
        let a = self.numerator.mul(&other.denominator);
        let b = other.numerator.mul(&self.denominator);
        let d = self.denominator.mul(&other.denominator);
        let (negative, n) = if self.negative == other.negative {
            (self.negative, a.add(&b))
        } else if a.cmp(&b) != Ordering::Less {
            (self.negative, a.sub(&b))
        } else {
            (other.negative, b.sub(&a))
        };
        Self::reduced(negative, n, d).unwrap_or_else(Self::zero)
    }

    /// `self - other`.
    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.neg())
    }

    /// `self * other`.
    pub fn mul(&self, other: &Self) -> Self {
        Self::reduced(
            self.negative != other.negative,
            self.numerator.mul(&other.numerator),
            self.denominator.mul(&other.denominator),
        )
        .unwrap_or_else(Self::zero)
    }

    /// `self / other`; `None` when `other` is zero.
    pub fn div(&self, other: &Self) -> Option<Self> {
        if other.is_zero() {
            return None;
        }
        Self::reduced(
            self.negative != other.negative,
            self.numerator.mul(&other.denominator),
            self.denominator.mul(&other.numerator),
        )
    }

    /// Total order of the values.
    pub fn compare(&self, other: &Self) -> Ordering {
        let d = self.sub(other);
        if d.is_zero() {
            Ordering::Equal
        } else if d.negative {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }

    /// The canonical text `p/q` (an integer prints as `p/1`).
    pub fn to_fraction(&self) -> String {
        format!(
            "{}{}/{}",
            if self.is_negative() { "-" } else { "" },
            self.numerator.to_decimal(),
            self.denominator.to_decimal()
        )
    }

    /// The value in scientific notation with `digits` significant digits, rounded half up in
    /// magnitude, such as `2.39999965429305450000e-3`. Zero prints as `0`.
    pub fn to_scientific(&self, digits: u32) -> String {
        if self.is_zero() {
            return "0".into();
        }
        // Find e with 10^(digits-1) <= |x| * 10^(digits-1-e) < 10^digits.
        let magnitude = self.abs();
        let mut exponent = estimate_exponent(&magnitude.numerator, &magnitude.denominator);
        let low = Natural::pow10(digits.saturating_sub(1));
        let high = Natural::pow10(digits);
        let mut scaled;
        loop {
            let shift = i64::from(digits) - 1 - exponent;
            let (n, d) = if shift >= 0 {
                let p = Natural::pow10(u32::try_from(shift).unwrap_or(0));
                (magnitude.numerator.mul(&p), magnitude.denominator.clone())
            } else {
                let p = Natural::pow10(u32::try_from(-shift).unwrap_or(0));
                (magnitude.numerator.clone(), magnitude.denominator.mul(&p))
            };
            let Some((q, r)) = n.div_rem(&d) else {
                return "0".into();
            };
            // Round half up: compare 2r with d.
            scaled = if r.shl(1).cmp(&d) == Ordering::Less {
                q
            } else {
                q.add(&Natural::from_u64(1))
            };
            if scaled.cmp(&low) == Ordering::Less {
                exponent -= 1;
            } else if scaled.cmp(&high) != Ordering::Less {
                exponent += 1;
            } else {
                break;
            }
        }
        let text = scaled.to_decimal();
        let (lead, rest) = text.split_at(1);
        format!(
            "{}{lead}.{rest}e{exponent}",
            if self.is_negative() { "-" } else { "" }
        )
    }
}

/// A decimal exponent close to `log10(n / d)`, from the bit lengths.
fn estimate_exponent(n: &Natural, d: &Natural) -> i64 {
    let bits = n.bits() as i64 - d.bits() as i64;
    // log10(2) ~ 0.30103; the caller corrects the estimate by one step at a time.
    (bits * 30103).div_euclid(100_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(n: i64, d: i64) -> Rational {
        Rational::ratio(n, d).unwrap()
    }

    #[test]
    fn arithmetic_is_exact_and_reduced() {
        assert_eq!(q(1, 3).add(&q(1, 6)), q(1, 2));
        assert_eq!(q(1, 3).sub(&q(1, 2)), q(-1, 6));
        assert_eq!(q(-2, 3).mul(&q(3, 4)), q(-1, 2));
        assert_eq!(q(1, 2).div(&q(-1, 4)).unwrap(), q(-2, 1));
        assert_eq!(q(6, 4).to_fraction(), "3/2");
        assert_eq!(q(0, 5).to_fraction(), "0/1");
        assert!(q(1, 3).div(&Rational::zero()).is_none());
        assert_eq!(q(1, 3).compare(&q(1, 2)), Ordering::Less);
    }

    #[test]
    fn binary64_values_convert_exactly() {
        assert_eq!(Rational::from_f64(0.5).unwrap(), q(1, 2));
        assert_eq!(Rational::from_f64(-3.0).unwrap(), q(-3, 1));
        // 0.1 is 3602879701896397 / 2^55.
        assert_eq!(
            Rational::from_f64(0.1).unwrap().to_fraction(),
            "3602879701896397/36028797018963968"
        );
        // The smallest subnormal is 2^-1074.
        let tiny = Rational::from_f64(f64::from_bits(1)).unwrap();
        assert_eq!(
            tiny.mul(&Rational::from_f64(2f64.powi(1000)).unwrap())
                .mul(&Rational::from_f64(2f64.powi(74)).unwrap()),
            q(1, 1)
        );
        assert!(Rational::from_f64(f64::NAN).is_none());
    }

    #[test]
    fn large_products_divide_back() {
        let big = Rational::from_f64(1e300).unwrap();
        let small = Rational::from_f64(1e-300).unwrap();
        let product = big.mul(&small);
        assert_eq!(product.div(&small).unwrap(), big);
        assert_ne!(
            product,
            q(1, 1),
            "1e300 * 1e-300 is not exactly one in binary64 inputs"
        );
    }

    #[test]
    fn square_roots_bound_from_below() {
        assert_eq!(q(9, 4).sqrt_within(8).unwrap(), q(3, 2));
        let root3 = q(3, 1).sqrt_within(200).unwrap();
        let square = root3.mul(&root3);
        assert_eq!(square.compare(&q(3, 1)), Ordering::Less);
        let step = Rational::from_f64(2f64.powi(-200)).unwrap();
        let above = root3.add(&step);
        assert_eq!(above.mul(&above).compare(&q(3, 1)), Ordering::Greater);
        assert!(q(-1, 1).sqrt_within(8).is_none());
    }

    #[test]
    fn decimals_parse_and_print() {
        assert_eq!(Rational::parse_decimal("-1.25").unwrap(), q(-5, 4));
        assert_eq!(Rational::parse_decimal("7").unwrap(), q(7, 1));
        assert!(Rational::parse_decimal("1e5").is_none());
        assert_eq!(q(1, 3).to_scientific(5), "3.3333e-1");
        assert_eq!(q(2, 3).to_scientific(5), "6.6667e-1");
        assert_eq!(q(-12345, 1).to_scientific(3), "-1.23e4");
        assert_eq!(q(99999, 1).to_scientific(3), "1.00e5");
        assert_eq!(
            Rational::from_f64(1e-320).unwrap().to_scientific(3),
            "1.00e-320"
        );
        let printed = Rational::from_f64(0.1).unwrap().to_scientific(21);
        assert_eq!(printed, "1.00000000000000005551e-1");
        assert_eq!(printed.parse::<f64>().unwrap(), 0.1);
    }
}
