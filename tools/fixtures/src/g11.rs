// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The G11 accuracy fixtures: original synthetic inputs and their exact rational references.
//!
//! The set exercises four classes the kernel must stay accurate on: rounded-mesh cross-checks,
//! subnormal mass, very large coordinates and cancellation-heavy inputs. Inputs are binary64
//! values; every reference is computed from their exact rational values with [`Rational`], with
//! no kernel, engine or production helper. Two references are not rational: π (a 64-digit
//! decimal, error below 1e-64) and square roots (bounded below within 2^-200); such rows are
//! marked `bounded`. The harness that runs the real builders against these files is
//! `crates/steel/tests/g11_accuracy.rs`; the derivations are in `docs/fixtures/g11/README.md`.

use crate::rational::Rational;
use crate::sha256::hex_digest;

/// Repository location of the committed G11 set.
pub const REPOSITORY_DIR: &str = "docs/fixtures/g11";
/// Version of the G11 set. Increase it whenever a case, a reference or the format changes.
pub const SET_VERSION: u32 = 1;
/// The generated files, in manifest order.
pub const FILES: [&str; 3] = ["cases.tsv", "expectations.tsv", "manifest.tsv"];

/// Significant digits of every printed decimal reference.
const DIGITS: u32 = 21;
/// Bits of every square root bound.
const ROOT_BITS: u32 = 200;
/// π to 64 decimals; the truncation error is below 1e-64.
const PI: &str = "3.1415926535897932384626433832795028841971693993751058209749445923";
/// The smallest positive normal binary64 value, 2^-1022.
const MIN_NORMAL: f64 = f64::MIN_POSITIVE;

/// A member profile.
#[derive(Debug, Clone)]
pub enum Profile {
    /// The first-scope centred rectangle, width and height.
    Rectangle([f64; 2]),
    /// A parametric section family token and its values in the family's stored order.
    Section(&'static str, Vec<f64>),
}

/// A fastener component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The washer under the head.
    Washer,
    /// The hexagonal head.
    Head,
    /// The round shank.
    Shank,
    /// The hexagonal nut.
    Nut,
}

impl Part {
    fn token(self) -> &'static str {
        match self {
            Self::Washer => "washer",
            Self::Head => "head",
            Self::Shank => "shank",
            Self::Nut => "nut",
        }
    }
}

/// The dimensions of a bolt prototype, millimetres.
#[derive(Debug, Clone, Copy)]
pub struct Bolt {
    /// Nominal shank diameter.
    pub diameter: f64,
    /// Head across flats.
    pub head_af: f64,
    /// Head height.
    pub head_h: f64,
    /// Nut across flats.
    pub nut_af: f64,
    /// Nut height.
    pub nut_h: f64,
    /// Washer outer diameter, inner diameter and thickness.
    pub washer: [f64; 3],
    /// Shank length.
    pub length: f64,
}

/// The body a case builds.
#[derive(Debug, Clone)]
pub enum Body {
    /// A plate: a counter-clockwise contour in the element-local plane `z = 0` and a thickness laid
    /// off towards `+Z` (the default `top` side).
    Plate {
        /// Contour points.
        points: Vec<[f64; 2]>,
        /// Thickness.
        thickness: f64,
    },
    /// A straight member between two element-local axis points.
    Member {
        /// The profile.
        profile: Profile,
        /// Axis start.
        start: [f64; 3],
        /// Axis end.
        end: [f64; 3],
    },
    /// One component of a bolt prototype built with a washer under the head.
    Fastener {
        /// The component.
        part: Part,
        /// The bolt.
        bolt: Bolt,
    },
}

/// The rigid rotation of a case's placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    /// No rotation.
    Identity,
    /// A half turn about `+Z`, quaternion `(0, 0, 1, 0)`: exact in binary64.
    HalfZ,
}

/// One fixture.
#[derive(Debug, Clone)]
pub struct Case {
    /// Stable identity `g11.<family>.<name>`.
    pub id: String,
    /// The class of the case: `mesh`, `subnormal`, `far` or `cancel`.
    pub family: &'static str,
    /// The body.
    pub body: Body,
    /// The material density in kg/m3, when one is authored.
    pub density: Option<f64>,
    /// The placement translation in millimetres.
    pub translation: [f64; 3],
    /// The placement rotation.
    pub turn: Turn,
}

/// The references of one case.
#[derive(Debug, Clone)]
pub struct Expectation {
    /// `ok`, or the F04 code of the expected refusal.
    pub outcome: &'static str,
    /// Whether every reference is an exact rational (`false`: it uses π or a square root).
    pub exact: bool,
    /// Whether the reference is of an ideal circle the kernel approximates by a polygon.
    pub curved: bool,
    /// Volume in mm3.
    pub volume: Rational,
    /// Mass in kg, when a density is authored and the outcome is `ok`.
    pub mass: Option<Rational>,
    /// Section area in mm2 (members).
    pub area: Option<Rational>,
    /// Axis length in mm (members).
    pub axis: Option<Rational>,
    /// World bounding box (plates).
    pub bounds: Option<[[Rational; 3]; 2]>,
    /// World volume centroid (plates and doubly symmetric members).
    pub centroid: Option<[Rational; 3]>,
}

fn r(x: f64) -> Result<Rational, String> {
    Rational::from_f64(x).ok_or_else(|| format!("non-finite input {x}"))
}

fn pi() -> Result<Rational, String> {
    Rational::parse_decimal(PI).ok_or_else(|| "pi constant".to_owned())
}

fn int(v: i64) -> Rational {
    Rational::integer(v)
}

fn half() -> Rational {
    Rational::ratio(1, 2).unwrap_or_else(Rational::zero)
}

fn div(a: &Rational, b: &Rational) -> Result<Rational, String> {
    a.div(b).ok_or_else(|| "division by zero".to_owned())
}

fn sum(values: &[Rational]) -> Rational {
    values.iter().fold(Rational::zero(), |acc, v| acc.add(v))
}

/// The parametric sections of the set, written as closed forms of the stored values.
///
/// `K = 1 - π/4` is the area between a quarter circle of unit radius and its square. Returns the
/// area and whether it uses π.
fn section_area(family: &str, values: &[f64]) -> Result<(Rational, bool), String> {
    let v: Vec<Rational> = values.iter().map(|x| r(*x)).collect::<Result<_, _>>()?;
    let k = int(1).sub(&div(&pi()?, &int(4))?);
    let sq = |x: &Rational| x.mul(x);
    // Cold-formed bends: `count` quarter annuli of inner radius r replace their sharp squares.
    // An inner radius of zero is a sharp section (sharp outer corners too): no correction.
    let bends = |radius: &Rational, t: &Rational, count: i64| {
        if radius.is_zero() {
            return Rational::zero();
        }
        let outer = radius.add(t);
        int(count).mul(&k).mul(&sq(radius).sub(&sq(&outer)))
    };
    let area = match (family, v.as_slice()) {
        ("i_shape", [h, b, tw, tf, rad]) => sum(&[
            int(2).mul(b).mul(tf),
            h.sub(&int(2).mul(tf)).mul(tw),
            int(4).mul(&k).mul(&sq(rad)),
        ]),
        ("channel", [h, b, tw, tf, rad]) => sum(&[
            int(2).mul(b).mul(tf),
            h.sub(&int(2).mul(tf)).mul(tw),
            int(2).mul(&k).mul(&sq(rad)),
        ]),
        ("angle", [h, b, t, rad]) => t.mul(&h.add(b).sub(t)).add(&k.mul(&sq(rad))),
        ("rhs", [h, b, t, rad]) => {
            let inner = if rad.compare(t).is_gt() {
                rad.sub(t)
            } else {
                Rational::zero()
            };
            // The wall written without a difference of near-equal areas.
            int(2)
                .mul(t)
                .mul(&h.add(b).sub(&int(2).mul(t)))
                .sub(&int(4).mul(&k).mul(&sq(rad).sub(&sq(&inner))))
        }
        ("chs", [d, t]) => pi()?.mul(t).mul(&d.sub(t)),
        ("plain_c", [h, b, t, rad]) => t
            .mul(&h.add(&int(2).mul(b)).sub(&int(2).mul(t)))
            .add(&bends(rad, t, 2)),
        ("lipped_c" | "lipped_z", [h, b, c, t, rad]) => t
            .mul(
                &h.add(&int(2).mul(b))
                    .add(&int(2).mul(c))
                    .sub(&int(4).mul(t)),
            )
            .add(&bends(rad, t, 4)),
        ("omega", [h, b, c, t, rad]) => t
            .mul(
                &b.add(&int(2).mul(h))
                    .add(&int(2).mul(c))
                    .sub(&int(4).mul(t)),
            )
            .add(&bends(rad, t, 4)),
        _ => return Err(format!("unknown section {family} with {} values", v.len())),
    };
    let curved = family == "chs" || v.last().is_some_and(|rad| !rad.is_zero());
    Ok((area, curved))
}

/// Exact square root when the value is a perfect square of the bound, else the bound.
fn root(value: &Rational) -> Result<(Rational, bool), String> {
    let low = value
        .sqrt_within(ROOT_BITS)
        .ok_or_else(|| "negative square".to_owned())?;
    let exact = low.mul(&low) == *value;
    Ok((low, exact))
}

fn place(p: [Rational; 3], turn: Turn, t: &[Rational; 3]) -> [Rational; 3] {
    let [x, y, z] = p;
    let (x, y) = match turn {
        Turn::Identity => (x, y),
        Turn::HalfZ => (x.neg(), y.neg()),
    };
    let [tx, ty, tz] = t;
    [x.add(tx), y.add(ty), z.add(tz)]
}

fn min_max(a: &Rational, b: &Rational) -> (Rational, Rational) {
    if a.compare(b).is_le() {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

/// Volume, world bounds and world centroid of a plate.
type PlateReference = (Rational, [[Rational; 3]; 2], [Rational; 3]);

fn plate(points: &[[f64; 2]], thickness: f64, case: &Case) -> Result<PlateReference, String> {
    let pts: Vec<[Rational; 2]> = points
        .iter()
        .map(|[x, y]| Ok([r(*x)?, r(*y)?]))
        .collect::<Result<_, String>>()?;
    let mut twice_area = Rational::zero();
    let mut mx = Rational::zero();
    let mut my = Rational::zero();
    for (p, q) in pts.iter().zip(pts.iter().cycle().skip(1)) {
        let [x0, y0] = p;
        let [x1, y1] = q;
        let cross = x0.mul(y1).sub(&x1.mul(y0));
        twice_area = twice_area.add(&cross);
        mx = mx.add(&x0.add(x1).mul(&cross));
        my = my.add(&y0.add(y1).mul(&cross));
    }
    if !twice_area.compare(&Rational::zero()).is_gt() {
        return Err(format!("{}: contour is not counter-clockwise", case.id));
    }
    let t = r(thickness)?;
    let area = twice_area.mul(&half());
    let volume = area.mul(&t);
    let six_area = int(3).mul(&twice_area);
    let local_centroid = [div(&mx, &six_area)?, div(&my, &six_area)?, t.mul(&half())];
    let first = pts.first().ok_or("empty contour")?;
    let (mut lo, mut hi) = (first.clone(), first.clone());
    for p in &pts {
        for i in 0..2 {
            if let (Some(l), Some(h), Some(v)) = (lo.get_mut(i), hi.get_mut(i), p.get(i)) {
                if v.compare(l).is_lt() {
                    *l = v.clone();
                }
                if v.compare(h).is_gt() {
                    *h = v.clone();
                }
            }
        }
    }
    let translation = [
        r(case.translation[0])?,
        r(case.translation[1])?,
        r(case.translation[2])?,
    ];
    let [lx, ly] = lo;
    let [hx, hy] = hi;
    let a = place([lx, ly, Rational::zero()], case.turn, &translation);
    let b = place([hx, hy, t], case.turn, &translation);
    let [(x0, x1), (y0, y1), (z0, z1)] = [
        min_max(&a[0], &b[0]),
        min_max(&a[1], &b[1]),
        min_max(&a[2], &b[2]),
    ];
    let centroid = place(local_centroid, case.turn, &translation);
    Ok((volume, [[x0, y0, z0], [x1, y1, z1]], centroid))
}

fn mass(volume: &Rational, density: Option<f64>) -> Result<Option<Rational>, String> {
    density
        .map(|rho| div(&volume.mul(&r(rho)?), &int(1_000_000_000)))
        .transpose()
}

/// The references of one case, from its inputs alone.
pub fn expectation(case: &Case) -> Result<Expectation, String> {
    let mut e = match &case.body {
        Body::Plate { points, thickness } => {
            let (volume, bounds, centroid) = plate(points, *thickness, case)?;
            Expectation {
                outcome: "ok",
                exact: true,
                curved: false,
                mass: mass(&volume, case.density)?,
                volume,
                area: None,
                axis: None,
                bounds: Some(bounds),
                centroid: Some(centroid),
            }
        }
        Body::Member {
            profile,
            start,
            end,
        } => {
            let (area, curved, symmetric) = match profile {
                Profile::Rectangle([w, h]) => (r(*w)?.mul(&r(*h)?), false, true),
                Profile::Section(family, values) => {
                    let (area, curved) = section_area(family, values)?;
                    (area, curved, matches!(*family, "i_shape" | "rhs" | "chs"))
                }
            };
            let s: Vec<Rational> = start.iter().map(|x| r(*x)).collect::<Result<_, _>>()?;
            let f: Vec<Rational> = end.iter().map(|x| r(*x)).collect::<Result<_, _>>()?;
            let squares = sum(&s
                .iter()
                .zip(&f)
                .map(|(a, b)| {
                    let d = b.sub(a);
                    d.mul(&d)
                })
                .collect::<Vec<_>>());
            let (axis, exact_root) = root(&squares)?;
            let volume = area.mul(&axis);
            let centroid = if symmetric {
                let translation = [
                    r(case.translation[0])?,
                    r(case.translation[1])?,
                    r(case.translation[2])?,
                ];
                let mid = |i: usize| match (s.get(i), f.get(i)) {
                    (Some(a), Some(b)) => a.add(b).mul(&half()),
                    _ => Rational::zero(),
                };
                Some(place([mid(0), mid(1), mid(2)], case.turn, &translation))
            } else {
                None
            };
            Expectation {
                outcome: "ok",
                exact: exact_root && !curved,
                curved,
                mass: mass(&volume, case.density)?,
                volume,
                area: Some(area),
                axis: Some(axis),
                bounds: None,
                centroid,
            }
        }
        Body::Fastener { part, bolt } => {
            let hexagon = |af: f64, h: f64| -> Result<Rational, String> {
                // Area of a regular hexagon across flats s: (sqrt(3)/2) * s^2.
                let (root3, _) = root(&int(3))?;
                Ok(root3.mul(&half()).mul(&r(af)?).mul(&r(af)?).mul(&r(h)?))
            };
            let disc = |d: f64| -> Result<Rational, String> {
                Ok(pi()?
                    .mul(&r(d)?)
                    .mul(&r(d)?)
                    .mul(&Rational::ratio(1, 4).ok_or("1/4")?))
            };
            let [od, id, wt] = bolt.washer;
            let (volume, curved) = match part {
                Part::Head => (hexagon(bolt.head_af, bolt.head_h)?, false),
                Part::Nut => (hexagon(bolt.nut_af, bolt.nut_h)?, false),
                Part::Shank => (disc(bolt.diameter)?.mul(&r(bolt.length)?), true),
                Part::Washer => (disc(od)?.sub(&disc(id)?).mul(&r(wt)?), true),
            };
            Expectation {
                outcome: "ok",
                exact: false,
                curved,
                volume,
                mass: None,
                area: None,
                axis: None,
                bounds: None,
                centroid: None,
            }
        }
    };
    // F06 section 8: a positive-density mass below the normal binary64 range is numeric
    // uncertainty, never a current value.
    if let Some(m) = &e.mass {
        let floor = r(MIN_NORMAL)?;
        if !m.is_zero() {
            let ratio = div(m, &floor)?;
            let near = ratio.compare(&Rational::ratio(1, 4).ok_or("1/4")?).is_gt()
                && ratio.compare(&int(4)).is_lt();
            if near {
                return Err(format!(
                    "{}: mass is within a factor 4 of the normal floor",
                    case.id
                ));
            }
            if m.compare(&floor).is_lt() {
                e.outcome = "UNSUPPORTED_GEOMETRY";
                e.mass = None;
            }
        }
    }
    Ok(e)
}

fn rect(w: f64, l: f64) -> Vec<[f64; 2]> {
    vec![[0.0, 0.0], [w, 0.0], [w, l], [0.0, l]]
}

fn shifted(points: &[[f64; 2]], dx: f64, dy: f64) -> Vec<[f64; 2]> {
    points.iter().map(|[x, y]| [x + dx, y + dy]).collect()
}

fn case(id: &str, family: &'static str, body: Body) -> Case {
    Case {
        id: format!("g11.{family}.{id}"),
        family,
        body,
        density: None,
        translation: [0.0; 3],
        turn: Turn::Identity,
    }
}

fn with_density(mut c: Case, density: f64) -> Case {
    c.density = Some(density);
    c
}

fn placed(mut c: Case, translation: [f64; 3], turn: Turn) -> Case {
    c.translation = translation;
    c.turn = turn;
    c
}

fn plate_body(points: Vec<[f64; 2]>, thickness: f64) -> Body {
    Body::Plate { points, thickness }
}

fn member_body(profile: Profile, start: [f64; 3], end: [f64; 3]) -> Body {
    Body::Member {
        profile,
        start,
        end,
    }
}

fn section(family: &'static str, values: &[f64]) -> Profile {
    Profile::Section(family, values.to_vec())
}

/// Every case of the set, in file order.
pub fn cases() -> Vec<Case> {
    const STEEL: f64 = 7850.0;
    let origin = [0.0; 3];
    let flat = [3000.0, 4000.0, 0.0];
    let skew_start = [100.5, 200.25, 300.125];
    let skew_end = [400.5, 600.25, 1500.125];
    let pentagon = vec![[0.0, 0.0], [0.3, 0.0], [0.3, 0.7], [0.1, 0.9], [0.0, 0.7]];
    let concave = vec![
        [0.0, 0.0],
        [500.5, 0.0],
        [500.5, 100.1],
        [100.1, 100.1],
        [100.1, 300.3],
        [0.0, 300.3],
    ];
    let sliver = vec![[0.0, 0.0], [1000.0, 0.0], [500.0, 0.001]];
    let near = 1e9 - 1000.0;
    let far_start = [999_998_000.0; 3];
    let far_end = [999_998_100.0, 999_998_400.0, 999_998_800.0];
    let m16 = Bolt {
        diameter: 16.0,
        head_af: 24.0,
        head_h: 10.0,
        nut_af: 24.0,
        nut_h: 14.8,
        washer: [30.0, 17.0, 3.0],
        length: 60.0,
    };
    let odd = Bolt {
        diameter: 20.3,
        head_af: 30.7,
        head_h: 12.9,
        nut_af: 30.7,
        nut_h: 18.1,
        washer: [37.3, 21.1, 3.7],
        length: 77.7,
    };
    let rhs_far = section("rhs", &[100.3, 50.1, 5.7, 0.0]);
    let mut all = vec![
        // Rounded-mesh cross-checks.
        with_density(
            case("plate_decimal_box", "mesh", plate_body(rect(0.1, 0.3), 0.7)),
            STEEL,
        ),
        case(
            "plate_thirds",
            "mesh",
            plate_body(rect(333.3333, 777.7777), 12.3),
        ),
        case("plate_pentagon", "mesh", plate_body(pentagon.clone(), 0.01)),
        with_density(
            case("plate_concave", "mesh", plate_body(concave, 10.1)),
            STEEL,
        ),
        with_density(
            case(
                "member_rectangle",
                "mesh",
                member_body(Profile::Rectangle([200.1, 300.7]), origin, flat),
            ),
            STEEL,
        ),
        case(
            "member_i_sharp",
            "mesh",
            member_body(
                section("i_shape", &[300.3, 150.1, 7.1, 10.7, 0.0]),
                origin,
                flat,
            ),
        ),
        case(
            "member_channel_sharp",
            "mesh",
            member_body(
                section("channel", &[200.2, 75.3, 8.5, 11.5, 0.0]),
                skew_start,
                skew_end,
            ),
        ),
        case(
            "member_angle_sharp",
            "mesh",
            member_body(
                section("angle", &[100.1, 65.3, 8.2, 0.0]),
                skew_start,
                skew_end,
            ),
        ),
        with_density(
            case(
                "member_rhs_sharp",
                "mesh",
                member_body(section("rhs", &[200.2, 100.1, 6.3, 0.0]), origin, flat),
            ),
            STEEL,
        ),
        case(
            "member_plain_c_sharp",
            "mesh",
            member_body(
                section("plain_c", &[150.1, 50.3, 3.2, 0.0]),
                skew_start,
                skew_end,
            ),
        ),
        case(
            "member_lipped_c_sharp",
            "mesh",
            member_body(
                section("lipped_c", &[200.2, 70.1, 20.3, 2.5, 0.0]),
                skew_start,
                skew_end,
            ),
        ),
        case(
            "member_lipped_z_sharp",
            "mesh",
            member_body(
                section("lipped_z", &[200.2, 70.1, 20.3, 2.5, 0.0]),
                skew_start,
                skew_end,
            ),
        ),
        case(
            "member_omega_sharp",
            "mesh",
            member_body(
                section("omega", &[80.3, 50.1, 20.2, 2.0, 0.0]),
                skew_start,
                skew_end,
            ),
        ),
        with_density(
            case(
                "member_i_rounded",
                "mesh",
                member_body(
                    section("i_shape", &[300.3, 150.1, 7.1, 10.7, 15.0]),
                    origin,
                    flat,
                ),
            ),
            STEEL,
        ),
        case(
            "member_rhs_rounded",
            "mesh",
            member_body(section("rhs", &[200.2, 100.1, 6.3, 9.45]), origin, flat),
        ),
        case(
            "member_chs",
            "mesh",
            member_body(section("chs", &[168.3, 7.1]), skew_start, skew_end),
        ),
        case(
            "member_lipped_c_rounded",
            "mesh",
            member_body(
                section("lipped_c", &[200.2, 70.1, 20.3, 2.5, 3.75]),
                skew_start,
                skew_end,
            ),
        ),
    ];
    for (name, bolt) in [("m16", m16), ("odd", odd)] {
        for part in [Part::Washer, Part::Head, Part::Shank, Part::Nut] {
            all.push(case(
                &format!("bolt_{name}_{}", part.token()),
                "mesh",
                Body::Fastener { part, bolt },
            ));
        }
    }
    all.extend([
        // Mass at the bottom of the binary64 range.
        with_density(
            case(
                "min_plate_tiny_density",
                "subnormal",
                plate_body(rect(0.001, 0.001), 0.001),
            ),
            1e-300,
        ),
        with_density(
            case(
                "unit_plate_1e310",
                "subnormal",
                plate_body(rect(1000.0, 1000.0), 1.0),
            ),
            1e-310,
        ),
        with_density(
            case(
                "largest_plate_smallest_density",
                "subnormal",
                plate_body(rect(1e6, 1e6), 1e6),
            ),
            f64::from_bits(1),
        ),
        with_density(
            case(
                "min_normal_density",
                "subnormal",
                plate_body(rect(1000.0, 1000.0), 10000.0),
            ),
            MIN_NORMAL,
        ),
        with_density(
            case(
                "normal_small_mass",
                "subnormal",
                plate_body(rect(1000.0, 1000.0), 1000.0),
            ),
            1e-300,
        ),
        with_density(
            case(
                "cap_density_min_plate",
                "subnormal",
                plate_body(rect(0.001, 0.001), 0.001),
            ),
            1e6,
        ),
        with_density(
            case(
                "zero_density",
                "subnormal",
                plate_body(rect(100.0, 200.0), 10.0),
            ),
            0.0,
        ),
        with_density(
            case(
                "member_tiny_mass",
                "subnormal",
                member_body(Profile::Rectangle([0.5, 0.25]), origin, [1000.0, 0.0, 0.0]),
            ),
            1e-310,
        ),
        // Very large coordinates.
        placed(
            case(
                "plate_translated",
                "far",
                plate_body(rect(300.3, 700.7), 10.1),
            ),
            [999_998_000.0, -999_999_000.0, 999_999_000.0],
            Turn::Identity,
        ),
        placed(
            case(
                "plate_half_turn",
                "far",
                plate_body(rect(300.3, 700.7), 10.1),
            ),
            [999_999_000.0, 999_999_000.0, -999_999_990.0],
            Turn::HalfZ,
        ),
        placed(
            case(
                "plate_at_negative_cap",
                "far",
                plate_body(rect(1.0, 2.0), 3.0),
            ),
            [-1e9, -1e9, -1e9],
            Turn::Identity,
        ),
        case(
            "plate_local_contour",
            "far",
            plate_body(shifted(&pentagon, near, near), 0.01),
        ),
        case(
            "plate_local_negative",
            "far",
            plate_body(shifted(&pentagon, -near, -near), 0.01),
        ),
        case(
            "member_local_axis",
            "far",
            member_body(rhs_far.clone(), far_start, far_end),
        ),
        case(
            "member_tiny_section",
            "far",
            member_body(Profile::Rectangle([0.5, 0.25]), far_start, far_end),
        ),
        placed(
            case(
                "member_translated",
                "far",
                member_body(
                    section("i_shape", &[300.3, 150.1, 7.1, 10.7, 0.0]),
                    origin,
                    flat,
                ),
            ),
            [999_990_000.0, -999_990_000.0, 999_990_000.0],
            Turn::HalfZ,
        ),
        // Cancellation-heavy inputs.
        case(
            "rhs_huge_thin",
            "cancel",
            member_body(
                section("rhs", &[999_999.9, 999_999.7, 0.4, 0.0]),
                origin,
                [1000.0, 0.0, 0.0],
            ),
        ),
        case(
            "rhs_mid_thin",
            "cancel",
            member_body(
                section("rhs", &[123_456.7, 98_765.3, 0.05, 0.0]),
                origin,
                [1000.0, 0.0, 0.0],
            ),
        ),
        case(
            "chs_huge_thin",
            "cancel",
            member_body(
                section("chs", &[999_999.9, 0.4]),
                origin,
                [1000.0, 0.0, 0.0],
            ),
        ),
        case("plate_sliver", "cancel", plate_body(sliver.clone(), 1.0)),
        case(
            "plate_sliver_far",
            "cancel",
            plate_body(shifted(&sliver, 99_999_000.0, -99_999_000.0), 1.0),
        ),
        case(
            "plate_thin_large",
            "cancel",
            plate_body(rect(1e6, 1e6), 0.34),
        ),
        case(
            "member_long_thin",
            "cancel",
            member_body(
                Profile::Rectangle([0.5, 0.5]),
                origin,
                [600_000.0, 800_000.0, 0.0],
            ),
        ),
        case(
            "member_near_vertical_column",
            "cancel",
            member_body(
                Profile::Rectangle([200.1, 100.3]),
                origin,
                [0.0009, 0.0, 1000.0],
            ),
        ),
        case(
            "member_near_vertical_beam",
            "cancel",
            member_body(
                Profile::Rectangle([200.1, 100.3]),
                origin,
                [0.0011, 0.0, 1000.0],
            ),
        ),
    ]);
    all
}

fn num(x: f64) -> String {
    format!("{x:e}")
}

fn nums(values: &[f64]) -> String {
    values.iter().map(|v| num(*v)).collect::<Vec<_>>().join(",")
}

fn parameters(body: &Body) -> (&'static str, String) {
    match body {
        Body::Plate { points, thickness } => (
            "plate",
            format!(
                "points={} thickness={}",
                points.iter().map(|p| nums(p)).collect::<Vec<_>>().join(";"),
                num(*thickness)
            ),
        ),
        Body::Member {
            profile,
            start,
            end,
        } => {
            let profile = match profile {
                Profile::Rectangle(wh) => format!("rectangle={}", nums(wh)),
                Profile::Section(family, values) => format!("{family}={}", nums(values)),
            };
            (
                "member",
                format!("{profile} start={} end={}", nums(start), nums(end)),
            )
        }
        Body::Fastener { part, bolt } => (
            "bolt",
            format!(
                "part={} diameter={} head={} nut={} washer={} length={}",
                part.token(),
                num(bolt.diameter),
                nums(&[bolt.head_af, bolt.head_h]),
                nums(&[bolt.nut_af, bolt.nut_h]),
                nums(&bolt.washer),
                num(bolt.length)
            ),
        ),
    }
}

/// A reference cell: the exact fraction and its 21-digit decimal, separated by a space.
fn cell(value: &Rational) -> String {
    format!("{} {}", value.to_fraction(), value.to_scientific(DIGITS))
}

fn opt(value: Option<&Rational>) -> String {
    value.map_or_else(|| "-".to_owned(), cell)
}

fn vector(values: Option<&[Rational; 3]>) -> String {
    values.map_or_else(
        || "-".to_owned(),
        |v| v.iter().map(cell).collect::<Vec<_>>().join(","),
    )
}

/// The three generated files as `(name, bytes)`, in [`FILES`] order.
pub fn generate() -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let all = cases();
    let mut ids = std::collections::BTreeSet::new();
    let mut cases_tsv = format!(
        "# beamcraft.g11-cases/{SET_VERSION}; original synthetic inputs, MPL-2.0, BeamCraft contributors\n\
         # Decimal tokens round to the nearest binary64. Units: mm, kg/m3. See README.md.\n\
         # id\tfamily\tbody\tparameters\tdensity_kg_m3\ttranslation_mm\trotation\n"
    );
    let mut expectations_tsv = format!(
        "# beamcraft.g11-expectations/{SET_VERSION}; references from exact rational evaluation of the binary64 inputs\n\
         # Each value is `fraction decimal`; `bounded` rows use pi to 64 decimals or square roots within 2^-200.\n\
         # id\toutcome\treference\tcurved\tvolume_mm3\tmass_kg\tsection_area_mm2\taxis_length_mm\tbounds_min_mm\tbounds_max_mm\tcentroid_mm\n"
    );
    for c in &all {
        if !ids.insert(c.id.clone()) {
            return Err(format!("duplicate id {}", c.id));
        }
        let (body, params) = parameters(&c.body);
        let rotation = match c.turn {
            Turn::Identity => "identity",
            Turn::HalfZ => "half_turn_z",
        };
        cases_tsv.push_str(&format!(
            "{}\t{}\t{body}\t{params}\t{}\t{}\t{rotation}\n",
            c.id,
            c.family,
            c.density.map_or_else(|| "-".to_owned(), num),
            nums(&c.translation),
        ));
        let e = expectation(c)?;
        let (lo, hi) = match &e.bounds {
            Some([lo, hi]) => (Some(lo), Some(hi)),
            None => (None, None),
        };
        expectations_tsv.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            c.id,
            e.outcome,
            if e.exact { "exact" } else { "bounded" },
            if e.curved { "curved" } else { "polygon" },
            cell(&e.volume),
            opt(e.mass.as_ref()),
            opt(e.area.as_ref()),
            opt(e.axis.as_ref()),
            vector(lo),
            vector(hi),
            vector(e.centroid.as_ref()),
        ));
    }
    let cases_bytes = cases_tsv.into_bytes();
    let expectations_bytes = expectations_tsv.into_bytes();
    let manifest = format!(
        "# beamcraft.g11-manifest/{SET_VERSION}\n# path\tbytes\tsha256\n\
         cases.tsv\t{}\t{}\nexpectations.tsv\t{}\t{}\n",
        cases_bytes.len(),
        hex_digest(&cases_bytes),
        expectations_bytes.len(),
        hex_digest(&expectations_bytes),
    );
    Ok(vec![
        ("cases.tsv", cases_bytes),
        ("expectations.tsv", expectations_bytes),
        ("manifest.tsv", manifest.into_bytes()),
    ])
}

/// Compares a directory with the generator byte for byte; returns the mismatching file names.
pub fn verify(dir: &std::path::Path) -> Result<Vec<String>, String> {
    let mut problems = Vec::new();
    for (name, bytes) in generate()? {
        match std::fs::read(dir.join(name)) {
            Ok(found) if found == bytes => {}
            Ok(_) => problems.push(format!("{name}: differs from the generator")),
            Err(e) => problems.push(format!("{name}: {e}")),
        }
    }
    Ok(problems)
}

/// Writes the generated files into `dir`, creating it when needed.
pub fn write(dir: &std::path::Path) -> Result<usize, String> {
    let files = generate()?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, bytes) in &files {
        std::fs::write(dir.join(name), bytes).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(files.len())
}
