// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A structural reader for the files this crate writes (E01 section 15.2, "tests").
//!
//! This is the harness of the acceptance cases, not a product feature (`IFCX-SCOPE-003`): it
//! parses the lexical form of an IFC4 STEP file, checks that the instances are numbered without
//! gaps, that every reference resolves, that every GlobalId is valid and unique, that no required
//! list is empty, and recomputes the volume and the closed-shell property of every tessellated
//! face set from the file alone (`IFCX-GEO-008`). Nothing builds elements from what it reads.

use crate::guid;
use std::collections::{BTreeMap, BTreeSet};

/// Largest list nesting the reader accepts.
const MAX_DEPTH: usize = 16;

/// A parsed attribute value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `$`, an unset attribute.
    Null,
    /// `*`, a derived attribute.
    Derived,
    /// An INTEGER.
    Int(i64),
    /// A REAL.
    Real(f64),
    /// A decoded string.
    Str(String),
    /// An enumeration literal without the dots.
    Enum(String),
    /// An instance reference.
    Ref(u64),
    /// A list.
    List(Vec<Value>),
    /// A typed value such as `IFCTEXT('a')`.
    Typed(String, Vec<Value>),
}

/// One `#n=NAME(...)` instance.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// The instance number.
    pub id: u64,
    /// The entity name in upper case.
    pub name: String,
    /// The attributes.
    pub args: Vec<Value>,
}

/// A parsed file.
#[derive(Debug, Clone, PartialEq)]
pub struct StepFile {
    /// The raw header section, without its framing lines.
    pub header: Vec<String>,
    /// The instances in file order.
    pub instances: Vec<Instance>,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyError(pub String);

impl core::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for VerifyError {}

fn fail<T>(message: impl Into<String>) -> Result<T, VerifyError> {
    Err(VerifyError(message.into()))
}

/// The result of a structural check.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// Instances per entity name.
    pub counts: BTreeMap<String, usize>,
    /// Every GlobalId of the file in file order.
    pub global_ids: Vec<String>,
    /// Per face-set instance: recomputed volume in the file's length unit cubed.
    pub volumes: BTreeMap<u64, f64>,
}

/// Decodes the STEP string escapes `''`, `\\`, `\X2\...\X0\` and `\X4\...\X0\`.
pub fn unescape(raw: &str) -> Result<String, VerifyError> {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        match c {
            '\'' => {
                if chars.get(i + 1) != Some(&'\'') {
                    return fail("a lone apostrophe inside a string");
                }
                out.push('\'');
                i += 2;
            }
            '\\' => {
                if chars.get(i + 1) == Some(&'\\') {
                    out.push('\\');
                    i += 2;
                    continue;
                }
                let wide = match chars.get(i..i + 4) {
                    Some(['\\', 'X', '2', '\\']) => false,
                    Some(['\\', 'X', '4', '\\']) => true,
                    _ => return fail("an unknown escape"),
                };
                i += 4;
                let width = if wide { 8 } else { 4 };
                let mut units: Vec<u16> = Vec::new();
                loop {
                    if chars.get(i..i + 4) == Some(&['\\', 'X', '0', '\\']) {
                        i += 4;
                        break;
                    }
                    let hex: String = match chars.get(i..i + width) {
                        Some(slice) => slice.iter().collect(),
                        None => return fail("an unterminated escape"),
                    };
                    let value = u32::from_str_radix(&hex, 16)
                        .map_err(|_| VerifyError("a bad escape digit".into()))?;
                    if wide {
                        out.push(
                            char::from_u32(value).ok_or(VerifyError("a bad code point".into()))?,
                        );
                    } else {
                        units.push(
                            u16::try_from(value).map_err(|_| VerifyError("a bad unit".into()))?,
                        );
                    }
                    i += width;
                }
                for decoded in char::decode_utf16(units) {
                    out.push(decoded.map_err(|_| VerifyError("a bad surrogate".into()))?);
                }
            }
            plain => {
                if !(' '..='~').contains(&plain) {
                    return fail("a non-ASCII character in a string");
                }
                out.push(plain);
                i += 1;
            }
        }
    }
    Ok(out)
}

fn parse_args(text: &str) -> Result<(String, Vec<Value>), VerifyError> {
    let bytes = text.as_bytes();
    let name_end = text
        .find('(')
        .ok_or(VerifyError("an instance without arguments".into()))?;
    let name = text.get(..name_end).unwrap_or_default().to_owned();
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return fail(format!("a bad entity name {name:?}"));
    }
    let mut stack: Vec<(Option<String>, Vec<Value>)> = vec![(Some(name.clone()), Vec::new())];
    let mut i = name_end + 1;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'(' => {
                if stack.len() >= MAX_DEPTH {
                    return fail("nesting is too deep");
                }
                stack.push((None, Vec::new()));
                i += 1;
            }
            b')' => {
                let (typed, items) = stack
                    .pop()
                    .ok_or(VerifyError("unbalanced parentheses".into()))?;
                match stack.last_mut() {
                    Some(parent) => parent.1.push(match typed {
                        Some(t) => Value::Typed(t, items),
                        None => Value::List(items),
                    }),
                    None => {
                        if i + 1 != bytes.len() {
                            return fail("text after the closing parenthesis");
                        }
                        return Ok((name, items));
                    }
                }
                i += 1;
            }
            b',' => i += 1,
            b'\'' => {
                let start = i + 1;
                let mut j = start;
                loop {
                    match bytes.get(j) {
                        Some(b'\'') if bytes.get(j + 1) == Some(&b'\'') => j += 2,
                        Some(b'\'') => break,
                        Some(_) => j += 1,
                        None => return fail("an unterminated string"),
                    }
                }
                let raw = text
                    .get(start..j)
                    .ok_or(VerifyError("a split character".into()))?;
                push(&mut stack, Value::Str(unescape(raw)?))?;
                i = j + 1;
            }
            b'#' => {
                let start = i + 1;
                let mut j = start;
                while bytes.get(j).is_some_and(u8::is_ascii_digit) {
                    j += 1;
                }
                let n = text.get(start..j).unwrap_or_default().parse::<u64>();
                push(
                    &mut stack,
                    Value::Ref(n.map_err(|_| VerifyError("a bad reference".into()))?),
                )?;
                i = j;
            }
            b'$' => {
                push(&mut stack, Value::Null)?;
                i += 1;
            }
            b'*' => {
                push(&mut stack, Value::Derived)?;
                i += 1;
            }
            b'.' => {
                let rest = text.get(i + 1..).unwrap_or_default();
                let end = rest
                    .find('.')
                    .ok_or(VerifyError("an unterminated enumeration".into()))?;
                push(
                    &mut stack,
                    Value::Enum(rest.get(..end).unwrap_or_default().to_owned()),
                )?;
                i += end + 2;
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                let mut j = i;
                while bytes
                    .get(j)
                    .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'E'))
                {
                    j += 1;
                }
                let number = text.get(start..j).unwrap_or_default();
                let value = if number.contains(['.', 'E']) {
                    Value::Real(
                        number
                            .parse::<f64>()
                            .map_err(|_| VerifyError(format!("a bad REAL {number:?}")))?,
                    )
                } else {
                    Value::Int(
                        number
                            .parse::<i64>()
                            .map_err(|_| VerifyError(format!("a bad INTEGER {number:?}")))?,
                    )
                };
                push(&mut stack, value)?;
                i = j;
            }
            b'A'..=b'Z' => {
                let start = i;
                let mut j = i;
                while bytes
                    .get(j)
                    .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                {
                    j += 1;
                }
                if bytes.get(j) != Some(&b'(') {
                    return fail("a bare word");
                }
                if stack.len() >= MAX_DEPTH {
                    return fail("nesting is too deep");
                }
                stack.push((
                    Some(text.get(start..j).unwrap_or_default().to_owned()),
                    Vec::new(),
                ));
                i = j + 1;
            }
            other => return fail(format!("an unexpected character {:?}", other as char)),
        }
    }
    fail("an unterminated instance")
}

fn push(stack: &mut [(Option<String>, Vec<Value>)], value: Value) -> Result<(), VerifyError> {
    stack
        .last_mut()
        .ok_or(VerifyError("a value outside a list".into()))?
        .1
        .push(value);
    Ok(())
}

/// Parses a whole file. The file must be 7-bit ASCII with LF line ends.
pub fn parse(bytes: &[u8]) -> Result<StepFile, VerifyError> {
    if !bytes.is_ascii() {
        return fail("the file is not 7-bit ASCII");
    }
    if bytes.contains(&b'\r') {
        return fail("the file has a CR");
    }
    let text = core::str::from_utf8(bytes).map_err(|_| VerifyError("not UTF-8".into()))?;
    let mut lines = text.split('\n');
    if lines.next() != Some("ISO-10303-21;") || lines.next() != Some("HEADER;") {
        return fail("the file does not start with a STEP header");
    }
    let mut header = Vec::new();
    loop {
        match lines.next() {
            Some("ENDSEC;") => break,
            Some(line) => header.push(line.to_owned()),
            None => return fail("an unterminated header"),
        }
    }
    if lines.next() != Some("DATA;") {
        return fail("no DATA section");
    }
    let mut instances = Vec::new();
    loop {
        match lines.next() {
            Some("ENDSEC;") => break,
            Some(line) => {
                let body = line
                    .strip_prefix('#')
                    .ok_or(VerifyError("a line that is no instance".into()))?;
                let (number, rest) = body
                    .split_once('=')
                    .ok_or(VerifyError("no equals sign".into()))?;
                let rest = rest
                    .strip_suffix(';')
                    .ok_or(VerifyError("no semicolon".into()))?;
                let (name, args) = parse_args(rest)?;
                instances.push(Instance {
                    id: number
                        .parse::<u64>()
                        .map_err(|_| VerifyError("a bad number".into()))?,
                    name,
                    args,
                });
            }
            None => return fail("an unterminated DATA section"),
        }
    }
    if lines.next() != Some("END-ISO-10303-21;")
        || lines.next() != Some("")
        || lines.next().is_some()
    {
        return fail("the file does not end with END-ISO-10303-21; and one LF");
    }
    Ok(StepFile { header, instances })
}

impl StepFile {
    /// The instance with this number.
    pub fn get(&self, id: u64) -> Option<&Instance> {
        // Numbering is gapless (checked by `check`), so the instance is at id - 1.
        let at = usize::try_from(id.checked_sub(1)?).ok()?;
        self.instances.get(at).filter(|i| i.id == id)
    }

    /// Every instance of an entity, in file order.
    pub fn all(&self, name: &str) -> Vec<&Instance> {
        self.instances.iter().filter(|i| i.name == name).collect()
    }

    /// The referenced instance of an attribute value.
    pub fn target(&self, value: &Value) -> Option<&Instance> {
        match value {
            Value::Ref(n) => self.get(*n),
            _ => None,
        }
    }

    /// The text of a `Value::Str`.
    pub fn text<'a>(&self, value: Option<&'a Value>) -> Option<&'a str> {
        match value {
            Some(Value::Str(s)) => Some(s),
            _ => None,
        }
    }

    /// The point list of a face set as `f64` triples.
    fn points(&self, set: &Instance) -> Result<Vec<[f64; 3]>, VerifyError> {
        let list = set
            .args
            .first()
            .and_then(|v| self.target(v))
            .ok_or(VerifyError("no point list".into()))?;
        let Some(Value::List(rows)) = list.args.first() else {
            return fail("a point list without coordinates");
        };
        rows.iter()
            .map(|row| match row {
                Value::List(c) => match c.as_slice() {
                    [Value::Real(x), Value::Real(y), Value::Real(z)] => Ok([*x, *y, *z]),
                    _ => fail("a coordinate is not three REALs"),
                },
                _ => fail("a coordinate is not a list"),
            })
            .collect()
    }

    /// Checks the whole file and recomputes the volume of every face set.
    pub fn check(&self) -> Result<Summary, VerifyError> {
        if self.header.last().map(String::as_str) != Some("FILE_SCHEMA(('IFC4'));") {
            return fail("the schema is not IFC4");
        }
        for (i, instance) in self.instances.iter().enumerate() {
            if instance.id != i as u64 + 1 {
                return fail(format!("instance numbers have a gap at #{}", instance.id));
            }
        }
        let mut summary = Summary {
            counts: BTreeMap::new(),
            global_ids: Vec::new(),
            volumes: BTreeMap::new(),
        };
        let mut seen = BTreeSet::new();
        for instance in &self.instances {
            *summary.counts.entry(instance.name.clone()).or_default() += 1;
            let mut pending: Vec<&Value> = instance.args.iter().collect();
            while let Some(value) = pending.pop() {
                match value {
                    Value::Ref(n) if self.get(*n).is_none() => {
                        return fail(format!("#{} refers to missing #{n}", instance.id));
                    }
                    Value::List(items) | Value::Typed(_, items) => pending.extend(items),
                    _ => {}
                }
            }
            let rooted = !matches!(
                instance.name.as_str(),
                "IFCCARTESIANPOINT" | "IFCDIRECTION" | "IFCAXIS2PLACEMENT3D" | "IFCLOCALPLACEMENT"
            ) && (instance.name.starts_with("IFCREL")
                || matches!(
                    instance.name.as_str(),
                    "IFCPROJECT"
                        | "IFCSITE"
                        | "IFCBUILDING"
                        | "IFCBUILDINGSTOREY"
                        | "IFCBEAM"
                        | "IFCCOLUMN"
                        | "IFCPLATE"
                        | "IFCBUILDINGELEMENTPROXY"
                        | "IFCPROPERTYSET"
                        | "IFCELEMENTQUANTITY"
                ));
            if rooted {
                let Some(Value::Str(id)) = instance.args.first() else {
                    return fail(format!("#{} has no GlobalId", instance.id));
                };
                guid::decode(id).map_err(|_| {
                    VerifyError(format!("#{} has an invalid GlobalId", instance.id))
                })?;
                if !seen.insert(id.clone()) {
                    return fail(format!("GlobalId {id} occurs twice"));
                }
                summary.global_ids.push(id.clone());
            }
            if instance.name.starts_with("IFCREL") {
                for related in &instance.args {
                    if matches!(related, Value::List(items) if items.is_empty()) {
                        return fail(format!("#{} relates an empty set", instance.id));
                    }
                }
            }
            if instance.name == "IFCTRIANGULATEDFACESET" {
                let volume = self.face_set(instance)?;
                summary.volumes.insert(instance.id, volume);
            }
        }
        Ok(summary)
    }

    /// Volume by the divergence sum and, for a set marked closed, the closed-shell property.
    fn face_set(&self, set: &Instance) -> Result<f64, VerifyError> {
        let points = self.points(set)?;
        let Some(Value::List(rows)) = set.args.get(3) else {
            return fail("a face set without triangles");
        };
        let closed = matches!(set.args.get(2), Some(Value::Enum(e)) if e == "T");
        let mut edges: BTreeMap<(i64, i64), u32> = BTreeMap::new();
        let mut sum = 0.0_f64;
        for row in rows {
            let Value::List(v) = row else {
                return fail("a triangle is not a list");
            };
            let [Value::Int(a), Value::Int(b), Value::Int(c)] = v.as_slice() else {
                return fail("a triangle is not three INTEGERs");
            };
            let at = |index: i64| -> Result<[f64; 3], VerifyError> {
                let zero_based =
                    usize::try_from(index.checked_sub(1).ok_or(VerifyError("index 0".into()))?)
                        .map_err(|_| VerifyError("a bad index".into()))?;
                points
                    .get(zero_based)
                    .copied()
                    .ok_or(VerifyError("an index out of range".into()))
            };
            let (p, q, r) = (at(*a)?, at(*b)?, at(*c)?);
            let cross = [
                q[1] * r[2] - q[2] * r[1],
                q[2] * r[0] - q[0] * r[2],
                q[0] * r[1] - q[1] * r[0],
            ];
            sum += p[0] * cross[0] + p[1] * cross[1] + p[2] * cross[2];
            for edge in [(*a, *b), (*b, *c), (*c, *a)] {
                *edges.entry(edge).or_default() += 1;
            }
        }
        if closed {
            for (&(a, b), &n) in &edges {
                if n != 1 || edges.get(&(b, a)) != Some(&1) {
                    return fail(
                        "a closed face set whose edges are not paired once in each direction",
                    );
                }
            }
        }
        Ok(sum / 6.0)
    }
}
