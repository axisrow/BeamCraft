// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Exact orientation predicates and exact scaled determinants.
//!
//! A static floating-point filter decides the sign when its error bound allows;
//! otherwise the sign is computed in exact integer arithmetic over the scaled
//! binary64 inputs, so a decided sign is never a guess and zero is exact zero.
//! No epsilon from the geometric tolerance policy enters any sign (G04 section 4).
//!
//! Filter derivation. For `orient3d` the nine differences are each rounded
//! once (relative error at most `u = 2^-53`), every one of the six triple
//! products is formed with two multiplications, one subtraction and two sums of
//! at most one rounding each, so each of its terms carries at most
//! `3u + 5u = 8u` relative error. Using twice that margin, `16u` times the
//! computed permanent (sum of absolute terms) bounds the total error. For
//! `orient2d` each term carries at most `2u + u + u = 4u`, and the margin used
//! is `8u`. Underflow adds at most `eta/2` per operation; the filter is skipped
//! when the permanent is below `1e-200`, far above any such absolute term.

use crate::bigint::{Big, Overflow};

/// Unit roundoff `2^-53`.
const U: f64 = 1.110_223_024_625_156_5e-16;
/// Smallest permanent for which the filter is trusted.
const FILTER_FLOOR: f64 = 1e-200;

/// Why an exact decision was not produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PredError {
    /// The exact-evaluation budget was used up.
    Budget,
    /// Values exceeded the fixed integer capacity (outside the coordinate caps).
    Range,
}

impl From<Overflow> for PredError {
    fn from(_: Overflow) -> Self {
        Self::Range
    }
}

/// Splits a finite value into `m * 2^e` with odd (or zero) `m`.
fn decompose(x: f64) -> (i64, i32) {
    let bits = x.to_bits();
    let fraction = bits & ((1u64 << 52) - 1);
    let field = i32::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    let (mut m, mut e) = if field == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1u64 << 52), field - 1075)
    };
    if m == 0 {
        return (0, 0);
    }
    let tz = m.trailing_zeros();
    m >>= tz;
    e += i32::try_from(tz).unwrap_or(0);
    let signed = i64::try_from(m).unwrap_or(0);
    (if bits >> 63 == 1 { -signed } else { signed }, e)
}

/// Smallest binary exponent among the nonzero values, or 0 if all are zero.
pub fn min_exponent(values: impl IntoIterator<Item = f64>) -> i32 {
    values
        .into_iter()
        .map(decompose)
        .filter(|(m, _)| *m != 0)
        .map(|(_, e)| e)
        .min()
        .unwrap_or(0)
}

/// The exact integer `x * 2^-e_min`; requires `e_min <= exponent(x)`.
pub fn scaled(x: f64, e_min: i32) -> Result<Big, Overflow> {
    let (m, e) = decompose(x);
    if m == 0 {
        return Ok(Big::ZERO);
    }
    let shift = u32::try_from(e.checked_sub(e_min).ok_or(Overflow)?).map_err(|_| Overflow)?;
    Big::from_i64_shifted(m, shift)
}

fn scaled3(p: [f64; 3], e: i32) -> Result<[Big; 3], Overflow> {
    let [x, y, z] = p;
    Ok([scaled(x, e)?, scaled(y, e)?, scaled(z, e)?])
}

fn diff3(a: &[Big; 3], b: &[Big; 3]) -> Result<[Big; 3], Overflow> {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    Ok([ax.sub(bx)?, ay.sub(by)?, az.sub(bz)?])
}

/// Exact `u x v`.
fn cross_big(u: &[Big; 3], v: &[Big; 3]) -> Result<[Big; 3], Overflow> {
    let [ux, uy, uz] = u;
    let [vx, vy, vz] = v;
    Ok([
        uy.mul(vz)?.sub(&uz.mul(vy)?)?,
        uz.mul(vx)?.sub(&ux.mul(vz)?)?,
        ux.mul(vy)?.sub(&uy.mul(vx)?)?,
    ])
}

/// Exact determinant of three rows, `r0 . (r1 x r2)`.
fn det3(r0: &[Big; 3], r1: &[Big; 3], r2: &[Big; 3]) -> Result<Big, Overflow> {
    let [x, y, z] = r0;
    let [cx, cy, cz] = cross_big(r1, r2)?;
    x.mul(&cx)?.add(&y.mul(&cy)?)?.add(&z.mul(&cz)?)
}

/// Exact `a . (b x c)` in units of `2^(3e)`; positive for an outward triangle.
pub(crate) fn triangle_det(e: i32, a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Result<Big, Overflow> {
    det3(&scaled3(a, e)?, &scaled3(b, e)?, &scaled3(c, e)?)
}

/// Exact `(b - a) x (c - a)` in units of `2^(2e)`.
pub fn triangle_cross(e: i32, a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Result<[Big; 3], Overflow> {
    let a = scaled3(a, e)?;
    cross_big(&diff3(&scaled3(b, e)?, &a)?, &diff3(&scaled3(c, e)?, &a)?)
}

/// Exact `a + b + c` per axis in units of `2^e`.
pub(crate) fn coordinate_sum(
    e: i32,
    a: [f64; 3],
    b: [f64; 3],
    c: [f64; 3],
) -> Result<[Big; 3], Overflow> {
    let (a, b, c) = (scaled3(a, e)?, scaled3(b, e)?, scaled3(c, e)?);
    let [ax, ay, az] = &a;
    let [bx, by, bz] = &b;
    let [cx, cy, cz] = &c;
    Ok([
        ax.add(bx)?.add(cx)?,
        ay.add(by)?.add(cy)?,
        az.add(bz)?.add(cz)?,
    ])
}

const fn sign_of(x: f64) -> i8 {
    if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }
}

/// The 2D point obtained by dropping axis `k` and keeping the cyclic order `k+1, k+2`.
///
/// The 2D orientation of a projected triangle then has the sign of the
/// triangle's normal component `n_k`.
pub const fn project(p: [f64; 3], k: usize) -> [f64; 2] {
    let [x, y, z] = p;
    match k {
        0 => [y, z],
        1 => [z, x],
        _ => [x, y],
    }
}

/// Orientation predicates with a bounded count of exact fallbacks.
#[derive(Debug)]
pub(crate) struct Predicates {
    exact_calls: u64,
    exact_limit: u64,
}

impl Predicates {
    /// Allows at most `exact_limit` exact fallbacks.
    pub(crate) const fn new(exact_limit: u64) -> Self {
        Self {
            exact_calls: 0,
            exact_limit,
        }
    }

    /// Exact fallbacks consumed so far.
    pub(crate) const fn exact_calls(&self) -> u64 {
        self.exact_calls
    }

    fn charge(&mut self) -> Result<(), PredError> {
        if self.exact_calls >= self.exact_limit {
            return Err(PredError::Budget);
        }
        self.exact_calls += 1;
        Ok(())
    }

    /// Sign of `(a-d) . ((b-d) x (c-d))`: positive when `d` lies on the clockwise
    /// side of triangle `abc` (opposite its counter-clockwise normal).
    pub(crate) fn orient3d(
        &mut self,
        a: [f64; 3],
        b: [f64; 3],
        c: [f64; 3],
        d: [f64; 3],
    ) -> Result<i8, PredError> {
        let [adx, ady, adz] = [a[0] - d[0], a[1] - d[1], a[2] - d[2]];
        let [bdx, bdy, bdz] = [b[0] - d[0], b[1] - d[1], b[2] - d[2]];
        let [cdx, cdy, cdz] = [c[0] - d[0], c[1] - d[1], c[2] - d[2]];
        let (bdycdz, bdzcdy) = (bdy * cdz, bdz * cdy);
        let (bdzcdx, bdxcdz) = (bdz * cdx, bdx * cdz);
        let (bdxcdy, bdycdx) = (bdx * cdy, bdy * cdx);
        let det = adx * (bdycdz - bdzcdy) + ady * (bdzcdx - bdxcdz) + adz * (bdxcdy - bdycdx);
        let permanent = adx.abs() * (bdycdz.abs() + bdzcdy.abs())
            + ady.abs() * (bdzcdx.abs() + bdxcdz.abs())
            + adz.abs() * (bdxcdy.abs() + bdycdx.abs());
        if permanent > FILTER_FLOOR && det.abs() > 16.0 * U * permanent {
            return Ok(sign_of(det));
        }
        self.charge()?;
        let e = min_exponent([a, b, c, d].into_iter().flatten());
        let d = scaled3(d, e)?;
        let rows = [
            diff3(&scaled3(a, e)?, &d)?,
            diff3(&scaled3(b, e)?, &d)?,
            diff3(&scaled3(c, e)?, &d)?,
        ];
        let [r0, r1, r2] = &rows;
        Ok(det3(r0, r1, r2)?.sign())
    }

    /// Sign of `(a-c) x (b-c)`: positive when `a, b, c` turn counter-clockwise.
    pub(crate) fn orient2d(
        &mut self,
        a: [f64; 2],
        b: [f64; 2],
        c: [f64; 2],
    ) -> Result<i8, PredError> {
        let left = (a[0] - c[0]) * (b[1] - c[1]);
        let right = (a[1] - c[1]) * (b[0] - c[0]);
        let det = left - right;
        let permanent = left.abs() + right.abs();
        if permanent > FILTER_FLOOR && det.abs() > 8.0 * U * permanent {
            return Ok(sign_of(det));
        }
        self.charge()?;
        let e = min_exponent([a, b, c].into_iter().flatten());
        let s = |x: f64| scaled(x, e);
        let (acx, acy) = (s(a[0])?.sub(&s(c[0])?)?, s(a[1])?.sub(&s(c[1])?)?);
        let (bcx, bcy) = (s(b[0])?.sub(&s(c[0])?)?, s(b[1])?.sub(&s(c[1])?)?);
        Ok(acx.mul(&bcy)?.sub(&acy.mul(&bcx)?)?.sign())
    }

    /// First axis `k` whose normal component of triangle `abc` is exactly nonzero,
    /// with that component's sign; `None` when the points are exactly collinear.
    pub(crate) fn normal_axis(
        &mut self,
        a: [f64; 3],
        b: [f64; 3],
        c: [f64; 3],
    ) -> Result<Option<(usize, i8)>, PredError> {
        // The float normal only orders the axes (largest component first, so the
        // exact test usually succeeds on the filter); decisions stay exact.
        let n = [
            (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]),
            (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]),
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]),
        ];
        let magnitude = |k: usize| n.get(k).map_or(0.0, |v| v.abs());
        let mut order = [0usize, 1, 2];
        order.sort_by(|x, y| magnitude(*y).total_cmp(&magnitude(*x)));
        for k in order {
            let s = self.orient2d(project(a, k), project(b, k), project(c, k))?;
            if s != 0 {
                return Ok(Some((k, s)));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independent integer oracle: i128 determinant of small integer coordinates.
    fn oracle3(a: [i64; 3], b: [i64; 3], c: [i64; 3], d: [i64; 3]) -> i8 {
        let r = |p: [i64; 3]| [0, 1, 2].map(|i| i128::from(p[i]) - i128::from(d[i]));
        let (x, y, z) = (r(a), r(b), r(c));
        let det = x[0] * (y[1] * z[2] - y[2] * z[1]) - x[1] * (y[0] * z[2] - y[2] * z[0])
            + x[2] * (y[0] * z[1] - y[1] * z[0]);
        det.signum() as i8
    }

    fn f(p: [i64; 3]) -> [f64; 3] {
        p.map(|v| v as f64)
    }

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }
        fn small(&mut self, range: i64) -> i64 {
            (self.next() % (2 * range as u64 + 1)) as i64 - range
        }
    }

    #[test]
    fn orient3d_matches_integer_oracle_including_exact_zero() {
        let mut rng = Lcg(0x9e37_79b9);
        let mut zeros = 0;
        for round in 0..4000 {
            let range = if round % 2 == 0 { 3 } else { 1_000_000 };
            let mut p = [[0i64; 3]; 4];
            for q in &mut p {
                for v in q.iter_mut() {
                    *v = rng.small(range);
                }
            }
            if round % 4 == 1 {
                // Force coplanarity: d = a + b - c gives an exactly planar quadruple.
                p[3] = [0, 1, 2].map(|i| p[0][i] + p[1][i] - p[2][i]);
            }
            let want = oracle3(p[0], p[1], p[2], p[3]);
            let got = Predicates::new(u64::MAX)
                .orient3d(f(p[0]), f(p[1]), f(p[2]), f(p[3]))
                .unwrap();
            assert_eq!(got, want, "{p:?}");
            zeros += usize::from(want == 0);
        }
        assert!(zeros > 500);
    }

    #[test]
    fn orient2d_matches_integer_oracle() {
        let mut rng = Lcg(7);
        for _ in 0..3000 {
            let p: [[i64; 2]; 3] = [0; 3].map(|_| [rng.small(4), rng.small(4)]);
            let want = ((i128::from(p[0][0] - p[2][0]) * i128::from(p[1][1] - p[2][1]))
                - (i128::from(p[0][1] - p[2][1]) * i128::from(p[1][0] - p[2][0])))
            .signum() as i8;
            let q = p.map(|v| v.map(|x| x as f64));
            assert_eq!(
                Predicates::new(u64::MAX)
                    .orient2d(q[0], q[1], q[2])
                    .unwrap(),
                want
            );
        }
    }

    #[test]
    fn decisions_survive_extreme_scales_and_tiny_offsets() {
        let mut p = Predicates::new(u64::MAX);
        for k in [-1000, -500, -60, 0, 60, 500, 900] {
            let s = 2f64.powi(k);
            let (a, b, c) = ([0.0, 0.0, 0.0], [s, 0.0, 0.0], [0.0, s, 0.0]);
            // d is exactly in the plane z = 0, then one binary step above or below it.
            assert_eq!(p.orient3d(a, b, c, [s, s, 0.0]).unwrap(), 0);
            let up = f64::from_bits(1);
            assert_eq!(p.orient3d(a, b, c, [s, s, up]).unwrap(), -1);
            assert_eq!(p.orient3d(a, b, c, [s, s, -up]).unwrap(), 1);
        }
        // Near-planar at large magnitude: only an exact decision is correct.
        let big = 2f64.powi(30);
        let (a, b, c) = ([big, 0.0, 0.0], [0.0, big, 0.0], [0.0, 0.0, big]);
        let on = [big / 3.0, big / 3.0, big / 3.0];
        let off = [big / 2.0, big / 2.0, big / 2.0];
        assert_eq!(p.orient3d(a, b, c, on).unwrap().abs(), 1);
        assert_eq!(p.orient3d(a, b, c, off).unwrap(), -1);
        assert_eq!(p.orient3d(a, b, c, [0.0, 0.0, 0.0]).unwrap(), 1);
    }

    #[test]
    fn antisymmetry_and_budget() {
        let mut p = Predicates::new(0);
        let (a, b, c, d) = ([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, 0.3, 1.0]);
        let s = p.orient3d(a, b, c, d).unwrap();
        assert_eq!(p.orient3d(b, a, c, d).unwrap(), -s);
        // A coplanar quadruple needs the exact path, which a zero budget refuses.
        assert_eq!(p.orient3d(a, b, c, [0.5, 0.5, 0.0]), Err(PredError::Budget));
        assert_eq!(p.exact_calls, 0);
    }

    #[test]
    fn exact_determinant_terms_match_integer_arithmetic() {
        let (a, b, c) = ([1.0, 2.0, 3.0], [4.0, 5.0, 6.5], [-7.0, 8.0, 9.0]);
        let e = min_exponent([a, b, c].into_iter().flatten());
        assert_eq!(e, -1);
        let det = triangle_det(e, a, b, c).unwrap();
        let exact = 1.0 * (5.0 * 9.0 - 6.5 * 8.0) - 2.0 * (4.0 * 9.0 - 6.5 * -7.0)
            + 3.0 * (4.0 * 8.0 - 5.0 * -7.0);
        let enclosed = det.enclose(3 * e).unwrap();
        assert!(enclosed.contains(exact), "{enclosed:?} {exact}");
        let [cx, cy, cz] = triangle_cross(e, a, b, c).unwrap();
        let cross = [0, 1, 2].map(|i| {
            let (u, v) = (
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
            );
            [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ][i]
        });
        assert!(cx.enclose(2 * e).unwrap().contains(cross[0]));
        assert!(cy.enclose(2 * e).unwrap().contains(cross[1]));
        assert!(cz.enclose(2 * e).unwrap().contains(cross[2]));
    }
}
