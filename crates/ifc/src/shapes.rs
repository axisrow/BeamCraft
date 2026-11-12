// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A bounded reader of the *placed triangle bodies* of a received IFC STEP file: the readable
//! half of the MVP IFC input workflows (X05 reference conversion, native steel import), and
//! the answer to "what geometry does this file hold".
//!
//! From the file alone it reads every product whose representation is an
//! `IfcProductDefinitionShape` over `IfcTriangulatedFaceSet` tessellations, resolves each
//! product's `IfcLocalPlacement` chain, honours the declared length unit, and returns the
//! placed bodies in millimetres. Everything it cannot read is an explicit loss, never a
//! guess and never a crash. It builds no elements, writes nothing and converts nothing into
//! BeamCraft's own model: the supported schema subset and the entity families a converter
//! admits remain owner decisions (E04, upstream Apibim-com/BeamCraft#527).
//!
//! Lexically it accepts what [`crate::read`] accepts (CRLF, gaps in instance numbers, blank
//! lines) and refuses the same framing breaks. One instance per DATA line: a real exporter
//! keeps instances on one line, and a line whose arguments cannot be split is a loss. The
//! placement frames follow `IfcAxis2Placement3D` (ISO 10303-21 and buildingSMART IFC4):
//! Z from `Axis` (default 0,0,1), X from `RefDirection` orthogonalized against Z (default
//! 1,0,0), Y = Z x X; a degenerate frame is a loss.

use crate::guid;
use crate::read::{self, Loss, ReadError, UnitsReading, instance_head, raw_string, strip_dots};
use beamcraft_model::digest::ContentDigest;
use std::collections::{BTreeMap, BTreeSet};

/// Largest number of instances indexed for reference resolution. A file under the input cap
/// cannot hold many more; beyond it the rest is not indexed and shapes are not read,
/// instead of growing the index without bound.
pub const MAX_INDEXED_INSTANCES: usize = 1_000_000;

/// Largest total number of vertices kept over all bodies. A hostile file is stopped instead
/// of allocating past the budget; the bodies read before the stop are kept.
pub const MAX_TOTAL_VERTICES: usize = 1_000_000;

/// Largest total number of triangles kept over all bodies.
pub const MAX_TOTAL_TRIANGLES: usize = 2_000_000;

/// Largest number of bodies kept in the report. Further bodies are counted in the totals
/// but not listed.
pub const MAX_SHAPES_LISTED: usize = 4096;

/// Largest number of losses listed; further losses collapse into the truncation flag.
pub const MAX_LOSSES: usize = 64;

/// Longest product name kept as a fact; a longer one is omitted as a loss.
const MAX_NAME_BYTES: usize = 256;

/// Deepest `IfcLocalPlacement` chain resolved; deeper chains are a loss.
const MAX_PLACEMENT_DEPTH: usize = 64;

/// Why one body or placement was refused; the caller records it as a loss.
struct Fail {
    code: &'static str,
    detail: String,
}

fn fail(code: &'static str, detail: String) -> Fail {
    Fail { code, detail }
}

/// A placed triangle body read from the file, in millimetres.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeBody {
    /// The instance number `#n` of the product.
    pub instance: u64,
    /// The STEP entity name, for example `IFCPLATE`.
    pub entity: String,
    /// The product's valid 22-character `IfcGloballyUniqueId`, when its first attribute
    /// decodes as one.
    pub global_id: Option<String>,
    /// The product's `Name` label, when readable and within 256 bytes.
    pub name: Option<String>,
    /// Whether the face set declares itself closed (`.T.`).
    pub closed: bool,
    /// Placed vertex positions in millimetres, in the face set's stored order.
    pub vertices_mm: Vec<[f32; 3]>,
    /// Triangles as zero-based vertex indices, in the face set's stored order.
    pub triangles: Vec<[u32; 3]>,
    /// Component-wise bounds of `vertices_mm`.
    pub bounds_mm: Bounds,
}

/// Component-wise bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// The smallest coordinate per axis.
    pub min: [f32; 3],
    /// The largest coordinate per axis.
    pub max: [f32; 3],
}

/// What one scan read from the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Shapes {
    /// SHA-256 of the exact bytes scanned.
    pub digest: ContentDigest,
    /// Size of the scanned bytes.
    pub bytes: u64,
    /// The declared `FILE_SCHEMA` token, upper case, for example `IFC4`.
    pub schema_token: Option<String>,
    /// Whether the declared schema is the IFC4 subset this crate writes and studies. A
    /// file outside it still reads: the reader is name-driven, and the schema fact stays
    /// a loss of the study scan.
    pub schema_recognized: bool,
    /// The declared project length unit in metres per file unit, as the study scan
    /// resolves it. Without it no body is read: the millimetre contract cannot be honoured.
    pub length_unit: Option<UnitsReading>,
    /// Instances whose seventh attribute resolves to an `IfcProductDefinitionShape`.
    pub product_count: u64,
    /// Bodies read over all products.
    pub shape_count: u64,
    /// Total vertices over the read bodies, in millimetres.
    pub vertex_total: u64,
    /// Total triangles over the read bodies.
    pub triangle_total: u64,
    /// The read bodies, in file order, capped at [`MAX_SHAPES_LISTED`].
    pub shapes: Vec<ShapeBody>,
    /// True when bodies beyond the cap were read but not listed.
    pub shapes_truncated: bool,
    /// The losses, capped at [`MAX_LOSSES`].
    pub losses: Vec<Loss>,
    /// True when losses beyond the cap were dropped.
    pub losses_truncated: bool,
}

/// One rigid placement frame: the columns `x`, `y`, `z` and the translation `t`, so
/// `world = X*vx + Y*vy + Z*vz + t` in file units.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame {
    x: [f64; 3],
    y: [f64; 3],
    z: [f64; 3],
    t: [f64; 3],
}

impl Frame {
    fn identity() -> Self {
        Self {
            x: [1.0, 0.0, 0.0],
            y: [0.0, 1.0, 0.0],
            z: [0.0, 0.0, 1.0],
            t: [0.0; 3],
        }
    }

    /// Maps a local point into this frame's coordinates.
    fn apply(&self, v: [f64; 3]) -> [f64; 3] {
        [
            self.x[0] * v[0] + self.y[0] * v[1] + self.z[0] * v[2] + self.t[0],
            self.x[1] * v[0] + self.y[1] * v[1] + self.z[1] * v[2] + self.t[1],
            self.x[2] * v[0] + self.y[2] * v[1] + self.z[2] * v[2] + self.t[2],
        ]
    }

    /// The frame of a space placed inside this one: a point in `inner`'s local space lands
    /// in this frame's coordinates. `self` is the outer frame. The columns are directions,
    /// so the outer translation never reaches them.
    fn compose(&self, inner: &Frame) -> Frame {
        let linear = |v: [f64; 3]| {
            [
                self.x[0] * v[0] + self.y[0] * v[1] + self.z[0] * v[2],
                self.x[1] * v[0] + self.y[1] * v[1] + self.z[1] * v[2],
                self.x[2] * v[0] + self.y[2] * v[1] + self.z[2] * v[2],
            ]
        };
        Frame {
            x: linear(inner.x),
            y: linear(inner.y),
            z: linear(inner.z),
            t: self.apply(inner.t),
        }
    }
}

fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if !len.is_finite() || len <= 0.0 {
        return None;
    }
    Some([v[0] / len, v[1] / len, v[2] / len])
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The `IfcAxis2Placement3D` frame of `location`, `axis` and `ref_direction` (each `None`
/// for `$`). `None` when a direction is degenerate: the frame is then not defined.
fn axis_frame(
    location: [f64; 3],
    axis: Option<[f64; 3]>,
    ref_direction: Option<[f64; 3]>,
) -> Option<Frame> {
    let z = normalize(axis.unwrap_or([0.0, 0.0, 1.0]))?;
    let x0 = normalize(ref_direction.unwrap_or([1.0, 0.0, 0.0]))?;
    // X orthogonalized against Z; a RefDirection parallel to Z leaves X undefined.
    let x = normalize([
        x0[0] - dot(x0, z) * z[0],
        x0[1] - dot(x0, z) * z[1],
        x0[2] - dot(x0, z) * z[2],
    ])?;
    let y = cross(z, x);
    if !y.iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(Frame {
        x,
        y,
        z,
        t: location,
    })
}

/// A STEP REAL: decimal forms like `1.`, `-2.5` and `1.E-5` (a mantissa may end on the
/// dot). `None` for anything else, including a non-finite value.
fn parse_real(raw: &str) -> Option<f64> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    let mut normalized = String::with_capacity(text.len() + 1);
    let mut previous = '\0';
    for c in text.chars() {
        if (c == 'e' || c == 'E') && previous == '.' {
            normalized.push('0');
        }
        normalized.push(c);
        previous = c;
    }
    normalized.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The exactly three reals of one inner list like `(1.,2.,3.)`.
fn parse_point(raw: &str) -> Option<[f64; 3]> {
    let inner = raw.trim().strip_prefix('(')?.strip_suffix(')')?;
    let mut parts = inner.split(',');
    let x = parse_real(parts.next()?)?;
    let y = parse_real(parts.next()?)?;
    let z = parse_real(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some([x, y, z])
}

/// Why a list scan stopped.
enum ScanStop {
    /// A token the grammar does not admit; the list is unreadable.
    Malformed,
    /// The caller's budget is exhausted; the whole scan stops.
    Budget,
}

/// Reads one token up to the next `,` or `)` and steps past a following comma.
fn token_at<'a>(bytes: &[u8], at: &mut usize, text: &'a str) -> Option<&'a str> {
    let start = *at;
    while !matches!(bytes.get(*at), None | Some(b',') | Some(b')')) {
        *at += 1;
    }
    let token = text.get(start..*at)?;
    if bytes.get(*at) == Some(&b',') {
        *at += 1;
    }
    Some(token)
}

/// Shifts `value` into `point` as its next slot: an unfinished tuple is left-shifted so
/// the first three values win.
fn shift_point(point: [f64; 3], value: f64) -> [f64; 3] {
    [point[1], point[2], value]
}

/// Runs `sink` over every `(a, b, c)` triple of a STEP nested coordinate list, without
/// materializing the list. The sink returns `false` to stop with [`ScanStop::Budget`].
fn for_each_point(list: &str, sink: &mut dyn FnMut([f64; 3]) -> bool) -> Result<(), ScanStop> {
    let inner = list
        .trim()
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or(ScanStop::Malformed)?;
    let bytes = inner.as_bytes();
    let mut i = 0usize;
    loop {
        while matches!(bytes.get(i), Some(b' ') | Some(b'\t') | Some(b',')) {
            i += 1;
        }
        if bytes.get(i).is_none() {
            return Ok(());
        }
        if bytes.get(i) != Some(&b'(') {
            return Err(ScanStop::Malformed);
        }
        i += 1;
        let mut point = [0.0; 3];
        for _ in 0..3 {
            let Some(token) = token_at(bytes, &mut i, inner) else {
                return Err(ScanStop::Malformed);
            };
            let Some(value) = parse_real(token) else {
                return Err(ScanStop::Malformed);
            };
            point = shift_point(point, value);
        }
        if bytes.get(i) != Some(&b')') {
            return Err(ScanStop::Malformed);
        }
        i += 1;
        if !sink(point) {
            return Err(ScanStop::Budget);
        }
    }
}

/// Runs `sink` over every `(i, j, k)` triangle of a STEP nested index list. The sink
/// returns `false` to stop with [`ScanStop::Budget`].
fn for_each_triangle(list: &str, sink: &mut dyn FnMut([u32; 3]) -> bool) -> Result<(), ScanStop> {
    let inner = list
        .trim()
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or(ScanStop::Malformed)?;
    let bytes = inner.as_bytes();
    let mut i = 0usize;
    loop {
        while matches!(bytes.get(i), Some(b' ') | Some(b'\t') | Some(b',')) {
            i += 1;
        }
        if bytes.get(i).is_none() {
            return Ok(());
        }
        if bytes.get(i) != Some(&b'(') {
            return Err(ScanStop::Malformed);
        }
        i += 1;
        let mut triangle = [0u32; 3];
        for _ in 0..3 {
            let Some(token) = token_at(bytes, &mut i, inner) else {
                return Err(ScanStop::Malformed);
            };
            let Ok(value) = token.trim().parse::<u32>() else {
                return Err(ScanStop::Malformed);
            };
            triangle = shift_triangle(triangle, value);
        }
        if bytes.get(i) != Some(&b')') {
            return Err(ScanStop::Malformed);
        }
        i += 1;
        if !sink(triangle) {
            return Err(ScanStop::Budget);
        }
    }
}

/// Shifts `value` into `triangle` as its next slot.
fn shift_triangle(triangle: [u32; 3], value: u32) -> [u32; 3] {
    [triangle[1], triangle[2], value]
}

/// The byte spans of the top-level arguments of an instance body, or `None` when the body
/// does not split (a multi-line fragment, an unbalanced string). Nothing is copied.
fn arg_spans(body: &str) -> Option<Vec<(usize, usize)>> {
    let bytes = body.as_bytes();
    let mut spans = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        if in_string {
            if b == b'\'' {
                if bytes.get(i.wrapping_add(1)) == Some(&b'\'') {
                    i += 1;
                } else {
                    in_string = false;
                }
            }
        } else {
            match b {
                b'\'' => in_string = true,
                b'(' => depth = depth.checked_add(1)?,
                b')' => depth = depth.checked_sub(1)?,
                b',' if depth == 0 => {
                    spans.push((start, i));
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    if in_string || depth != 0 {
        return None;
    }
    spans.push((start, bytes.len()));
    Some(spans)
}

/// The text of one argument span within its instance body.
fn span_text<'a>(body: &'a str, spans: &[(usize, usize)], at: usize) -> &'a str {
    spans
        .get(at)
        .and_then(|(start, end)| body.get(*start..*end))
        .unwrap_or_default()
}

/// The `#n` of one reference argument, or `None` for `$` and anything else.
fn ref_of(text: &str) -> Option<u64> {
    let digits = text.trim().strip_prefix('#')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The `#n` list of one reference-list argument like `(#7,#9)`.
fn ref_list(text: &str) -> Option<Vec<u64>> {
    let inner = text
        .trim()
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))?;
    let mut out = Vec::new();
    for item in inner.split(',') {
        out.push(ref_of(item)?);
    }
    Some(out)
}

/// The valid 22-character GlobalId of a product's first attribute, when it decodes.
fn first_global_id(attributes: &str) -> Option<String> {
    let text = read::first_string(attributes)?;
    guid::decode(text).ok()?;
    Some(text.to_owned())
}

/// The mutable state of one shape scan.
struct Scan<'a> {
    text: &'a str,
    instances: Vec<(u64, String, &'a str)>,
    index: BTreeMap<u64, usize>,
    /// Reference resolution is disabled: instance numbers repeat or several DATA sections
    /// exist, so a `#n` is ambiguous.
    unique: bool,
    frames: BTreeMap<u64, Result<Frame, &'static str>>,
    out: Shapes,
    /// The vertex or triangle budget is exhausted; everything after is skipped.
    stopped: bool,
    /// The codes whose first occurrence only is listed (the per-product repeats carry no
    /// new information).
    sampled: BTreeSet<&'static str>,
}

/// The raw, still-unplaced body of one `IfcTriangulatedFaceSet`.
struct RawBody {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    closed: bool,
}

/// Reads the placed triangle bodies of one received IFC file. Linear in the input size;
/// every allocation is bounded by the module constants. The framing, digest, schema and
/// unit facts are the study scan's ([`read::inspect`]), so both commands answer identically
/// about the same bytes.
pub fn read(bytes: &[u8]) -> Result<Shapes, ReadError> {
    let study = read::inspect(bytes)?;
    // The study scan accepted the bytes as UTF-8 text, so this conversion cannot fail;
    // the unreachable empty fallback would only skip the (already refused) scan.
    let text = core::str::from_utf8(bytes).unwrap_or("");
    let mut scan = Scan {
        text,
        instances: Vec::new(),
        index: BTreeMap::new(),
        unique: true,
        frames: BTreeMap::new(),
        out: Shapes {
            digest: study.digest,
            bytes: study.bytes,
            schema_token: study.schema_token.clone(),
            schema_recognized: study.schema_recognized,
            length_unit: study.length_unit,
            product_count: 0,
            shape_count: 0,
            vertex_total: 0,
            triangle_total: 0,
            shapes: Vec::new(),
            shapes_truncated: false,
            losses: Vec::new(),
            losses_truncated: false,
        },
        stopped: false,
        sampled: BTreeSet::new(),
    };
    scan.index_instances();
    scan.resolve();
    Ok(scan.out)
}

impl<'a> Scan<'a> {
    fn loss(&mut self, code: &'static str, detail: String) {
        if self.out.losses.len() < MAX_LOSSES {
            self.out.losses.push(Loss { code, detail });
        } else {
            self.out.losses_truncated = true;
        }
    }

    /// A loss whose repeats carry no new information: only its first occurrence is listed,
    /// so one homogeneous file cannot flood the list.
    fn sampled_loss(&mut self, code: &'static str, detail: String) {
        if self.sampled.insert(code) {
            self.loss(code, detail);
        }
    }

    /// Walks the DATA sections and indexes every instance head. The framing was already
    /// accepted by the study scan, so this walk cannot run off the end of the file.
    fn index_instances(&mut self) {
        let mut sections = 0usize;
        let mut in_data = false;
        for raw in self.text.split('\n') {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            match (in_data, line) {
                (_, "DATA;") => {
                    in_data = true;
                    sections += 1;
                    if sections > 1 {
                        self.unique = false;
                        self.loss(
                            "MULTIPLE_DATA_SECTIONS",
                            "instance numbers may repeat across sections; shapes are not \
                             read"
                                .to_owned(),
                        );
                    }
                }
                (true, "ENDSEC;") => in_data = false,
                (true, line) => {
                    if line.is_empty() || !line.starts_with('#') {
                        continue;
                    }
                    if let Some((id, name, attributes)) = instance_head(line) {
                        if self.instances.len() >= MAX_INDEXED_INSTANCES {
                            self.loss(
                                "INSTANCE_CAP",
                                format!(
                                    "more than {MAX_INDEXED_INSTANCES} instances exist; \
                                     further instances are not indexed"
                                ),
                            );
                            return;
                        }
                        if self.index.contains_key(&id) {
                            self.unique = false;
                            self.loss(
                                "REPEATED_INSTANCE_NUMBER",
                                format!("#{id} occurs more than once; shapes are not read"),
                            );
                        } else {
                            self.index.insert(id, self.instances.len());
                        }
                        self.instances.push((id, name, attributes));
                    }
                }
                (false, _) => {}
            }
        }
    }

    /// The (entity name, attribute text) of an indexed instance, copied out so no borrow
    /// of the scan crosses a mutation.
    fn lookup(&self, id: u64) -> Option<(String, &'a str)> {
        self.index
            .get(&id)
            .and_then(|at| self.instances.get(*at))
            .map(|(_, name, body)| (name.clone(), *body))
    }

    fn resolve(&mut self) {
        if !self.unique {
            self.loss(
                "REFERENCE_RESOLUTION_DISABLED",
                "instance numbers or DATA sections are not unique enough to resolve \
                 references; shapes are not read"
                    .to_owned(),
            );
            return;
        }
        if self.out.length_unit.is_none() {
            self.loss(
                "GEOMETRY_WITHOUT_LENGTH_UNIT",
                "the file declares no SI length unit, so no body can be placed in \
                 millimetres; no shape is read"
                    .to_owned(),
            );
        }
        for pos in 0..self.instances.len() {
            if self.stopped {
                break;
            }
            let Some((id, _, body)) = self.instances.get(pos) else {
                continue;
            };
            let (id, body) = (*id, *body);
            let Some(spans) = arg_spans(body) else {
                self.loss(
                    "INSTANCE_ARGS_UNREADABLE",
                    format!(
                        "the arguments of #{id} do not split into arguments, so the \
                         instance is not read"
                    ),
                );
                continue;
            };
            // An IfcProduct keeps GlobalId..Representation in attributes 1..7: its
            // Representation attribute is the seventh. Only an instance whose seventh
            // attribute resolves to an IfcProductDefinitionShape counts as a product, so
            // nothing is guessed from a name table.
            let Some(shape_ref) = ref_of(span_text(body, &spans, 6).trim()) else {
                continue;
            };
            let Some((shape_name, shape_body)) = self.lookup(shape_ref) else {
                continue;
            };
            if shape_name != "IFCPRODUCTDEFINITIONSHAPE" {
                continue;
            }
            self.out.product_count += 1;
            if self.out.length_unit.is_none() {
                continue;
            }
            let Some((entity, _)) = self.lookup(id) else {
                continue;
            };
            let global_id = first_global_id(span_text(body, &spans, 0).trim());
            let raw_name = raw_string(span_text(body, &spans, 2).trim());
            let name = raw_name.clone().filter(|text| text.len() <= MAX_NAME_BYTES);
            if raw_name.is_some() && name.is_none() {
                self.loss(
                    "PRODUCT_NAME_OMITTED",
                    format!("the Name of #{id} is longer than {MAX_NAME_BYTES} bytes"),
                );
            }
            let placement = match span_text(body, &spans, 5).trim() {
                "$" => Frame::identity(),
                text => match ref_of(text).map(|at| self.frame_of(at, &mut Vec::new())) {
                    Some(Ok(frame)) => frame,
                    Some(Err(why)) => {
                        self.loss("PLACEMENT_UNRESOLVED", format!("#{id}: {why}"));
                        continue;
                    }
                    None => {
                        self.loss(
                            "PLACEMENT_UNREADABLE",
                            format!(
                                "the ObjectPlacement of #{id} is neither $ nor one instance \
                                 reference"
                            ),
                        );
                        continue;
                    }
                },
            };
            self.read_shape(id, &entity, global_id, name, &placement, shape_body);
        }
    }

    /// Reads every tessellation of one product's shape, applies `placement` and the unit
    /// scale, and keeps what survives the budgets.
    #[allow(clippy::too_many_arguments)]
    fn read_shape(
        &mut self,
        id: u64,
        entity: &str,
        global_id: Option<String>,
        name: Option<String>,
        placement: &Frame,
        shape_body: &'a str,
    ) {
        let Some(spans) = arg_spans(shape_body) else {
            self.loss(
                "INSTANCE_ARGS_UNREADABLE",
                format!("the arguments of #{id} do not split into arguments"),
            );
            return;
        };
        // IfcProductDefinitionShape(Name, Description, Representations).
        let Some(reps) = ref_list(span_text(shape_body, &spans, 2).trim()) else {
            self.loss(
                "SHAPE_REFERENCE_UNREADABLE",
                format!(
                    "the Representations attribute of #{id} is not a list of instance \
                     references"
                ),
            );
            return;
        };
        let mut bodies: Vec<ShapeBody> = Vec::new();
        for at in reps {
            if self.stopped {
                break;
            }
            let Some((rep_name, rep_body)) = self.lookup(at) else {
                self.loss(
                    "SHAPE_REFERENCE_DANGLING",
                    format!("#{at} in the Representations of #{id} refers to a missing instance"),
                );
                continue;
            };
            if rep_name != "IFCSHAPEREPRESENTATION" {
                self.loss(
                    "SHAPE_REFERENCE_NOT_REPRESENTATION",
                    format!("#{at} in the Representations of #{id} is a {rep_name}"),
                );
                continue;
            }
            let Some(rep_spans) = arg_spans(rep_body) else {
                self.loss(
                    "INSTANCE_ARGS_UNREADABLE",
                    format!("the arguments of #{at} do not split into arguments"),
                );
                continue;
            };
            // IfcShapeRepresentation(Context, RepresentationIdentifier,
            // RepresentationType, Items).
            let rep_type =
                raw_string(span_text(rep_body, &rep_spans, 2).trim()).unwrap_or_default();
            if rep_type != "Tessellation" {
                self.sampled_loss(
                    "REPRESENTATION_NOT_TESSELLATED",
                    format!(
                        "the representation type {rep_type:?} is not read here; only \
                         'Tessellation' face sets are"
                    ),
                );
                continue;
            }
            let Some(items) = ref_list(span_text(rep_body, &rep_spans, 3).trim()) else {
                self.loss(
                    "SHAPE_REFERENCE_UNREADABLE",
                    format!("the Items attribute of #{at} is not a list of instance references"),
                );
                continue;
            };
            for item in items {
                if self.stopped {
                    break;
                }
                let Some((item_name, item_body)) = self.lookup(item) else {
                    self.loss(
                        "SHAPE_REFERENCE_DANGLING",
                        format!("#{item} in the Items of #{at} refers to a missing instance"),
                    );
                    continue;
                };
                if item_name != "IFCTRIANGULATEDFACESET" {
                    self.sampled_loss(
                        "SHAPE_ITEM_NOT_FACE_SET",
                        format!(
                            "an {item_name} item in a tessellation representation is not \
                             read; only IfcTriangulatedFaceSet items are"
                        ),
                    );
                    continue;
                }
                match self.read_face_set(item_body) {
                    Ok(Some(raw)) => {
                        if let Some(body) = self.place_body(
                            id,
                            entity,
                            global_id.clone(),
                            name.clone(),
                            placement,
                            raw,
                        ) {
                            bodies.push(body);
                        }
                    }
                    Ok(None) => {}
                    Err(why) => self.loss(why.code, why.detail),
                }
            }
        }
        if bodies.is_empty() {
            self.sampled_loss(
                "SHAPE_NOT_TESSELLATED",
                "a resolved product carries no readable tessellated body".to_owned(),
            );
            return;
        }
        for body in bodies {
            self.out.vertex_total += body.vertices_mm.len() as u64;
            self.out.triangle_total += body.triangles.len() as u64;
            if self.out.shapes.len() < MAX_SHAPES_LISTED {
                self.out.shapes.push(body);
            } else {
                self.out.shapes_truncated = true;
            }
            self.out.shape_count += 1;
        }
    }

    /// Reads one `IfcTriangulatedFaceSet(Coordinates, Normals, Closed, Triangles, Unit)`
    /// body in file units. `Ok(None)` means the body was refused and the loss is already
    /// recorded.
    fn read_face_set(&mut self, body: &'a str) -> Result<Option<RawBody>, Fail> {
        let Some(spans) = arg_spans(body) else {
            return Err(fail(
                "INSTANCE_ARGS_UNREADABLE",
                "the arguments of a face set do not split into arguments".to_owned(),
            ));
        };
        // IfcTriangulatedFaceSet(Coordinates, Normals, Closed, Triangles, Unit).
        let closed = matches!(strip_dots(span_text(body, &spans, 2).trim()), "T");
        let Some(coordinates) = ref_of(span_text(body, &spans, 0).trim()) else {
            return Err(fail(
                "SHAPE_COORDINATES_DANGLING",
                "the Coordinates attribute of a face set is not one instance reference".to_owned(),
            ));
        };
        let Some((list_name, list_body)) = self.lookup(coordinates) else {
            return Err(fail(
                "SHAPE_COORDINATES_DANGLING",
                format!("#{coordinates} in a face set refers to a missing instance"),
            ));
        };
        if list_name != "IFCCARTESIANPOINTLIST3D" {
            return Err(fail(
                "SHAPE_COORDINATES_DANGLING",
                format!("#{coordinates} is a {list_name}, not an IfcCartesianPointList3D"),
            ));
        }
        let Some(list_spans) = arg_spans(list_body) else {
            return Err(fail(
                "INSTANCE_ARGS_UNREADABLE",
                format!("the arguments of #{coordinates} do not split into arguments"),
            ));
        };
        let points_text = span_text(list_body, &list_spans, 0);
        let triangles_text = span_text(body, &spans, 3).trim().to_owned();
        let mut vertices: Vec<[f64; 3]> = Vec::new();
        {
            let budget = self.out.vertex_total;
            let mut sink = |point: [f64; 3]| {
                if budget.saturating_add(vertices.len() as u64) + 1 > MAX_TOTAL_VERTICES as u64 {
                    return false;
                }
                vertices.push(point);
                true
            };
            if let Err(stop) = for_each_point(points_text, &mut sink) {
                return match stop {
                    ScanStop::Malformed => Err(fail(
                        "COORDINATE_LIST_UNREADABLE",
                        format!(
                            "the coordinate list #{coordinates} is not a list of three-real \
                             points"
                        ),
                    )),
                    ScanStop::Budget => {
                        self.stopped = true;
                        Err(fail(
                            "SHAPE_BUDGET_EXCEEDED",
                            format!(
                                "the vertex budget of {MAX_TOTAL_VERTICES} is exhausted; \
                                 the remaining bodies are not read"
                            ),
                        ))
                    }
                };
            }
        }
        let point_count = vertices.len();
        let mut triangles: Vec<[u32; 3]> = Vec::new();
        {
            let budget = self.out.triangle_total;
            let mut bad: Option<u32> = None;
            let mut sink = |triangle: [u32; 3]| {
                if budget.saturating_add(triangles.len() as u64) + 1 > MAX_TOTAL_TRIANGLES as u64 {
                    return false;
                }
                // IFC indices are one-based; zero-based here. An index the vertex list
                // does not hold refuses the whole body, never a crash.
                if triangle
                    .iter()
                    .any(|i| *i == 0 || u64::from(*i) > point_count as u64)
                {
                    bad = Some(triangle.iter().copied().fold(0u32, u32::max));
                    return false;
                }
                triangles.push([triangle[0] - 1, triangle[1] - 1, triangle[2] - 1]);
                true
            };
            if let Err(stop) = for_each_triangle(&triangles_text, &mut sink) {
                return match stop {
                    ScanStop::Malformed => Err(fail(
                        "TRIANGLE_LIST_UNREADABLE",
                        "the triangle list of a face set is not a list of three-index \
                         triangles"
                            .to_owned(),
                    )),
                    // The sink stops either on the budget or on a bad index; the captured
                    // flag says which.
                    ScanStop::Budget => {
                        if let Some(index) = bad {
                            return Err(fail(
                                "TRIANGLE_INDEX_OUT_OF_RANGE",
                                format!(
                                    "a triangle of a face set names index {index}, which \
                                     the vertex list of {point_count} points does not hold"
                                ),
                            ));
                        }
                        self.stopped = true;
                        Err(fail(
                            "SHAPE_BUDGET_EXCEEDED",
                            format!(
                                "the triangle budget of {MAX_TOTAL_TRIANGLES} is exhausted; \
                                 the remaining bodies are not read"
                            ),
                        ))
                    }
                };
            }
        }
        if triangles.is_empty() {
            return Err(fail(
                "TRIANGLE_LIST_UNREADABLE",
                "a face set holds no triangle".to_owned(),
            ));
        }
        Ok(Some(RawBody {
            vertices,
            triangles,
            closed,
        }))
    }

    /// Applies the placement and the unit scale to a raw body and keeps it in millimetres.
    #[allow(clippy::too_many_arguments)]
    fn place_body(
        &mut self,
        id: u64,
        entity: &str,
        global_id: Option<String>,
        name: Option<String>,
        placement: &Frame,
        raw: RawBody,
    ) -> Option<ShapeBody> {
        let unit = self.out.length_unit?;
        let scale = unit.meters_per_unit * 1000.0;
        let mut vertices = Vec::with_capacity(raw.vertices.len());
        let mut bounds = Bounds {
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
        };
        let clean = |v: f64| (v * scale) as f32 + 0.0;
        for point in &raw.vertices {
            let world = placement.apply(*point);
            let mm = [clean(world[0]), clean(world[1]), clean(world[2])];
            if !mm.iter().all(|v| v.is_finite()) {
                self.loss(
                    "COORDINATE_OVERFLOW",
                    format!(
                        "a vertex of #{id} leaves the finite millimetre range after the \
                         placement and the unit scale"
                    ),
                );
                return None;
            }
            bounds.min[0] = bounds.min[0].min(mm[0]);
            bounds.min[1] = bounds.min[1].min(mm[1]);
            bounds.min[2] = bounds.min[2].min(mm[2]);
            bounds.max[0] = bounds.max[0].max(mm[0]);
            bounds.max[1] = bounds.max[1].max(mm[1]);
            bounds.max[2] = bounds.max[2].max(mm[2]);
            vertices.push(mm);
        }
        Some(ShapeBody {
            instance: id,
            entity: entity.to_owned(),
            global_id,
            name,
            closed: raw.closed,
            vertices_mm: vertices,
            triangles: raw.triangles,
            bounds_mm: bounds,
        })
    }

    /// The composed world frame of the `IfcLocalPlacement` at `#at`, memoized.
    fn frame_of(&mut self, at: u64, stack: &mut Vec<u64>) -> Result<Frame, &'static str> {
        if let Some(cached) = self.frames.get(&at) {
            return *cached;
        }
        let computed = if stack.contains(&at) {
            Err("the placement chain loops")
        } else if stack.len() >= MAX_PLACEMENT_DEPTH {
            Err("the placement chain is deeper than 64 steps")
        } else {
            stack.push(at);
            let computed = self.compute_frame(at, stack);
            stack.pop();
            computed
        };
        self.frames.insert(at, computed);
        computed
    }

    /// The frame of one `IfcLocalPlacement(PlacementRelTo, RelativePlacement)`.
    fn compute_frame(&mut self, at: u64, stack: &mut Vec<u64>) -> Result<Frame, &'static str> {
        let Some((name, body)) = self.lookup(at) else {
            return Err("the placement refers to a missing instance");
        };
        if name != "IFCLOCALPLACEMENT" {
            return Err("the placement is not an IfcLocalPlacement");
        }
        let Some(spans) = arg_spans(body) else {
            return Err("the arguments of the placement do not split");
        };
        let local = match ref_of(span_text(body, &spans, 1).trim()) {
            Some(axes) => self.axes_frame(axes)?,
            None => return Err("the relative placement is not one instance reference"),
        };
        match span_text(body, &spans, 0).trim() {
            "$" => Ok(local),
            text => match ref_of(text) {
                Some(rel) => Ok(self.frame_of(rel, stack)?.compose(&local)),
                None => Err("the PlacementRelTo is neither $ nor one instance reference"),
            },
        }
    }

    /// The frame of one `IfcAxis2Placement3D(Location, Axis, RefDirection)`.
    fn axes_frame(&mut self, at: u64) -> Result<Frame, &'static str> {
        let Some((name, body)) = self.lookup(at) else {
            return Err("the relative placement refers to a missing instance");
        };
        if name != "IFCAXIS2PLACEMENT3D" {
            return Err("the relative placement is not an IfcAxis2Placement3D");
        }
        let Some(spans) = arg_spans(body) else {
            return Err("the arguments of the relative placement do not split");
        };
        let location = match ref_of(span_text(body, &spans, 0).trim()) {
            Some(point) => self.point_of(point)?,
            None => return Err("the location is not one instance reference"),
        };
        let direction = |scan: &mut Self, text: &str| -> Result<Option<[f64; 3]>, &'static str> {
            match text.trim() {
                "$" | "*" => Ok(None),
                text => match ref_of(text) {
                    Some(at) => Ok(Some(scan.direction_of(at)?)),
                    None => Err("a direction attribute is not one instance reference"),
                },
            }
        };
        let axis = direction(self, span_text(body, &spans, 1))?;
        let ref_direction = direction(self, span_text(body, &spans, 2))?;
        axis_frame(location, axis, ref_direction).ok_or("the axis frame is degenerate")
    }

    /// The three reals of an `IfcCartesianPoint` at `#at`.
    fn point_of(&self, at: u64) -> Result<[f64; 3], &'static str> {
        let Some((name, body)) = self.lookup(at) else {
            return Err("the location refers to a missing instance");
        };
        if name != "IFCCARTESIANPOINT" {
            return Err("the location is not an IfcCartesianPoint");
        }
        let Some(spans) = arg_spans(body) else {
            return Err("the arguments of the location do not split");
        };
        parse_point(span_text(body, &spans, 0))
            .filter(|p| p.iter().all(|v| v.is_finite()))
            .ok_or("the location is not three real numbers")
    }

    /// The two or three reals of an `IfcDirection` at `#at`; a missing third is 0.
    fn direction_of(&self, at: u64) -> Result<[f64; 3], &'static str> {
        let Some((name, body)) = self.lookup(at) else {
            return Err("a direction refers to a missing instance");
        };
        if name != "IFCDIRECTION" {
            return Err("a direction is not an IfcDirection");
        }
        let Some(spans) = arg_spans(body) else {
            return Err("the arguments of a direction do not split");
        };
        let raw = span_text(body, &spans, 0);
        let inner = raw
            .trim()
            .strip_prefix('(')
            .and_then(|rest| rest.strip_suffix(')'))
            .ok_or("a direction is not a list of real numbers")?;
        let mut parts = inner.split(',');
        let x = parse_real(parts.next().ok_or("a direction holds no real number")?)
            .ok_or("a direction is not a list of real numbers")?;
        let y = parse_real(
            parts
                .next()
                .ok_or("a direction holds fewer than two reals")?,
        )
        .ok_or("a direction is not a list of real numbers")?;
        let z = match parts.next() {
            None => 0.0,
            Some(text) => {
                if parts.next().is_some() {
                    return Err("a direction holds more than three real numbers");
                }
                parse_real(text).ok_or("a direction is not a list of real numbers")?
            }
        };
        Ok([x, y, z])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ElementData, Entity, ExportModel, Limits, Mesh, Pose};
    use beamcraft_model::id::Uuid7;

    fn read_str(text: &str) -> Shapes {
        read(text.as_bytes()).ok().unwrap()
    }

    /// A contributor-original synthetic IFC4 file: one project in millimetres, a site
    /// placement chain, one plate product and a two-triangle body.
    fn plate_ifc() -> String {
        [
            "ISO-10303-21;",
            "HEADER;",
            "FILE_DESCRIPTION(('synthetic shapes file'),'2;1');",
            "FILE_NAME('plate.ifc','2026-11-12T00:00:00',('study'),('apibim'),'none',\
             'beamcraft shape tests','');",
            "FILE_SCHEMA(('IFC4'));",
            "ENDSEC;",
            "DATA;",
            "#1=IFCPROJECT('0YvucVMOX9kw8jhtQ9abcd',*,$,$,$,$,$,(#2),#3);",
            "#2=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#4,$);",
            "#3=IFCUNITASSIGNMENT((#5));",
            "#4=IFCAXIS2PLACEMENT3D(#6,$,$);",
            "#5=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);",
            "#6=IFCCARTESIANPOINT((0.,0.,0.));",
            "#10=IFCSITE('2Bnr5VMOX0PO8jvHqZabcd',*,'Site',$,$,#11,$,$,.ELEMENT.,$,$,$,$,$);",
            "#11=IFCLOCALPLACEMENT($,#12);",
            "#12=IFCAXIS2PLACEMENT3D(#6,$,$);",
            "#20=IFCPLATE('1Anr5VMOX0PO8jvHqZabcd',*,'PL-A',$,$,#21,#22,$,.NOTDEFINED.);",
            "#21=IFCLOCALPLACEMENT(#11,#12);",
            "#22=IFCPRODUCTDEFINITIONSHAPE($,$,(#23));",
            "#23=IFCSHAPEREPRESENTATION(#2,'Body','Tessellation',(#24));",
            "#24=IFCTRIANGULATEDFACESET(#25,$,.T.,((1,2,3),(1,3,4)),$);",
            "#25=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(100.,0.,0.),(100.,80.,0.),(0.,80.,0.)));",
            "ENDSEC;",
            "END-ISO-10303-21;",
            "",
        ]
        .join("\n")
    }

    #[test]
    fn a_plate_with_an_identity_chain_reads_back_placed() {
        let file = plate_ifc();
        let out = read_str(&file);
        assert_eq!(out.schema_token.as_deref(), Some("IFC4"));
        assert_eq!(
            out.length_unit,
            Some(UnitsReading {
                meters_per_unit: 0.001,
                origin: "project_units",
            })
        );
        assert_eq!(out.product_count, 1);
        assert_eq!(out.shape_count, 1);
        assert_eq!(out.vertex_total, 4);
        assert_eq!(out.triangle_total, 2);
        assert!(out.losses.is_empty(), "{:?}", out.losses);
        let shape = out.shapes.first().unwrap();
        assert_eq!(shape.instance, 20);
        assert_eq!(shape.entity, "IFCPLATE");
        assert_eq!(shape.global_id.as_deref(), Some("1Anr5VMOX0PO8jvHqZabcd"));
        assert_eq!(shape.name.as_deref(), Some("PL-A"));
        assert!(shape.closed);
        assert_eq!(
            shape.vertices_mm.as_slice(),
            &[
                [0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [100.0, 80.0, 0.0],
                [0.0, 80.0, 0.0]
            ]
        );
        assert_eq!(shape.triangles, vec![[0, 1, 2], [0, 2, 3]]);
        assert_eq!(
            shape.bounds_mm,
            Bounds {
                min: [0.0, 0.0, 0.0],
                max: [100.0, 80.0, 0.0],
            }
        );
        // The same bytes answer with the same shapes.
        assert_eq!(out, read_str(&file));
    }

    /// The product's local placement now rotates 90 degrees about Z (axis Z, ref Y) and
    /// translates by (10, 20, 30).
    fn rotated_ifc() -> String {
        plate_ifc().replace(
            "#21=IFCLOCALPLACEMENT(#11,#12);",
            "#21=IFCLOCALPLACEMENT(#11,#27);\
             \n#27=IFCAXIS2PLACEMENT3D(#28,#29,#30);\
             \n#28=IFCCARTESIANPOINT((10.,20.,30.));\
             \n#29=IFCDIRECTION((0.,0.,1.));\
             \n#30=IFCDIRECTION((0.,1.,0.));",
        )
    }

    #[test]
    fn a_rotated_placement_places_the_vertices() {
        let out = read_str(&rotated_ifc());
        assert!(out.losses.is_empty(), "{:?}", out.losses);
        let shape = out.shapes.first().unwrap();
        // X = (0,1,0) orthogonalized, Y = Z x X = (-1,0,0): the local x axis points along
        // world y, plus the translation (10, 20, 30).
        assert_eq!(
            shape.vertices_mm.as_slice(),
            &[
                [10.0, 20.0, 30.0],
                [10.0, 120.0, 30.0],
                [-70.0, 120.0, 30.0],
                [-70.0, 20.0, 30.0]
            ]
        );
        assert_eq!(
            shape.bounds_mm,
            Bounds {
                min: [-70.0, 20.0, 30.0],
                max: [10.0, 120.0, 30.0],
            }
        );
    }

    #[test]
    fn a_spatial_chain_lifts_the_body() {
        // The site placement lifts to z = 1000; the plate rides on it.
        let file = plate_ifc()
            .replace(
                "#11=IFCLOCALPLACEMENT($,#12);",
                "#11=IFCLOCALPLACEMENT($,#13);",
            )
            .replace(
                "#12=IFCAXIS2PLACEMENT3D(#6,$,$);",
                "#12=IFCAXIS2PLACEMENT3D(#6,$,$);\n\
                 #13=IFCAXIS2PLACEMENT3D(#14,$,$);\n\
                 #14=IFCCARTESIANPOINT((0.,0.,1000.));",
            );
        let out = read_str(&file);
        assert!(out.losses.is_empty(), "{:?}", out.losses);
        let shape = out.shapes.first().unwrap();
        assert!(
            shape.vertices_mm.iter().all(|v| v[2] == 1000.0),
            "{:?}",
            shape.vertices_mm
        );
    }

    #[test]
    fn a_dangling_placement_skips_the_product_with_a_loss() {
        let file = plate_ifc().replace(
            "#21=IFCLOCALPLACEMENT(#11,#12);",
            "#21=IFCLOCALPLACEMENT(#11,#99);",
        );
        let out = read_str(&file);
        assert_eq!(out.product_count, 1);
        assert_eq!(out.shape_count, 0);
        assert!(
            out.losses.iter().any(|l| l.code == "PLACEMENT_UNRESOLVED"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn a_looping_placement_chain_is_a_loss_never_a_hang() {
        let file = plate_ifc().replace(
            "#11=IFCLOCALPLACEMENT($,#12);",
            "#11=IFCLOCALPLACEMENT(#21,#12);",
        );
        let out = read_str(&file);
        assert_eq!(out.shape_count, 0);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "PLACEMENT_UNRESOLVED" && l.detail.contains("loops")),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn a_swept_solid_representation_is_a_loss_not_a_crash() {
        let file = plate_ifc().replace(
            "#23=IFCSHAPEREPRESENTATION(#2,'Body','Tessellation',(#24));",
            "#23=IFCSHAPEREPRESENTATION(#2,'Body','SweptSolid',(#24));",
        );
        let out = read_str(&file);
        assert_eq!(out.product_count, 1);
        assert_eq!(out.shape_count, 0);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "REPRESENTATION_NOT_TESSELLATED"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn an_out_of_range_triangle_index_refuses_the_body() {
        let file = plate_ifc().replace("((1,2,3),(1,3,4))", "((1,2,3),(1,3,9))");
        let out = read_str(&file);
        assert_eq!(out.shape_count, 0);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "TRIANGLE_INDEX_OUT_OF_RANGE"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn an_unreadable_coordinate_list_is_a_loss() {
        let file = plate_ifc().replace(
            "#25=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(100.,0.,0.),(100.,80.,0.),(0.,80.,0.)));",
            "#25=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(100.,0.,0.),junk));",
        );
        let out = read_str(&file);
        assert_eq!(out.shape_count, 0);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "COORDINATE_LIST_UNREADABLE"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn a_fragmented_instance_is_a_loss_never_a_panic() {
        // A point list split over two lines: the second line is not an instance, so the
        // arguments of the instance do not split and the body is refused.
        let file = plate_ifc().replace(
            "#25=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(100.,0.,0.),(100.,80.,0.),(0.,80.,0.)));",
            "#25=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),\n(100.,0.,0.)));",
        );
        let out = read_str(&file);
        assert_eq!(out.shape_count, 0);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "INSTANCE_ARGS_UNREADABLE"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn a_file_without_a_si_length_unit_counts_products_but_reads_no_shapes() {
        let file = plate_ifc().replace(
            "#5=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);",
            "#5=IFCCONVERSIONBASEDUNIT(*,.LENGTHUNIT.,'INCH',#27);",
        );
        let out = read_str(&file);
        assert_eq!(out.length_unit, None);
        assert_eq!(out.product_count, 1);
        assert_eq!(out.shape_count, 0);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "GEOMETRY_WITHOUT_LENGTH_UNIT"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn a_repeated_instance_number_disables_the_read() {
        let file = plate_ifc().replace(
            "ENDSEC;\nEND-ISO-10303-21;",
            "#20=IFCWALL('1Anr5VMOX0PO8jvHqZabcd',*,'dup',$,$,$,$,$,$);\n\
             ENDSEC;\nEND-ISO-10303-21;",
        );
        let out = read_str(&file);
        assert_eq!(out.shape_count, 0);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "REPEATED_INSTANCE_NUMBER"),
            "{:?}",
            out.losses
        );
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "REFERENCE_RESOLUTION_DISABLED"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn several_data_sections_disable_the_read() {
        let file = plate_ifc().replace(
            "ENDSEC;\nEND-ISO-10303-21;",
            "ENDSEC;\nDATA;\n#99=IFCWALL('1Anr5VMOX0PO8jvHqZabcd',*,'x',$,$,$,$,$,$);\n\
             ENDSEC;\nEND-ISO-10303-21;",
        );
        let out = read_str(&file);
        assert_eq!(out.shape_count, 0);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "MULTIPLE_DATA_SECTIONS"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn a_degenerate_axis_frame_is_a_loss() {
        let file = plate_ifc().replace(
            "#21=IFCLOCALPLACEMENT(#11,#12);",
            "#21=IFCLOCALPLACEMENT(#11,#27);\
             \n#27=IFCAXIS2PLACEMENT3D(#28,#29,#30);\
             \n#28=IFCCARTESIANPOINT((0.,0.,0.));\
             \n#29=IFCDIRECTION((1.,0.,0.));\
             \n#30=IFCDIRECTION((1.,0.,0.));",
        );
        let out = read_str(&file);
        assert_eq!(out.shape_count, 0);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "PLACEMENT_UNRESOLVED" && l.detail.contains("degenerate")),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn a_metre_file_scales_to_millimetres() {
        let file = plate_ifc().replace(
            "#5=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);",
            "#5=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);",
        );
        let out = read_str(&file);
        let shape = out.shapes.first().unwrap();
        assert_eq!(shape.vertices_mm[1], [100000.0, 0.0, 0.0]);
    }

    #[test]
    fn a_long_product_name_is_omitted_with_a_loss() {
        let long = "N".repeat(MAX_NAME_BYTES + 1);
        let file = plate_ifc().replace("'PL-A'", &format!("'{long}'"));
        let out = read_str(&file);
        assert!(out.shapes.first().unwrap().name.is_none());
        assert!(
            out.losses.iter().any(|l| l.code == "PRODUCT_NAME_OMITTED"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn an_invalid_global_id_reads_as_none_without_a_loss() {
        let file = plate_ifc().replace("'1Anr5VMOX0PO8jvHqZabcd'", "'not a global id at all'");
        let out = read_str(&file);
        let shape = out.shapes.first().unwrap();
        assert_eq!(shape.global_id, None);
        assert_eq!(out.shape_count, 1);
    }

    #[test]
    fn framing_breaks_stay_refusals() {
        assert_eq!(
            read(b"not a step file\n"),
            Err(ReadError::NotAStepFile(
                "the first line is not ISO-10303-21;"
            ))
        );
    }

    #[test]
    fn the_vertex_budget_stops_the_scan() {
        // A long but readable list parses first; the probe pins the list grammar: the
        // argument is one outer paren over the point tuples.
        let mut list = String::from("(");
        for _ in 0..1000 {
            list.push_str("(1.,2.,3.),");
        }
        list.push_str("(4.,5.,6.))");
        let mut count = 0usize;
        let result = for_each_point(&list, &mut |_| {
            count += 1;
            true
        });
        assert!(result.is_ok(), "the probe list must parse");
        assert_eq!(count, 1001);
        // More points than the budget: the scan stops with the budget loss, and the
        // unread bodies are never read.
        let mut list = String::from("(");
        for _ in 0..=MAX_TOTAL_VERTICES {
            list.push_str("(1.,2.,3.),");
        }
        list.push_str("(4.,5.,6.))");
        let file = plate_ifc().replace(
            "#25=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(100.,0.,0.),(100.,80.,0.),(0.,80.,0.)));",
            &format!("#25=IFCCARTESIANPOINTLIST3D({list});"),
        );
        let out = read_str(&file);
        assert!(
            out.losses.iter().any(|l| l.code == "SHAPE_BUDGET_EXCEEDED"),
            "{:?}",
            out.losses
        );
        assert_eq!(out.shape_count, 0);
    }

    /// The writer round trip: what `write_ifc` emits, `read` reads back placed.
    #[test]
    fn the_own_export_reads_back_placed() {
        let id = Uuid7::parse("01990000-0000-7000-8000-00000000a001").unwrap();
        let mesh = Mesh {
            points: vec![
                [0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [100.0, 80.0, 0.0],
                [0.0, 80.0, 10.0],
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            closed: true,
        };
        let model = ExportModel {
            project_id: id,
            project_name: None,
            revision_unix_ms: 0,
            levels: Vec::new(),
            materials: Vec::new(),
            elements: vec![ElementData {
                id,
                entity: Entity::Plate,
                kind_token: "plate".to_owned(),
                name: "PL-1".to_owned(),
                schema_version: "0.1.0".to_owned(),
                geometry_mode: "exact".to_owned(),
                pose: Some(Pose {
                    translation_mm: [1000.0, 0.0, 0.0],
                    axis: [0.0, 0.0, 1.0],
                    ref_direction: [1.0, 0.0, 0.0],
                }),
                mesh: Some(mesh.clone()),
                level: None,
                material: None,
                profile: None,
                hole_count: None,
                quantities: None,
                declared: None,
                assembly: None,
            }],
        };
        let (bytes, _) = crate::write_ifc(&model, &Limits::DEFAULT, Vec::new()).unwrap();
        let out = read(&bytes).ok().unwrap();
        assert!(out.losses.is_empty(), "{:?}", out.losses);
        assert_eq!(out.product_count, 1);
        assert_eq!(out.shape_count, 1);
        let shape = out.shapes.first().unwrap();
        assert_eq!(shape.entity, "IFCPLATE");
        assert!(shape.closed);
        assert_eq!(shape.triangles, mesh.triangles);
        // The placement chain (site, building) is identity; the element pose translation
        // is the only offset, so the placed vertices are the local ones plus (1000, 0, 0).
        assert_eq!(
            shape.vertices_mm.as_slice(),
            &[
                [1000.0, 0.0, 0.0],
                [1100.0, 0.0, 0.0],
                [1100.0, 80.0, 0.0],
                [1000.0, 80.0, 10.0]
            ]
        );
        assert_eq!(
            shape.bounds_mm,
            Bounds {
                min: [1000.0, 0.0, 0.0],
                max: [1100.0, 80.0, 10.0],
            }
        );
    }
}
