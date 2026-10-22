// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original test oracle. It shares no code with the layer engine: the parser, the
// composition and the vector algebra here are written only against the committed layer text and
// the public USD semantics the mapping relies on.

//! Reads a BeamCraft revision as an independent reader and checks it, replacing the retired
//! OpenUSD Python oracles with a std-only Rust process.
//!
//! Usage:
//! - `beamcraft-usd-oracle revision ROOT_LAYER EXPECTATION_JSON` (expectation: an array of
//!   plate/member entries)
//! - `beamcraft-usd-oracle reference ROOT_LAYER EXPECTATION_JSON` (expectation: one object)
//!
//! The directory of ROOT_LAYER holds the layers named in the revision manifest (root.usda,
//! authored.usda, derived.usda and any external layers). The oracle parses the layer text with
//! its own bounded parser, composes the layer stack and reference arcs, recomputes volumes,
//! bounds and placements from the parsed points, and cross-checks every authored opinion
//! against the expectation. It exits 0 and prints one JSON report only if every check passes;
//! otherwise it prints the failures to stderr and exits 1.

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use beamcraft_json::{Json, Limits};

/// The owned layers, strongest first, exactly as the writer composes a revision.
const OWNED_STACK: [&str; 3] = ["root.usda", "authored.usda", "derived.usda"];

/// Bounded text limits for one layer file.
const MAX_LAYER_BYTES: usize = 8 * 1024 * 1024;
const MAX_DEPTH: usize = 32;
const MAX_VALUES: usize = 200_000;
const MAX_STRING_BYTES: usize = 4096;
const MAX_NAME_BYTES: usize = 512;

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_default();
    let root = args.next().unwrap_or_default();
    let expect = args.next();
    if !matches!(mode.as_str(), "revision" | "reference" | "marks") {
        eprintln!("usage: beamcraft-usd-oracle revision|reference ROOT_LAYER EXPECTATION_JSON");
        eprintln!("       beamcraft-usd-oracle marks ROOT_LAYER");
        return std::process::ExitCode::from(2);
    }
    let marks_only = mode == "marks";
    let expect = match expect {
        Some(expect) => expect,
        None if marks_only => String::new(),
        None => {
            eprintln!("usage: beamcraft-usd-oracle revision|reference ROOT_LAYER EXPECTATION_JSON");
            return std::process::ExitCode::from(2);
        }
    };
    let outcome = run(&mode, Path::new(&root), &expect, marks_only);
    let (report, messages) = match outcome {
        Ok(report) => (report, Vec::new()),
        Err(messages) => (
            Json::object([("failed", Json::uint(messages.len() as u64))]),
            messages,
        ),
    };
    if messages.is_empty() {
        match report.encode(&Limits::DEFAULT) {
            Ok(bytes) => {
                println!("{}", String::from_utf8_lossy(&bytes));
                return std::process::ExitCode::SUCCESS;
            }
            Err(e) => eprintln!("FAIL report: {e}"),
        }
    } else {
        for message in &messages {
            eprintln!("FAIL {message}");
        }
        eprintln!("{} oracle FAILED: {} failures", mode, messages.len());
    }
    std::process::ExitCode::FAILURE
}

/// Runs all checks for `mode`; `Ok` carries the JSON report. In the `marks` mode the report is
/// the independent read itself (every approved mark, keyed by element id) and no expectation
/// file is read.
fn run(mode: &str, root: &Path, expect_path: &str, marks_only: bool) -> Result<Json, Vec<String>> {
    let mut findings = Findings::default();
    let report: Option<Json> = (|| {
        let stage = Stage::open(root);
        if marks_only {
            let report = read_marks(&stage);
            findings.messages.extend(stage.take_problems());
            return findings.messages.is_empty().then_some(report);
        }
        // An unreadable or undecodable expectation is a failure, never a silent pass.
        let bytes = match std::fs::read(expect_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                findings.fail(format!("{expect_path}: {e}"));
                return None;
            }
        };
        let expect = match beamcraft_json::decode(&bytes, &Limits::DEFAULT) {
            Ok(expect) => expect,
            Err(e) => {
                findings.fail(format!("{expect_path}: {e}"));
                return None;
            }
        };
        let report = if mode == "reference" {
            check_reference(&stage, &expect, &mut findings)
        } else {
            check_revision(&stage, &expect, &mut findings)
        };
        findings.messages.extend(stage.take_problems());
        findings.messages.is_empty().then_some(report)
    })();
    match report {
        Some(report) => Ok(report),
        None => Err(findings.messages),
    }
}

/// The independent read of every approved mark: `{element id: [series, number, basis]}`.
fn read_marks(stage: &Stage) -> Json {
    let mut marks: Vec<(String, Json)> = Vec::new();
    for (_, layer, foreign) in stage.layers.borrow().iter() {
        if *foreign {
            continue;
        }
        for spec in &layer.specs {
            let value = |name: &str| spec.attributes.iter().find(|(k, _)| k == name);
            let (Some(series), Some(number), Some(basis), Some(id)) = (
                value("steel:numbering:series").and_then(|(_, v)| v.as_text()),
                value("steel:numbering:number").and_then(|(_, v)| v.as_f64()),
                value("steel:numbering:basis").and_then(|(_, v)| v.as_text()),
                value("bim:id").and_then(|(_, v)| v.as_text()),
            ) else {
                continue;
            };
            // Marks are whole numbers; a fractional spelling is a corrupt mark.
            let number = if number.fract() == 0.0 && number.is_sign_positive() {
                Json::uint(number as u64)
            } else {
                Json::Null
            };
            // Owned layers are strongest first: the first opinion on an element wins.
            if !marks.iter().any(|(k, _)| *k == id) {
                marks.push((
                    id.to_owned(),
                    Json::Array(vec![Json::text(series), number, Json::text(basis)]),
                ));
            }
        }
    }
    marks.sort_by(|a, b| a.0.cmp(&b.0));
    Json::Object(marks.into_iter().collect())
}

/// Collected failures; checks keep going so one run reports everything.
#[derive(Default)]
struct Findings {
    messages: Vec<String>,
}

impl Findings {
    fn check(&mut self, ok: bool, message: String) {
        if !ok {
            self.messages.push(message);
        }
    }

    fn fail(&mut self, message: String) {
        self.messages.push(message);
    }

    fn close(&mut self, where_: &str, actual: f64, expected: f64, tol: f64) {
        self.check(
            (actual - expected).abs() <= tol,
            format!("{where_}: {actual:?} != {expected:?} (tol {tol})"),
        );
    }

    fn close_rel(&mut self, where_: &str, actual: f64, expected: f64, rel: f64) {
        let within = (actual - expected).abs() <= rel * expected.abs().max(1e-300);
        self.check(
            within,
            format!("{where_}: {actual:?} != {expected:?} (rel {rel})"),
        );
    }
}

// ---- Vector algebra -------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
struct V3 {
    x: f64,
    y: f64,
    z: f64,
}

impl V3 {
    fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    fn scale(self, k: f64) -> Self {
        Self::new(self.x * k, self.y * k, self.z * k)
    }

    fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    fn norm(self) -> f64 {
        self.dot(self).sqrt()
    }
}

/// A unit quaternion in USD real-first storage.
#[derive(Clone, Copy)]
struct Quat {
    s: f64,
    x: f64,
    y: f64,
    z: f64,
}

impl Quat {
    fn rotate(self, v: V3) -> V3 {
        let n = V3::new(self.x, self.y, self.z);
        n.scale(2.0 * n.dot(v))
            .add(v.scale(self.s * self.s - n.dot(n)))
            .add(n.cross(v).scale(2.0 * self.s))
    }
}

fn bounds_of(points: &[V3]) -> (V3, V3) {
    let mut lo = V3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = V3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        lo = V3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = V3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    (lo, hi)
}

/// One component of a point, named for the check messages.
fn pick(v: V3, axis: &str) -> f64 {
    match axis {
        "x" => v.x,
        "y" => v.y,
        _ => v.z,
    }
}

/// Divergence-theorem volume of a closed polygon mesh (fan triangulation per face); `None`
/// when a face count is absurd or the indices do not cover the faces.
fn signed_volume(points: &[V3], counts: &[usize], indices: &[usize]) -> Option<f64> {
    let mut total = 0.0;
    let mut cursor = 0usize;
    for count in counts {
        let end = cursor.checked_add(*count)?;
        let face = indices.get(cursor..end)?;
        cursor = end;
        let Some(first) = face.first().and_then(|i| points.get(*i)).copied() else {
            continue;
        };
        for pair in face.get(1..).unwrap_or(&[]).windows(2) {
            let Some(b) = pair.first().and_then(|i| points.get(*i)).copied() else {
                continue;
            };
            let Some(c) = pair.get(1).and_then(|i| points.get(*i)).copied() else {
                continue;
            };
            total += first.dot(b.cross(c)) / 6.0;
        }
    }
    Some(total)
}

/// Newell corner normal of a face, from its first three points.
fn face_normal(points: &[V3], face: &[usize]) -> Option<V3> {
    let a = points.get(*face.first()?).copied()?;
    let b = points.get(*face.get(1)?).copied()?;
    let c = points.get(*face.get(2)?).copied()?;
    Some(b.sub(a).cross(c.sub(a)))
}

// ---- The usda parser -------------------------------------------------------------------------

/// A parsed attribute or metadata value.
#[derive(Debug, Clone)]
enum Value {
    Text(String),
    Num(f64),
    Tuple(Vec<Value>),
    List(Vec<Value>),
}

impl Value {
    fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    fn as_vec3(&self) -> Option<V3> {
        match self {
            Value::Tuple(items) if items.len() == 3 => Some(V3::new(
                items.first()?.as_f64()?,
                items.get(1)?.as_f64()?,
                items.get(2)?.as_f64()?,
            )),
            _ => None,
        }
    }

    fn as_quat(&self) -> Option<Quat> {
        match self {
            Value::Tuple(items) if items.len() == 4 => Some(Quat {
                s: items.first()?.as_f64()?,
                x: items.get(1)?.as_f64()?,
                y: items.get(2)?.as_f64()?,
                z: items.get(3)?.as_f64()?,
            }),
            _ => None,
        }
    }

    fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(items) => Some(items),
            _ => None,
        }
    }

    fn as_v3_list(&self) -> Vec<V3> {
        self.as_list()
            .map(|items| items.iter().filter_map(Value::as_vec3).collect())
            .unwrap_or_default()
    }

    fn as_num_list(&self) -> Vec<usize> {
        self.as_list()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_f64)
                    .map(|f| if f >= 0.0 { f as usize } else { 0 })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// One prim spec as authored in one layer.
#[derive(Debug, Default, Clone)]
struct Spec {
    path: String,
    type_name: Option<String>,
    kind: Option<String>,
    display_name: Option<String>,
    /// The reference arc: (asset path, target prim name).
    reference: Option<(String, String)>,
    attributes: Vec<(String, Value)>,
    /// The prim had a body in this layer.
    has_body: bool,
}

/// One parsed layer file.
#[derive(Debug, Default, Clone)]
struct Layer {
    meters_per_unit: Option<f64>,
    up_axis: Option<String>,
    default_prim: Option<String>,
    sublayers: Vec<String>,
    specs: Vec<Spec>,
}

/// Bounded recursive-descent parser for the canonical text the writer emits. Anything the
/// writer cannot emit is an error: strictness here is the structural compliance check.
struct Parser {
    bytes: Vec<u8>,
    pos: usize,
    values: usize,
}

impl Parser {
    fn layer(path: &Path) -> Result<Layer, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if bytes.len() > MAX_LAYER_BYTES {
            return Err(format!(
                "{}: larger than {MAX_LAYER_BYTES} bytes",
                path.display()
            ));
        }
        let mut parser = Self {
            bytes,
            pos: 0,
            values: 0,
        };
        parser.document()
    }

    fn error(&self, what: &str) -> String {
        let line = self
            .bytes
            .get(..self.pos.min(self.bytes.len()))
            .unwrap_or(&[])
            .iter()
            .filter(|b| **b == b'\n')
            .count()
            + 1;
        format!("usda line {line}: {what}")
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn word(&mut self) -> String {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'.'))
        {
            self.pos += 1;
        }
        String::from_utf8_lossy(self.bytes.get(start..self.pos).unwrap_or(&[])).into_owned()
    }

    fn literal(&mut self, word: &str) -> bool {
        if self.bytes.get(self.pos..self.pos + word.len()) == Some(word.as_bytes()) {
            self.pos += word.len();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.bump() == Some(byte) {
            Ok(())
        } else {
            self.pos = self.pos.saturating_sub(1);
            Err(self.error(&format!("expected {:?}", byte as char)))
        }
    }

    fn quoted(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => return Ok(out),
                Some(b'\\') => match self.bump() {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    _ => return Err(self.error("bad escape")),
                },
                Some(byte) if byte < 0x20 => return Err(self.error("control byte in string")),
                Some(byte) => out.push(byte as char),
            }
            if out.len() > MAX_STRING_BYTES {
                return Err(self.error("string too long"));
            }
        }
    }

    /// An asset reference: `@relative/path@`.
    fn asset(&mut self) -> Result<String, String> {
        self.expect(b'@')?;
        let start = self.pos;
        while self.peek().is_some_and(|b| b != b'@') {
            self.pos += 1;
        }
        let text =
            String::from_utf8_lossy(self.bytes.get(start..self.pos).unwrap_or(&[])).into_owned();
        self.expect(b'@')?;
        Ok(text)
    }

    fn number(&mut self) -> Result<f64, String> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'))
        {
            self.pos += 1;
        }
        let text =
            String::from_utf8_lossy(self.bytes.get(start..self.pos).unwrap_or(&[])).into_owned();
        text.parse::<f64>()
            .map_err(|_| self.error(&format!("bad number {text:?}")))
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("value too deep"));
        }
        self.values += 1;
        if self.values > MAX_VALUES {
            return Err(self.error("too many values"));
        }
        self.skip_space();
        match self.peek() {
            Some(b'"') => self.quoted().map(Value::Text),
            Some(b'(') => self.bracketed(b')', depth).map(Value::Tuple),
            Some(b'[') => self.bracketed(b']', depth).map(Value::List),
            Some(_) => self.number().map(Value::Num),
            None => Err(self.error("expected a value")),
        }
    }

    /// A `(`- or `[`-delimited value list, comma separated, trailing comma allowed.
    fn bracketed(&mut self, close: u8, depth: usize) -> Result<Vec<Value>, String> {
        self.bump();
        let mut items = Vec::new();
        loop {
            self.skip_space();
            if self.peek() == Some(close) {
                self.bump();
                return Ok(items);
            }
            items.push(self.value(depth + 1)?);
            self.skip_space();
            match self.peek() {
                Some(b',') => {
                    self.bump();
                }
                _ => {
                    self.expect(close)?;
                    return Ok(items);
                }
            }
            if items.len() > MAX_VALUES {
                return Err(self.error("too many values"));
            }
        }
    }

    fn document(&mut self) -> Result<Layer, String> {
        self.skip_space();
        if !self.literal("#usda 1.0") {
            return Err(self.error("not a usda text layer"));
        }
        let mut layer = Layer::default();
        self.skip_space();
        if self.peek() == Some(b'(') {
            self.bump();
            loop {
                self.skip_space();
                match self.peek() {
                    Some(b')') => {
                        self.bump();
                        break;
                    }
                    None => return Err(self.error("unterminated layer metadata")),
                    _ => self.layer_metadata(&mut layer)?,
                }
            }
        }
        loop {
            self.skip_space();
            if !self.starts_prim() {
                break;
            }
            let mut spec = self.prim_header("", 0)?;
            if spec.has_body {
                self.prim_body(&mut spec, &mut layer.specs, 0)?;
            }
            layer.specs.push(spec);
        }
        self.skip_space();
        if self.pos != self.bytes.len() {
            return Err(self.error("trailing bytes after the layer"));
        }
        Ok(layer)
    }

    fn layer_metadata(&mut self, layer: &mut Layer) -> Result<(), String> {
        if self.literal("customLayerData") {
            self.skip_space();
            self.expect(b'=')?;
            self.skip_space();
            return self.skip_dict();
        }
        if self.literal("subLayers") {
            self.skip_space();
            self.expect(b'=')?;
            self.skip_space();
            self.expect(b'[')?;
            loop {
                self.skip_space();
                if self.peek() == Some(b']') {
                    self.bump();
                    return Ok(());
                }
                let asset = self.asset()?;
                layer.sublayers.push(asset);
                self.skip_space();
                match self.peek() {
                    Some(b',') => {
                        self.bump();
                    }
                    _ => {
                        self.expect(b']')?;
                        return Ok(());
                    }
                }
            }
        }
        let key = self.word();
        self.skip_space();
        self.expect(b'=')?;
        self.skip_space();
        match key.as_str() {
            "metersPerUnit" => {
                layer.meters_per_unit = Some(self.number()?);
                Ok(())
            }
            "upAxis" => {
                layer.up_axis = Some(self.quoted()?);
                Ok(())
            }
            "defaultPrim" => {
                layer.default_prim = Some(self.quoted()?);
                Ok(())
            }
            other => Err(self.error(&format!("unexpected layer metadata {other:?}"))),
        }
    }

    /// Skips a `{ ... }` dictionary without interpreting it.
    fn skip_dict(&mut self) -> Result<(), String> {
        self.skip_space();
        self.expect(b'{')?;
        let mut depth = 1usize;
        while depth > 0 {
            match self.bump() {
                None => return Err(self.error("unterminated dictionary")),
                Some(b'{') => depth += 1,
                Some(b'}') => depth -= 1,
                _ => (),
            }
        }
        Ok(())
    }

    fn starts_prim(&mut self) -> bool {
        let saved = self.pos;
        let word = self.word();
        self.pos = saved;
        word == "def" || word == "over"
    }

    /// Parses `def|over [Type] "Name" ( metadata ) {`.
    fn prim_header(&mut self, parent: &str, depth: usize) -> Result<Spec, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("prims too deep"));
        }
        let specifier = self.word();
        if specifier != "def" && specifier != "over" {
            return Err(self.error(&format!("expected def or over, found {specifier:?}")));
        }
        self.skip_space();
        let type_name = match self.peek() {
            Some(b'"') => None,
            _ => {
                let name = self.word();
                if name.is_empty() {
                    return Err(self.error("missing prim type"));
                }
                Some(name)
            }
        };
        self.skip_space();
        let name = self.quoted()?;
        if name.is_empty() || name.len() > MAX_NAME_BYTES {
            return Err(self.error("bad prim name"));
        }
        let mut spec = Spec {
            path: format!("{parent}/{name}"),
            type_name,
            ..Spec::default()
        };
        self.skip_space();
        if self.peek() == Some(b'(') {
            self.bump();
            loop {
                self.skip_space();
                match self.peek() {
                    Some(b')') => {
                        self.bump();
                        break;
                    }
                    None => return Err(self.error("unterminated prim metadata")),
                    _ => self.prim_metadata(&mut spec)?,
                }
            }
        }
        self.skip_space();
        if self.peek() == Some(b'{') {
            self.bump();
            spec.has_body = true;
        }
        Ok(spec)
    }

    fn prim_metadata(&mut self, spec: &mut Spec) -> Result<(), String> {
        if self.literal("prepend") {
            self.skip_space();
            if !self.literal("references") {
                return Err(self.error("only references may be prepended"));
            }
            self.skip_space();
            self.expect(b'=')?;
            self.skip_space();
            let asset = self.asset()?;
            self.skip_space();
            self.expect(b'<')?;
            // The target is a prim path: `</Site>`, leading slash included.
            self.expect(b'/')?;
            let target = self.word();
            self.expect(b'>')?;
            if spec.reference.is_some() {
                return Err(self.error("more than one reference arc"));
            }
            spec.reference = Some((asset, target));
            return Ok(());
        }
        let key = self.word();
        self.skip_space();
        self.expect(b'=')?;
        self.skip_space();
        match key.as_str() {
            "kind" => {
                spec.kind = Some(self.quoted()?);
                Ok(())
            }
            "displayName" => {
                spec.display_name = Some(self.quoted()?);
                Ok(())
            }
            other => Err(self.error(&format!("unexpected prim metadata {other:?}"))),
        }
    }

    /// Parses the body of `spec`: child prims, attributes and relationships. Children go
    /// straight into `specs`; the layer list order does not affect composition.
    fn prim_body(
        &mut self,
        spec: &mut Spec,
        specs: &mut Vec<Spec>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(self.error("prims too deep"));
        }
        loop {
            self.skip_space();
            match self.peek() {
                Some(b'}') => {
                    self.bump();
                    return Ok(());
                }
                None => return Err(self.error("unterminated prim body")),
                _ => (),
            }
            if self.starts_prim() {
                let mut child = self.prim_header(&spec.path, depth + 1)?;
                if child.has_body {
                    self.prim_body(&mut child, specs, depth + 1)?;
                }
                specs.push(child);
                continue;
            }
            self.property(spec)?;
        }
    }

    fn property(&mut self, spec: &mut Spec) -> Result<(), String> {
        if self.literal("custom") {
            self.skip_space();
        }
        if self.peek() == Some(b'r') && self.literal("rel") {
            self.skip_space();
            let name = self.word();
            self.skip_space();
            if !self.literal("=") {
                return Err(self.error("relationship without a target"));
            }
            self.skip_space();
            self.expect(b'<')?;
            let start = self.pos;
            while self.peek().is_some_and(|b| b != b'>') {
                self.pos += 1;
            }
            let target = String::from_utf8_lossy(self.bytes.get(start..self.pos).unwrap_or(&[]))
                .into_owned();
            self.expect(b'>')?;
            return spec.push_attr(name, Value::Text(target));
        }
        let mut type_name = self.word();
        if self.literal("[]") {
            type_name.push_str("[]");
        }
        const TYPES: &[&str] = &[
            "string",
            "token",
            "double",
            "int",
            "double3",
            "quatd",
            "double3[]",
            "token[]",
            "int[]",
            "float3[]",
            "point3f[]",
            "normal3f[]",
        ];
        if !TYPES.contains(&type_name.as_str()) {
            return Err(self.error(&format!("unexpected attribute type {type_name:?}")));
        }
        self.skip_space();
        let name = self.word();
        self.skip_space();
        self.expect(b'=')?;
        let value = self.value(0)?;
        self.skip_space();
        if self.peek() == Some(b'(') {
            self.bump();
            loop {
                self.skip_space();
                if self.peek() == Some(b')') {
                    self.bump();
                    break;
                }
                if self.literal("interpolation") {
                    self.skip_space();
                    self.expect(b'=')?;
                    self.skip_space();
                    let mode = self.quoted()?;
                    if mode != "uniform" {
                        return Err(self.error("unsupported interpolation"));
                    }
                } else {
                    return Err(self.error("unexpected attribute metadata"));
                }
            }
        }
        spec.push_attr(name, value)
    }
}

trait AttrPush {
    fn push_attr(&mut self, name: String, value: Value) -> Result<(), String>;
}

impl AttrPush for Spec {
    fn push_attr(&mut self, name: String, value: Value) -> Result<(), String> {
        if name.is_empty() || name.len() > 256 {
            return Err(format!("bad attribute name {name:?}"));
        }
        if self.attributes.iter().any(|(k, _)| *k == name) {
            return Err(format!("attribute {name:?} authored twice"));
        }
        self.attributes.push((name, value));
        Ok(())
    }
}

// ---- Composition ------------------------------------------------------------------------------

/// The composed stage: owned layers strongest first, then the referenced foreign subtree.
struct Stage {
    dir: PathBuf,
    layers: RefCell<Vec<(String, Layer, bool)>>,
    problems: RefCell<Vec<String>>,
    /// Foreign specs keyed by their composed path, filled when the arc is resolved.
    foreign_specs: RefCell<BTreeMap<String, Spec>>,
    /// (asset, foreign layer) of the one resolved reference arc.
    foreign_layer: RefCell<Option<(String, Layer)>>,
}

impl Stage {
    /// Loads the root layer and its sublayers; the stack must be exactly the owned three.
    fn open(root: &Path) -> Self {
        let stage = Self {
            dir: root.parent().unwrap_or(Path::new(".")).to_path_buf(),
            layers: RefCell::new(Vec::new()),
            problems: RefCell::new(Vec::new()),
            foreign_specs: RefCell::new(BTreeMap::new()),
            foreign_layer: RefCell::new(None),
        };
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut problems = stage.problems.borrow_mut();
        if name != OWNED_STACK[0] {
            problems.push(format!(
                "the root layer is {name:?}, expected {:?}",
                OWNED_STACK[0]
            ));
        }
        drop(problems);
        let root_layer = stage.load(&name, false);
        let sublayers = root_layer.sublayers.clone();
        stage.layers.borrow_mut().push((name, root_layer, false));
        {
            let mut problems = stage.problems.borrow_mut();
            for (index, wanted) in OWNED_STACK.iter().enumerate().skip(1) {
                match sublayers.get(index - 1) {
                    Some(got) if got == wanted => (),
                    Some(got) => problems.push(format!("sublayer {got:?} is not {wanted:?}")),
                    None => problems.push(format!("the sublayer {wanted:?} is missing")),
                }
            }
            if sublayers.len() != OWNED_STACK.len() - 1 {
                problems.push(format!(
                    "the layer stack names {} sublayers, expected {}",
                    sublayers.len(),
                    OWNED_STACK.len() - 1
                ));
            }
        }
        for sub in sublayers.iter().take(OWNED_STACK.len() - 1) {
            let layer = stage.load(sub, false);
            stage.layers.borrow_mut().push((sub.clone(), layer, false));
        }
        stage.resolve_arcs();
        stage
    }

    fn load(&self, name: &str, foreign: bool) -> Layer {
        let path = self.dir.join(name);
        let parsed = Parser::layer(&path);
        match parsed {
            Ok(layer) => {
                if !layer.sublayers.is_empty() && foreign {
                    self.problem(format!("{name}: a foreign layer declares sublayers"));
                }
                layer
            }
            Err(message) => {
                self.problem(message);
                Layer::default()
            }
        }
    }

    fn problem(&self, message: String) {
        self.problems.borrow_mut().push(message);
    }

    fn take_problems(&self) -> Vec<String> {
        std::mem::take(&mut self.problems.borrow_mut())
    }

    /// The specs authored for `path` across the owned layers, strongest first.
    fn owned_specs(&self, path: &str) -> Vec<Spec> {
        self.layers
            .borrow()
            .iter()
            .filter(|(_, _, foreign)| !foreign)
            .filter_map(|(_, layer, _)| layer.specs.iter().find(|s| s.path == path).cloned())
            .collect()
    }

    /// A composed prim: local opinions, then the foreign subtree baked by [`Self::resolve_arcs`].
    fn prim(&self, path: &str) -> Option<Prim> {
        let mut prim = Prim {
            path: path.to_owned(),
            ..Prim::default()
        };
        for spec in &self.owned_specs(path) {
            prim.absorb(spec);
        }
        let foreign = self.foreign_specs.borrow().get(path).cloned();
        if let Some(foreign_spec) = foreign {
            let layer = self
                .foreign_layer
                .borrow()
                .as_ref()
                .map(|(_, l)| l.clone())
                .unwrap_or_default();
            prim.absorb_foreign(Some(&foreign_spec), &layer, self);
        }
        Some(prim)
    }

    /// Resolves every reference arc the owned layers author: at most one foreign layer, whose
    /// subtree is keyed by its composed path. Order-independent, so lookups below the arc
    /// holder compose the same way whenever they happen.
    fn resolve_arcs(&self) {
        let arcs: Vec<(String, String, String)> = self
            .layers
            .borrow()
            .iter()
            .filter(|(_, _, foreign)| !foreign)
            .flat_map(|(_, layer, _)| {
                layer.specs.iter().filter_map(|s| {
                    s.reference
                        .clone()
                        .map(|(asset, target)| (s.path.clone(), asset, target))
                })
            })
            .collect();
        for (holder, asset, target) in arcs {
            let known = self.foreign_layer.borrow().as_ref().map(|(a, _)| a.clone());
            match known {
                Some(known) if known != asset => {
                    self.problem(format!("{holder}: a second referenced layer {asset:?}"));
                    continue;
                }
                Some(_) => continue,
                None => (),
            }
            let foreign = self.load(&asset, true);
            let foreign_path = format!("/{target}");
            let mut subtree = self.foreign_specs.borrow_mut();
            for spec in &foreign.specs {
                if let Some(rest) = spec.path.strip_prefix(&foreign_path) {
                    subtree.insert(format!("{holder}{rest}"), spec.clone());
                }
            }
            if !subtree.contains_key(&holder) {
                self.problem(format!(
                    "{holder}: the referenced layer {asset:?} lacks {foreign_path:?}"
                ));
            }
            *self.foreign_layer.borrow_mut() = Some((asset, foreign));
        }
    }

    /// The resolved reference arc: (asset, foreign layer).
    fn foreign(&self) -> Option<(String, Layer)> {
        self.foreign_layer.borrow().clone()
    }

    /// Stage-level metadata from the root layer.
    fn root_layer(&self) -> Layer {
        self.layers
            .borrow()
            .first()
            .map(|(_, l, _)| l.clone())
            .unwrap_or_default()
    }
}

/// A composed prim: strongest local opinions, then the foreign subtree if referenced.
#[derive(Default, Clone)]
struct Prim {
    path: String,
    type_name: Option<String>,
    kind: Option<String>,
    display_name: Option<String>,
    attributes: BTreeMap<String, Value>,
}

impl Prim {
    fn absorb(&mut self, spec: &Spec) {
        if self.type_name.is_none() {
            self.type_name.clone_from(&spec.type_name);
        }
        if self.kind.is_none() {
            self.kind.clone_from(&spec.kind);
        }
        if self.display_name.is_none() {
            self.display_name.clone_from(&spec.display_name);
        }
        for (name, value) in &spec.attributes {
            self.attributes
                .entry(name.clone())
                .or_insert_with(|| value.clone());
        }
    }

    /// Foreign opinions are weaker than every local layer.
    fn absorb_foreign(&mut self, spec: Option<&Spec>, foreign: &Layer, stage: &Stage) {
        let Some(spec) = spec else {
            stage.problem("the referenced foreign prim is missing".to_owned());
            return;
        };
        if self.type_name.is_none() {
            self.type_name.clone_from(&spec.type_name);
        }
        if self.kind.is_none() {
            self.kind.clone_from(&spec.kind);
        }
        if self.display_name.is_none() {
            self.display_name.clone_from(&spec.display_name);
        }
        for (name, value) in &spec.attributes {
            self.attributes
                .entry(name.clone())
                .or_insert_with(|| value.clone());
        }
        let _ = foreign;
    }

    fn text(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).and_then(Value::as_text)
    }

    fn num(&self, name: &str) -> Option<f64> {
        self.attributes.get(name).and_then(Value::as_f64)
    }

    fn v3s(&self, name: &str) -> Vec<V3> {
        self.attributes
            .get(name)
            .map(Value::as_v3_list)
            .unwrap_or_default()
    }

    fn nums(&self, name: &str) -> Vec<usize> {
        self.attributes
            .get(name)
            .map(Value::as_num_list)
            .unwrap_or_default()
    }

    fn has(&self, name: &str) -> bool {
        self.attributes.contains_key(name)
    }
}

// ---- Placement --------------------------------------------------------------------------------

/// Applies the authored `xformOp:*` ops of `prim` to a point. The last op listed applies to the
/// point first, so the ops are applied in reverse listed order.
fn apply_ops(prim: &Prim, p: V3, findings: &mut Findings) -> V3 {
    let order: Vec<String> = prim
        .attributes
        .get("xformOpOrder")
        .and_then(Value::as_list)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_text)
                .map(|t| t.to_owned())
                .collect()
        })
        .unwrap_or_default();
    let mut point = p;
    for op in order.iter().rev() {
        match op.as_str() {
            "xformOp:translate" => match prim.vec3("xformOp:translate") {
                Some(t) => point = point.add(t),
                None => findings.fail(format!("{}: translate op without a value", prim.path)),
            },
            "xformOp:orient" => match prim.quat("xformOp:orient") {
                Some(q) => point = q.rotate(point),
                None => findings.fail(format!("{}: orient op without a value", prim.path)),
            },
            "xformOp:rotateX:bimUpAxis" => {
                let angle = prim.num("xformOp:rotateX:bimUpAxis").unwrap_or(0.0);
                let (sin, cos) = angle.to_radians().sin_cos();
                point = V3::new(
                    point.x,
                    point.y * cos - point.z * sin,
                    point.y * sin + point.z * cos,
                );
            }
            "xformOp:scale:bimUnits" => match prim.vec3("xformOp:scale:bimUnits") {
                Some(s) => point = V3::new(point.x * s.x, point.y * s.y, point.z * s.z),
                None => findings.fail(format!("{}: scale op without a value", prim.path)),
            },
            other => findings.fail(format!("{}: unexpected xform op {other:?}", prim.path)),
        }
    }
    point
}

impl Prim {
    fn vec3(&self, name: &str) -> Option<V3> {
        self.attributes.get(name).and_then(Value::as_vec3)
    }

    fn quat(&self, name: &str) -> Option<Quat> {
        self.attributes.get(name).and_then(Value::as_quat)
    }
}

/// The ancestor prims of `path`, root first, excluding `path` itself.
fn ancestors(stage: &Stage, path: &str) -> Vec<Prim> {
    let mut chain = Vec::new();
    let mut cursor = path.to_owned();
    while let Some((parent, _)) = cursor.rsplit_once('/') {
        if parent.is_empty() {
            break;
        }
        if let Some(prim) = stage.prim(parent) {
            chain.push(prim);
        }
        cursor = parent.to_owned();
    }
    // Nearest parent first: the parent's ops apply to the point before the grandparent's.
    chain
}

/// Maps local points of `path` to stage space through its own ops and every ancestor's ops.
fn world_points(stage: &Stage, path: &str, local: &[V3], findings: &mut Findings) -> Vec<V3> {
    let chain = ancestors(stage, path);
    let own = stage.prim(path);
    local
        .iter()
        .map(|p| {
            let mut v = *p;
            if let Some(prim) = &own {
                v = apply_ops(prim, v, findings);
            }
            for prim in &chain {
                v = apply_ops(prim, v, findings);
            }
            v
        })
        .collect()
}

// ---- Expectation helpers ----------------------------------------------------------------------

fn expect_text<'a>(expect: &'a Json, key: &str, findings: &mut Findings) -> Option<&'a str> {
    match expect.get(key).and_then(|v| v.string()) {
        Ok(text) => Some(text),
        Err(_) => {
            findings.fail(format!("the expectation lacks {key:?}"));
            None
        }
    }
}

fn expect_float(expect: &Json, key: &str, findings: &mut Findings) -> Option<f64> {
    match expect.get(key).and_then(|v| v.float()) {
        Ok(n) => Some(n),
        Err(_) => {
            findings.fail(format!("the expectation lacks {key:?}"));
            None
        }
    }
}

/// `Some(None)` when the key is present but null: an unknown that must stay unknown.
fn optional_float(expect: &Json, key: &str) -> Option<Option<f64>> {
    match expect.get(key) {
        Ok(Json::Null) => Some(None),
        Ok(v) => v.float().map(Some).ok(),
        Err(_) => None,
    }
}

fn expect_v3(expect: &Json, key: &str, findings: &mut Findings) -> Option<V3> {
    let array = expect.get(key).ok()?.array().ok()?;
    if array.len() != 3 {
        findings.fail(format!("the expectation {key:?} is not a 3-vector"));
        return None;
    }
    Some(V3::new(
        array.first()?.float().ok()?,
        array.get(1)?.float().ok()?,
        array.get(2)?.float().ok()?,
    ))
}

/// A mandatory 3-vector: an absent or malformed key is itself a finding, so a malformed
/// expectation cannot silently skip the check that reads it.
fn require_v3(expect: &Json, key: &str, findings: &mut Findings) -> Option<V3> {
    if expect.get(key).is_err() {
        findings.fail(format!("the expectation lacks {key:?}"));
        return None;
    }
    expect_v3(expect, key, findings)
}

fn dashless(id: &str) -> String {
    id.chars().filter(|c| *c != '-').collect()
}

fn report_number(value: f64) -> Json {
    Json::number(value).unwrap_or(Json::Null)
}

// ---- The revision checks ----------------------------------------------------------------------

fn check_revision(stage: &Stage, expect: &Json, findings: &mut Findings) -> Json {
    let root = stage.root_layer();
    findings.check(
        root.meters_per_unit == Some(0.001),
        format!("metersPerUnit is {:?}", root.meters_per_unit),
    );
    findings.check(
        root.up_axis.as_deref() == Some("Z"),
        format!("upAxis is {:?}", root.up_axis),
    );
    findings.check(
        root.default_prim.as_deref() == Some("World"),
        format!("defaultPrim is {:?}", root.default_prim),
    );
    let mut report_plates: Vec<Json> = Vec::new();
    let entries = match expect.array() {
        Ok(entries) => entries.to_vec(),
        Err(_) => {
            findings.fail("the expectation is not an array of element entries".to_owned());
            return Json::object([("plates", Json::Array(Vec::new()))]);
        }
    };
    for entry in &entries {
        if let Some(one) = check_element(stage, entry, findings) {
            report_plates.push(one);
        }
    }
    Json::object([("plates", Json::Array(report_plates))])
}

/// Checks one plate or member expectation entry; `Some` carries its report entry.
fn check_element(stage: &Stage, entry: &Json, findings: &mut Findings) -> Option<Json> {
    let id = expect_text(entry, "id", findings)?.to_owned();
    let name = expect_text(entry, "name", findings)?.to_owned();
    let kind = entry.get("kind").ok().and_then(|k| k.string().ok());
    let is_member = matches!(kind, Some("beam") | Some("column"));
    let path = format!("/World/Steel/Parts/PRT_{}", dashless(&id));
    let prim = stage.prim(&path)?;
    findings.check(
        prim.type_name.as_deref() == Some("Xform"),
        format!("{path}: type is {:?}", prim.type_name),
    );
    findings.check(
        prim.text("bim:id") == Some(id.as_str()),
        format!("{path}: bim:id is {:?}", prim.text("bim:id")),
    );
    findings.check(
        prim.text("bim:name") == Some(name.as_str()),
        format!("{path}: bim:name is {:?}", prim.text("bim:name")),
    );
    let element_kind = if is_member {
        kind.map(str::to_owned).unwrap_or_default()
    } else {
        "plate".to_owned()
    };
    findings.check(
        prim.text("bim:elementKind") == Some(element_kind.as_str()),
        format!(
            "{path}: bim:elementKind is {:?}",
            prim.text("bim:elementKind")
        ),
    );
    findings.check(
        prim.text("bim:geometryMode") == Some("parametric"),
        format!(
            "{path}: bim:geometryMode is {:?}",
            prim.text("bim:geometryMode")
        ),
    );
    if !is_member {
        findings.check(
            prim.text("bim:schemaVersion") == Some("0.1.0"),
            format!(
                "{path}: bim:schemaVersion is {:?}",
                prim.text("bim:schemaVersion")
            ),
        );
    }
    // Kind inheritance and display name, as a plain reader composes them.
    findings.check(
        prim.kind.as_deref() == Some("component"),
        format!("{path}: kind is {:?}", prim.kind),
    );
    findings.check(
        prim.display_name.as_deref() == Some(name.as_str()),
        format!("{path}: displayName is {:?}", prim.display_name),
    );
    check_folder_kinds(stage, findings);

    if is_member {
        check_member_inputs(stage, &prim, entry, findings);
    } else {
        check_plate_inputs(stage, &prim, entry, findings);
    }

    // The derived mesh and its quantities.
    let geometry_path = format!("{path}/Geometry");
    let mesh = stage.prim(&geometry_path)?;
    findings.check(
        mesh.type_name.as_deref() == Some("Mesh"),
        format!("{geometry_path}: type is {:?}", mesh.type_name),
    );
    findings.check(
        mesh.kind.as_deref() == Some("subcomponent"),
        format!("{geometry_path}: kind is {:?}", mesh.kind),
    );
    let points = mesh.v3s("points");
    let counts = mesh.nums("faceVertexCounts");
    let indices = mesh.nums("faceVertexIndices");
    findings.check(
        indices.iter().all(|i| *i < points.len().max(1)),
        format!("{geometry_path}: an index is out of range"),
    );
    findings.check(
        mesh.text("orientation") == Some("rightHanded"),
        format!(
            "{geometry_path}: orientation is {:?}",
            mesh.text("orientation")
        ),
    );
    findings.check(
        mesh.text("subdivisionScheme") == Some("none"),
        format!(
            "{geometry_path}: subdivisionScheme is {:?}",
            mesh.text("subdivisionScheme")
        ),
    );
    if entry.get("hole_counts").is_err() {
        findings.check(
            points.len() == 8 && counts.len() == 12 && indices.len() == 36,
            format!(
                "{geometry_path}: mesh counts are {}/{}/{}",
                points.len(),
                counts.len(),
                indices.len()
            ),
        );
    } else {
        findings.check(
            counts.iter().all(|c| *c == 3) && indices.len() == counts.len() * 3,
            format!("{geometry_path}: the mesh is not a triangle mesh"),
        );
        findings.check(
            counts.len() > 12,
            format!(
                "{geometry_path}: a plate with holes has {} faces",
                counts.len()
            ),
        );
    }
    check_standard_metadata(&mesh, &points, &counts, &indices, findings);

    let Some(volume) = signed_volume(&points, &counts, &indices) else {
        findings.fail(format!(
            "{geometry_path}: a face count is absurd or the indices do not cover the faces"
        ));
        return Some(Json::object([
            ("id", Json::text(id)),
            ("volume_mm3", Json::Null),
        ]));
    };
    findings.check(
        volume > 0.0,
        format!("{geometry_path}: volume {volume:?} is not outward oriented"),
    );
    if let Some(cli) = expect_float(entry, "cli_volume_mm3", findings) {
        findings.close_rel(&format!("{geometry_path}: volume"), volume, cli, 1e-6);
    }
    if let Some(Some(expected)) = optional_float(entry, "expected_volume_mm3") {
        findings.close_rel(&format!("{geometry_path}: volume"), volume, expected, 1e-6);
    }

    // Placement and world bounds.
    if let Some(translation) = expect_v3(entry, "translation", findings) {
        let origin = world_points(stage, &path, &[V3::new(0.0, 0.0, 0.0)], findings)
            .into_iter()
            .next()
            .unwrap_or(V3::new(f64::NAN, f64::NAN, f64::NAN));
        findings.check(
            origin == translation,
            format!("{path}: composed translation is {origin:?}"),
        );
    }
    let world = world_points(stage, &path, &points, findings);
    let (lo, hi) = bounds_of(&world);
    if let (Some(want_lo), Some(want_hi)) = (
        expect_v3(entry, "bounds_min", findings),
        expect_v3(entry, "bounds_max", findings),
    ) {
        for axis in ["x", "y", "z"] {
            findings.close(
                &format!("{path}: bounds.min.{axis}"),
                pick(lo, axis),
                pick(want_lo, axis),
                1e-3,
            );
            findings.close(
                &format!("{path}: bounds.max.{axis}"),
                pick(hi, axis),
                pick(want_hi, axis),
                1e-3,
            );
        }
    }

    // Stored quantity records beside the mesh.
    if let Some(cli) = expect_float(entry, "cli_volume_mm3", findings) {
        findings.close(
            &format!("{geometry_path}: stored volume"),
            mesh.num("bim:volumeMm3").unwrap_or(f64::NAN),
            cli,
            1e-6 * cli.abs().max(1.0),
        );
    }
    findings.check(
        mesh.text("bim:volumeStatus") == Some("current"),
        format!(
            "{geometry_path}: volume status is {:?}",
            mesh.text("bim:volumeStatus")
        ),
    );
    findings.check(
        mesh.text("bim:shapeSignature").map(str::len) == Some(64),
        format!("{geometry_path}: shape signature is not a sha256"),
    );
    match entry.get("cli_mass_kg") {
        Err(_) => (), // The key is absent: mass was not part of this expectation.
        Ok(Json::Null) => findings.check(
            !mesh.has("bim:massKg"),
            format!("{geometry_path}: an unknown mass has a stored value"),
        ),
        Ok(value) => {
            if let (Some(stored), Some(want)) = (mesh.num("bim:massKg"), value.float().ok()) {
                findings.close_rel(&format!("{geometry_path}: stored mass"), stored, want, 1e-6);
            }
        }
    }

    // Material: one pinned immutable definition, or no target at all (never a guessed default).
    // The member mass is cross-checked from the expectation density below; a member's material
    // target itself stays unchecked, as in the retired Python oracle.
    let material_target = prim.text("bim:material").map(str::to_owned);
    if is_member
        && let Some(Some(density)) = optional_float(entry, "density")
        && let Some(cli_mass) = expect_float(entry, "cli_mass_kg", findings)
    {
        let mass = volume * density / 1e9;
        findings.close_rel(&format!("{path}: derived mass"), mass, cli_mass, 1e-6);
    } else if !is_member {
        match entry.get("material") {
            Err(_) => findings.check(
                material_target.is_none(),
                format!("{path}: an unexpected material target {material_target:?}"),
            ),
            Ok(Json::Null) => findings.check(
                material_target.is_none(),
                format!("{path}: a plate without a material targets {material_target:?}"),
            ),
            Ok(material) => check_material(
                stage,
                &path,
                material,
                material_target,
                volume,
                entry,
                findings,
            ),
        }
    }
    Some(Json::object([
        ("id", Json::text(id)),
        ("volume_mm3", report_number(volume)),
    ]))
}

/// Authored member inputs: axis points, optional roll, pinned profile.
fn check_member_inputs(stage: &Stage, prim: &Prim, entry: &Json, findings: &mut Findings) {
    let path = &prim.path;
    let axis = prim.v3s("bim:axisPointsMm");
    let (Some(start), Some(end)) = (
        require_v3(entry, "axis_start", findings),
        require_v3(entry, "axis_end", findings),
    ) else {
        return;
    };
    findings.check(
        axis == vec![start, end],
        format!("{path}: axis is {:?}", axis),
    );
    match optional_float(entry, "roll") {
        Some(Some(roll)) => findings.check(
            prim.num("steel:member:rollRad") == Some(roll),
            format!("{path}: roll is {:?}", prim.num("steel:member:rollRad")),
        ),
        Some(None) => findings.check(
            !prim.has("steel:member:rollRad"),
            format!("{path}: an unexpected roll is authored"),
        ),
        None => (),
    }
    let Some(profile) = expect_text(entry, "profile", findings) else {
        return;
    };
    let target = prim.text("bim:profile").map(str::to_owned);
    let want = format!("/World/Definitions/Profiles/PRF_{profile}");
    findings.check(
        target.as_deref() == Some(want.as_str()),
        format!("{path}: bim:profile targets {target:?}"),
    );
    let Some(definition) = stage.prim(&want) else {
        findings.fail(format!("{want}: the pinned profile definition is missing"));
        return;
    };
    findings.check(
        definition.text("bim:definitionKind") == Some("rectangle"),
        format!(
            "{want}: definitionKind is {:?}",
            definition.text("bim:definitionKind")
        ),
    );
    findings.check(
        definition.text("bim:units") == Some("mm"),
        format!("{want}: units is {:?}", definition.text("bim:units")),
    );
    if let (Some(width), Some(want_width)) = (
        definition.num("steel:profile:widthMm"),
        expect_float(entry, "width", findings),
    ) {
        findings.close(&format!("{want}: widthMm"), width, want_width, 1e-9);
    }
    if let (Some(height), Some(want_height)) = (
        definition.num("steel:profile:heightMm"),
        expect_float(entry, "height", findings),
    ) {
        findings.close(&format!("{want}: heightMm"), height, want_height, 1e-9);
    }
}

/// Authored plate inputs: thickness, contour and hole arrays when expected.
fn check_plate_inputs(stage: &Stage, prim: &Prim, entry: &Json, findings: &mut Findings) {
    let _ = stage;
    let path = &prim.path;
    if let Some(want) = expect_float(entry, "thickness", findings) {
        findings.close(
            &format!("{path}: thicknessMm"),
            prim.num("steel:plate:thicknessMm").unwrap_or(f64::NAN),
            want,
            1e-9,
        );
    }
    let contour = prim.v3s("steel:plate:contourPointsMm");
    if let (Some(w), Some(l)) = (
        expect_float(entry, "width", findings),
        expect_float(entry, "length", findings),
    ) {
        let want = vec![
            V3::new(0.0, 0.0, 0.0),
            V3::new(w, 0.0, 0.0),
            V3::new(w, l, 0.0),
            V3::new(0.0, l, 0.0),
        ];
        findings.check(contour == want, format!("{path}: contour is {contour:?}"));
    }
    let Ok(holes) = entry.get("hole_counts").and_then(|h| h.array()) else {
        return;
    };
    let wanted: Vec<u64> = holes.iter().filter_map(|h| h.unsigned().ok()).collect();
    let counts: Vec<u64> = prim
        .nums("steel:plate:holeVertexCounts")
        .into_iter()
        .map(|c| c as u64)
        .collect();
    findings.check(
        counts == wanted,
        format!("{path}: hole counts are {counts:?}"),
    );
    let hole_points = prim.v3s("steel:plate:holePointsMm");
    findings.check(
        hole_points.len() == wanted.iter().sum::<u64>() as usize,
        format!("{path}: {} hole points", hole_points.len()),
    );
    findings.check(
        hole_points.iter().all(|p| p.z == 0.0),
        format!("{path}: a hole point is not in the XY plane"),
    );
}

/// One pinned immutable material definition, or no target at all (never a guessed default).
fn check_material(
    stage: &Stage,
    path: &str,
    material: &Json,
    target: Option<String>,
    volume: f64,
    entry: &Json,
    findings: &mut Findings,
) {
    let digest = material
        .get("digest")
        .ok()
        .and_then(|d| d.string().ok())
        .unwrap_or_default()
        .to_owned();
    let want = format!("/World/Definitions/Materials/MAT_{digest}");
    findings.check(
        target.as_deref() == Some(want.as_str()),
        format!("{path}: material targets {target:?}"),
    );
    let Some(definition) = stage.prim(&want) else {
        findings.fail(format!("{want}: the material definition is missing"));
        return;
    };
    findings.check(
        definition.text("bim:units") == Some("mm"),
        format!("{want}: units is {:?}", definition.text("bim:units")),
    );
    findings.check(
        definition.text("bim:schemaVersion") == Some("0.1.0"),
        format!(
            "{want}: schemaVersion is {:?}",
            definition.text("bim:schemaVersion")
        ),
    );
    findings.check(
        definition.text("bim:definitionKind") == Some("material"),
        format!(
            "{want}: definitionKind is {:?}",
            definition.text("bim:definitionKind")
        ),
    );
    findings.check(
        definition.text("bim:contentDigest") == Some(digest.as_str()),
        format!(
            "{want}: contentDigest is {:?}",
            definition.text("bim:contentDigest")
        ),
    );
    let density = definition.num("bim:densityKgM3");
    match material.get("density").ok().and_then(|d| d.float().ok()) {
        None => {
            findings.check(
                density.is_none(),
                format!("{want}: an unknown density has a value"),
            );
            findings.check(
                matches!(entry.get("cli_mass_kg"), Ok(Json::Null)),
                format!("{path}: the CLI reports a mass for an unknown density"),
            );
        }
        Some(want_density) => {
            findings.close(
                &format!("{want}: density"),
                density.unwrap_or(f64::NAN),
                want_density,
                1e-9,
            );
            if let Some(cli_mass) = expect_float(entry, "cli_mass_kg", findings) {
                let mass = volume * want_density / 1e9;
                findings.close_rel(&format!("{path}: derived mass"), mass, cli_mass, 1e-6);
            }
        }
    }
}

/// `/World` is the assembly, `/World/Steel` its group, as a plain reader composes them.
fn check_folder_kinds(stage: &Stage, findings: &mut Findings) {
    for (path, want) in [("/World", "assembly"), ("/World/Steel", "group")] {
        let got = stage.prim(path).and_then(|p| p.kind);
        findings.check(
            got.as_deref() == Some(want),
            format!("{path}: kind is {got:?}"),
        );
    }
}

/// U09 metadata of a derived mesh: authored extent and flat outward normals.
fn check_standard_metadata(
    mesh: &Prim,
    points: &[V3],
    counts: &[usize],
    indices: &[usize],
    findings: &mut Findings,
) {
    let where_ = &mesh.path;
    let extent = mesh.v3s("extent");
    findings.check(
        extent.len() == 2,
        format!("{where_}: the authored extent has {} corners", extent.len()),
    );
    if extent.len() == 2 {
        let (lo, hi) = bounds_of(points);
        for (corner, want, half) in [
            (extent.first().copied(), lo, "min"),
            (extent.get(1).copied(), hi, "max"),
        ] {
            if let Some(corner) = corner {
                for axis in ["x", "y", "z"] {
                    findings.close(
                        &format!("{where_}: extent.{half}.{axis}"),
                        pick(corner, axis),
                        pick(want, axis),
                        1e-3,
                    );
                }
            }
        }
    }
    let normals = mesh.v3s("normals");
    findings.check(
        !normals.is_empty() && normals.len() == counts.len(),
        format!(
            "{where_}: {} normals for {} faces",
            normals.len(),
            counts.len()
        ),
    );
    let mut cursor = 0usize;
    for (index, count) in counts.iter().enumerate() {
        let Some(end) = cursor.checked_add(*count) else {
            findings.fail(format!("{where_}: face {index} has an absurd count"));
            break;
        };
        let face: Vec<usize> = indices
            .get(cursor..end)
            .map(|window| window.to_vec())
            .unwrap_or_default();
        cursor = end;
        if face.len() != *count {
            findings.fail(format!("{where_}: face {index} reads past the index array"));
            break;
        }
        let Some(newell) = face_normal(points, &face) else {
            break;
        };
        if newell.norm() > 0.0 {
            let authored = normals
                .get(index)
                .copied()
                .unwrap_or(V3::new(0.0, 0.0, 0.0));
            let dot = authored.dot(newell.scale(1.0 / newell.norm()));
            findings.check(
                dot > 0.999,
                format!("{where_}: face {index} normal is not outward (dot {dot:?})"),
            );
        }
    }
}

// ---- The reference checks ---------------------------------------------------------------------

fn check_reference(stage: &Stage, expect: &Json, findings: &mut Findings) -> Json {
    let root = stage.root_layer();
    findings.check(
        root.meters_per_unit == Some(0.001),
        format!("metersPerUnit is {:?}", root.meters_per_unit),
    );
    findings.check(
        root.up_axis.as_deref() == Some("Z"),
        format!("upAxis is {:?}", root.up_axis),
    );
    let Some(id) = expect_text(expect, "id", findings) else {
        return reference_report(f64::NAN, "");
    };
    let id = id.to_owned();
    let name = expect_text(expect, "name", findings)
        .unwrap_or_default()
        .to_owned();
    let digest = expect_text(expect, "digest", findings)
        .unwrap_or_default()
        .to_owned();
    let wrapper_path = format!("/World/References/REF_{}", dashless(&id));
    let holder_path = format!("{wrapper_path}/Correction/Source");
    let wrapper = match stage.prim(&wrapper_path) {
        Some(wrapper) => wrapper,
        None => {
            findings.fail(format!("missing wrapper {wrapper_path}"));
            return reference_report(f64::NAN, "");
        }
    };
    findings.check(
        wrapper.text("bim:elementKind") == Some("reference"),
        format!(
            "{wrapper_path}: elementKind is {:?}",
            wrapper.text("bim:elementKind")
        ),
    );
    findings.check(
        wrapper.text("bim:id") == Some(id.as_str()),
        format!("{wrapper_path}: bim:id is {:?}", wrapper.text("bim:id")),
    );
    findings.check(
        wrapper.display_name.as_deref() == Some(name.as_str()),
        format!("{wrapper_path}: displayName is {:?}", wrapper.display_name),
    );
    findings.check(
        wrapper.text("bim:reference:sourceDigest") == Some(digest.as_str()),
        format!(
            "{wrapper_path}: sourceDigest is {:?}",
            wrapper.text("bim:reference:sourceDigest")
        ),
    );

    // The arc holder composes exactly the owned arc and the foreign root prim; no owned layer
    // holds any other opinion at or below it.
    let owned = stage.owned_specs(&holder_path);
    let arc = owned.iter().find(|s| s.reference.is_some());
    findings.check(
        owned.len() == 1 && arc.is_some(),
        format!(
            "{holder_path}: {} owned specs compose the holder",
            owned.len()
        ),
    );
    if let Some(spec) = arc {
        findings.check(
            spec.type_name.is_none() && spec.kind.is_none() && spec.attributes.is_empty(),
            format!("{holder_path}: the owned arc spec holds extra opinions"),
        );
    }
    let Some((asset, foreign)) = stage.foreign() else {
        findings.fail(format!("{holder_path}: no reference arc"));
        return reference_report(f64::NAN, "");
    };
    findings.check(
        foreign.up_axis.as_deref() == Some("Y"),
        format!("{asset}: the foreign up axis is {:?}", foreign.up_axis),
    );
    findings.check(
        foreign.meters_per_unit == Some(1.0),
        format!(
            "{asset}: the foreign metersPerUnit is {:?}",
            foreign.meters_per_unit
        ),
    );
    findings.check(
        foreign.default_prim.as_deref() == Some("Site"),
        format!(
            "{asset}: the foreign defaultPrim is {:?}",
            foreign.default_prim
        ),
    );
    findings.check(
        asset.starts_with("external/"),
        format!("{holder_path}: the arc asset {asset:?} is not external"),
    );

    // The foreign type shows through the holder.
    if let Some(holder) = stage.prim(&holder_path) {
        findings.check(
            holder.type_name.as_deref() == Some("Xform"),
            format!(
                "{holder_path}: the foreign root type is {:?}",
                holder.type_name
            ),
        );
    }
    // The note the source declares is read through the wrapper.
    let note_path = format!("{holder_path}/Block");
    if let Some(note_prim) = stage.prim(&note_path) {
        findings.check(
            note_prim.text("note") == Some("declared by the source"),
            format!(
                "{note_path}: the foreign attribute is {:?}",
                note_prim.text("note")
            ),
        );
    }

    // Placement and correction place the foreign points where the closed form says.
    let body_path = format!("{holder_path}/Block/Body");
    let Some(body) = stage.prim(&body_path) else {
        findings.fail(format!("missing foreign mesh {body_path}"));
        return reference_report(f64::NAN, &asset);
    };
    let local = body.v3s("points");
    let world = world_points(stage, &body_path, &local, findings);
    let mut worst = 0.0f64;
    if let Ok(wanted) = expect.get("world_points").and_then(|w| w.array()) {
        if wanted.len() != world.len() {
            findings.fail(format!("point count {} != {}", world.len(), wanted.len()));
        }
        for (got, want) in world.iter().zip(wanted) {
            if let Some(want) = json_v3(want) {
                let delta = (got.x - want.x)
                    .abs()
                    .max((got.y - want.y).abs())
                    .max((got.z - want.z).abs());
                worst = worst.max(delta);
            }
        }
        findings.check(worst <= 1e-6, format!("world points differ by {worst}"));
    }
    if let (Some(lo), Some(hi)) = (
        require_v3(expect, "world_min", findings),
        require_v3(expect, "world_max", findings),
    ) {
        let (min, max) = bounds_of(&world);
        for axis in ["x", "y", "z"] {
            findings.close(
                &format!("world_min.{axis}"),
                pick(min, axis),
                pick(lo, axis),
                1e-3,
            );
            findings.close(
                &format!("world_max.{axis}"),
                pick(max, axis),
                pick(hi, axis),
                1e-3,
            );
        }
    }
    reference_report(worst, &asset)
}

fn json_v3(value: &Json) -> Option<V3> {
    let array = value.array().ok()?;
    Some(V3::new(
        array.first()?.float().ok()?,
        array.get(1)?.float().ok()?,
        array.get(2)?.float().ok()?,
    ))
}

fn reference_report(worst: f64, asset: &str) -> Json {
    Json::object([
        ("worst_point_error_mm", report_number(worst)),
        ("foreign_layer", Json::text(asset)),
    ])
}

// ---- Self-checks ------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch layer directory, removed on drop.
    struct Layers(PathBuf);

    impl Layers {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "beamcraft-usd-oracle-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.0.join(name), text).unwrap();
        }
    }

    impl Drop for Layers {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_a_layer_and_composes_a_reference() {
        let layers = Layers::new("compose");
        layers.write(
            "root.usda",
            "#usda 1.0\n(\n    defaultPrim = \"World\"\n    metersPerUnit = 0.001\n    subLayers = [@authored.usda@, @derived.usda@]\n    upAxis = \"Z\"\n)\n",
        );
        layers.write(
            "derived.usda",
            "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n",
        );
        layers.write(
            "authored.usda",
            concat!(
                "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n",
                "def Xform \"World\"\n{\n",
                "    def Xform \"Ref\"\n    {\n",
                "        def \"Source\" (\n",
                "            prepend references = @foreign.usda@</Site>\n",
                "        )\n        {\n        }\n    }\n}\n",
            ),
        );
        layers.write(
            "foreign.usda",
            concat!(
                "#usda 1.0\n(\n    defaultPrim = \"Site\"\n    metersPerUnit = 1.0\n    upAxis = \"Y\"\n)\n",
                "def Xform \"Site\"\n{\n    def Mesh \"Body\"\n    {\n",
                "        point3f[] points = [(0, 0, 0), (2, 0, 0), (2, 1, 0), (0, 1, 0)]\n",
                "        int[] faceVertexCounts = [4]\n        int[] faceVertexIndices = [0, 1, 2, 3]\n",
                "    }\n}\n",
            ),
        );
        let stage = Stage::open(&layers.0.join("root.usda"));
        assert!(stage.take_problems().is_empty());
        // Resolving the holder bakes the foreign subtree into the stage.
        assert_eq!(
            stage.prim("/World/Ref/Source").and_then(|p| p.type_name),
            Some("Xform".to_owned()),
            "the foreign type shows through the holder"
        );
        let body = stage.prim("/World/Ref/Source/Body");
        let body = body.unwrap_or_default();
        assert_eq!(body.type_name.as_deref(), Some("Mesh"));
        assert_eq!(body.v3s("points").len(), 4);
    }

    #[test]
    fn rejects_structural_noise() {
        let layers = Layers::new("strict");
        layers.write("root.usda", "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\ndef Xform \"World\"\n{\n    frobnicate = 1\n}\n");
        assert!(Parser::layer(&layers.0.join("root.usda")).is_err());
        layers.write("bad.usda", "#usda 2.0\n");
        assert!(Parser::layer(&layers.0.join("bad.usda")).is_err());
    }

    #[test]
    fn volumes_and_quats_match_the_closed_forms() {
        let cube = [
            V3::new(0.0, 0.0, 0.0),
            V3::new(1.0, 0.0, 0.0),
            V3::new(0.0, 1.0, 0.0),
            V3::new(1.0, 1.0, 0.0),
            V3::new(0.0, 0.0, 1.0),
            V3::new(1.0, 0.0, 1.0),
            V3::new(0.0, 1.0, 1.0),
            V3::new(1.0, 1.0, 1.0),
        ];
        // The writer's exact triangle order for a unit box, outward oriented.
        let counts = vec![3; 12];
        let indices: Vec<usize> = vec![
            0, 4, 6, 0, 6, 2, 1, 3, 7, 1, 7, 5, 0, 1, 5, 0, 5, 4, 2, 6, 7, 2, 7, 3, 0, 2, 3, 0, 3,
            1, 4, 5, 7, 4, 7, 6,
        ];
        let volume = signed_volume(&cube, &counts, &indices).unwrap();
        assert!((volume - 1.0).abs() < 1e-12, "{volume:?}");
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let quarter = Quat {
            s: half,
            x: 0.0,
            y: 0.0,
            z: half,
        };
        let turned = quarter.rotate(V3::new(2000.0, -3000.0, 1000.0));
        assert!((turned.x - 3000.0).abs() < 1e-9, "{turned:?}");
        assert!((turned.y - 2000.0).abs() < 1e-9);
        assert!((turned.z - 1000.0).abs() < 1e-9);
    }
}
