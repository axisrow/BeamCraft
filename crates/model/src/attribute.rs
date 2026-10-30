// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! I04 user attributes: typed definitions and the values elements carry.
//!
//! A definition (name, type, optional unit, choice options, optional set) lives in the model's
//! attribute schema; a value is a `bim:attr:<name>` property of an element. A value may outlive
//! its definition (an undefine never touches values): such an unknown attribute stays stored and
//! is shown, never dropped. Admission here checks only the argument forms; whether a name is
//! free, a definition exists and a value matches its definition are the engine's decisions.

use crate::command::{CommandError, label_is_valid};
use crate::digest::{ContentDigest, Sha256};
use crate::error::ErrorCode;
use crate::id::ElementId;
use crate::limits::{
    MAX_ATTR_NAME_BYTES, MAX_ATTR_NUMBER, MAX_ATTR_OPTIONS, MAX_ATTR_SET_BYTES, MAX_ATTR_UNIT_BYTES,
};
use beamcraft_geom::canonical_f64_bytes;

/// The stored type of a user attribute (I04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrType {
    /// Free text, 1 to 256 bytes without control characters.
    Text,
    /// A finite number with the magnitude cap.
    Number,
    /// A 32-bit signed integer.
    Integer,
    /// True or false.
    Boolean,
    /// One of the definition's options.
    Choice,
}

impl AttrType {
    /// The stored token of the type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Choice => "choice",
        }
    }

    /// Parses the stored token of the type.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "text" => Some(Self::Text),
            "number" => Some(Self::Number),
            "integer" => Some(Self::Integer),
            "boolean" => Some(Self::Boolean),
            "choice" => Some(Self::Choice),
            _ => None,
        }
    }

    /// The USD attribute type a value of this attribute is stored as.
    pub const fn usd_type(self) -> &'static str {
        match self {
            Self::Text => "string",
            Self::Number => "double",
            Self::Integer => "int",
            Self::Boolean => "bool",
            Self::Choice => "string",
        }
    }
}

/// The value of one user attribute on one element.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    /// Free text.
    Text(String),
    /// A number.
    Number(f64),
    /// An integer.
    Integer(i32),
    /// A boolean.
    Boolean(bool),
    /// A choice option.
    Choice(String),
}

impl AttrValue {
    /// The type a value of this shape carries.
    pub fn attr_type(&self) -> AttrType {
        match self {
            Self::Text(_) => AttrType::Text,
            Self::Choice(_) => AttrType::Choice,
            Self::Number(_) => AttrType::Number,
            Self::Integer(_) => AttrType::Integer,
            Self::Boolean(_) => AttrType::Boolean,
        }
    }

    /// The stored text of a text or choice value.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(t) | Self::Choice(t) => Some(t),
            _ => None,
        }
    }

    /// Admits the value's own form: text and choice are label-bounded, a number is finite within
    /// the cap. Whether the value matches its attribute's definition is the engine's decision.
    pub fn validate(&self) -> Result<(), CommandError> {
        match self {
            Self::Text(t) | Self::Choice(t) => {
                if !label_is_valid(t) {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        "value",
                        "text must be 1 to 256 bytes without control characters",
                    )
                    .with_rule("DV-BASE-003"));
                }
                Ok(())
            }
            Self::Number(n) => {
                if !n.is_finite() {
                    return Err(CommandError::new(
                        ErrorCode::NonFiniteNumber,
                        "value",
                        "value is not finite",
                    )
                    .with_rule("DV-ATTR-003"));
                }
                if n.abs() > MAX_ATTR_NUMBER {
                    return Err(CommandError::new(
                        ErrorCode::LimitExceeded,
                        "value",
                        "number exceeds the attribute value cap",
                    )
                    .with_rule("DV-LIM-001"));
                }
                Ok(())
            }
            Self::Integer(_) | Self::Boolean(_) => Ok(()),
        }
    }
}

/// Whether `name` is a valid attribute name: a USD identifier of 1 to 64 bytes, so the definition
/// prim and the element property carry it verbatim.
pub fn attr_name_is_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_ATTR_NAME_BYTES
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One definition of the attribute schema: a name, a type, an optional unit, the options of a
/// choice and an optional set. Units are labels of the stored value's unit; nothing converts.
#[derive(Debug, Clone, PartialEq)]
pub struct AttrDef {
    /// The attribute name (a USD identifier).
    pub name: String,
    /// The stored type.
    pub ty: AttrType,
    /// The unit the value is stored in, when declared.
    pub unit: Option<String>,
    /// The admitted options, exactly for a choice and empty otherwise.
    pub options: Vec<String>,
    /// The attribute set this definition belongs to, when any.
    pub set: Option<String>,
}

impl AttrDef {
    /// Admits the definition's own form. Whether the name is free is the engine's decision.
    pub fn validate(&self) -> Result<(), CommandError> {
        if !attr_name_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "an attribute name is 1 to 64 ASCII letters, digits or underscores",
            ));
        }
        if let Some(unit) = &self.unit {
            short_text_unit(unit)?;
        }
        match self.ty {
            AttrType::Choice => {
                if self.options.is_empty() || self.options.len() > MAX_ATTR_OPTIONS {
                    return Err(CommandError::new(
                        ErrorCode::InvalidArgument,
                        "options",
                        "a choice declares 1 to 64 options",
                    ));
                }
                let mut seen: Vec<&str> = Vec::new();
                for option in &self.options {
                    if !label_is_valid(option) {
                        return Err(CommandError::new(
                            ErrorCode::InvalidArgument,
                            "options",
                            "an option must be 1 to 256 bytes without control characters",
                        )
                        .with_rule("DV-BASE-003"));
                    }
                    if seen.contains(&option.as_str()) {
                        return Err(CommandError::new(
                            ErrorCode::InvalidArgument,
                            "options",
                            "duplicate option",
                        ));
                    }
                    seen.push(option.as_str());
                }
            }
            _ if !self.options.is_empty() => {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "options",
                    "only a choice carries options",
                ));
            }
            _ => {}
        }
        if let Some(set) = &self.set
            && (set.is_empty() || set.len() > MAX_ATTR_SET_BYTES)
        {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "set",
                "a set name is 1 to 64 bytes",
            ));
        }
        Ok(())
    }
}

/// A unit label: 1 to 16 bytes without control characters; it is stored verbatim.
fn short_text_unit(unit: &str) -> Result<(), CommandError> {
    if unit.is_empty()
        || unit.len() > MAX_ATTR_UNIT_BYTES
        || unit.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
    {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "unit",
            "a unit is 1 to 16 bytes without control characters",
        ));
    }
    Ok(())
}

/// The one refusal of a malformed attribute name.
fn check_name(name: &str) -> Result<(), CommandError> {
    if attr_name_is_valid(name) {
        Ok(())
    } else {
        Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "name",
            "an attribute name is 1 to 64 ASCII letters, digits or underscores",
        ))
    }
}

/// SHA-256 of a versioned, length-prefixed encoding of validated arguments.
fn fingerprint(
    tag: &[u8],
    build: impl FnOnce(&mut Vec<u8>) -> Result<(), CommandError>,
) -> Result<ContentDigest, CommandError> {
    let mut bytes = Vec::with_capacity(96);
    bytes.extend_from_slice(tag);
    bytes.push(0);
    build(&mut bytes)?;
    Sha256::digest(&bytes).map_err(|_| too_large())
}

/// I04 `DefineAttribute`: adds one definition to the schema.
#[derive(Debug, Clone, PartialEq)]
pub struct DefineAttribute {
    /// The definition.
    pub def: AttrDef,
}

impl DefineAttribute {
    /// Admits the definition; the value bounds are the engine's decisions.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        self.def.validate()
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        fingerprint(b"BeamCraft/I04/DefineAttribute/v1", |bytes| {
            push_text(bytes, &self.def.name);
            bytes.extend_from_slice(self.def.ty.as_str().as_bytes());
            bytes.push(u8::from(self.def.unit.is_some()));
            if let Some(unit) = &self.def.unit {
                push_text(bytes, unit);
            }
            bytes.extend_from_slice(&(self.def.options.len() as u64).to_le_bytes());
            for option in &self.def.options {
                push_text(bytes, option);
            }
            bytes.push(u8::from(self.def.set.is_some()));
            if let Some(set) = &self.def.set {
                push_text(bytes, set);
            }
            Ok(())
        })
    }
}

/// I04 `UndefineAttribute`: removes one definition; stored values are untouched.
#[derive(Debug, Clone, PartialEq)]
pub struct UndefineAttribute {
    /// The attribute name.
    pub name: String,
}

impl UndefineAttribute {
    /// Admits the name form; whether the definition exists is the engine's decision.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        check_name(&self.name)
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        fingerprint(b"BeamCraft/I04/UndefineAttribute/v1", |bytes| {
            push_text(bytes, &self.name);
            Ok(())
        })
    }
}

/// I04 `SetAttribute`: stores one value of one defined attribute on one element.
#[derive(Debug, Clone, PartialEq)]
pub struct SetAttribute {
    /// The element.
    pub target_id: ElementId,
    /// The attribute name.
    pub name: String,
    /// The value.
    pub value: AttrValue,
}

impl SetAttribute {
    /// Admits the name form and the value's own form; the definition and its type are the
    /// engine's decisions.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        check_name(&self.name)?;
        self.value.validate()
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        fingerprint(b"BeamCraft/I04/SetAttribute/v1", |bytes| {
            bytes.extend_from_slice(self.target_id.as_bytes());
            push_text(bytes, &self.name);
            match &self.value {
                AttrValue::Text(t) | AttrValue::Choice(t) => {
                    bytes.push(u8::from(matches!(&self.value, AttrValue::Choice(_))));
                    push_text(bytes, t);
                }
                AttrValue::Number(n) => {
                    bytes.push(b'n');
                    bytes.extend_from_slice(&canonical_f64_bytes(*n).map_err(|_| too_large())?);
                }
                AttrValue::Integer(i) => {
                    bytes.push(b'i');
                    bytes.extend_from_slice(&i.to_le_bytes());
                }
                AttrValue::Boolean(b) => bytes.push(if *b { b't' } else { b'f' }),
            }
            Ok(())
        })
    }
}

/// I04 `ClearAttribute`: removes one stored value; an unknown attribute clears by name too.
#[derive(Debug, Clone, PartialEq)]
pub struct ClearAttribute {
    /// The element.
    pub target_id: ElementId,
    /// The attribute name.
    pub name: String,
}

impl ClearAttribute {
    /// Admits the name form; whether a value is present is the engine's decision.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        check_name(&self.name)
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        fingerprint(b"BeamCraft/I04/ClearAttribute/v1", |bytes| {
            bytes.extend_from_slice(self.target_id.as_bytes());
            push_text(bytes, &self.name);
            Ok(())
        })
    }
}

/// Appends a length-prefixed UTF-8 string to a fingerprint buffer.
fn push_text(bytes: &mut Vec<u8>, text: &str) {
    bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
    bytes.extend_from_slice(text.as_bytes());
}

fn too_large() -> CommandError {
    CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
        .with_rule("DV-LIM-001")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str, ty: AttrType) -> AttrDef {
        AttrDef {
            name: name.into(),
            ty,
            unit: None,
            options: Vec::new(),
            set: None,
        }
    }

    #[test]
    fn names_follow_the_usd_identifier_rule() {
        for good in ["a", "Z", "_x", "CoatWeight_2", &"a".repeat(64)] {
            assert!(attr_name_is_valid(good), "{good}");
        }
        for bad in ["", "9x", "-x", "a b", "a.b", "x\x7f", &"a".repeat(65)] {
            assert!(!attr_name_is_valid(bad), "{bad}");
        }
    }

    #[test]
    fn definitions_admit_their_form_and_refuse_the_rest() {
        let mut choice = def("Finish", AttrType::Choice);
        assert!(choice.validate().is_err());
        choice.options = vec!["plain".into(), "galvanized".into()];
        assert!(choice.validate().is_ok());
        let mut duplicate = choice.clone();
        duplicate.options[1] = "plain".into();
        assert!(duplicate.validate().is_err());
        assert!(def("X", AttrType::Text).validate().is_ok());
        let mut stray = def("X", AttrType::Number);
        stray.options = vec!["one".into()];
        assert!(stray.validate().is_err());
        let mut unit = def("Len", AttrType::Number);
        unit.unit = Some("mm".into());
        assert!(unit.validate().is_ok());
        unit.unit = Some(String::new());
        assert!(unit.validate().is_err());
        unit.unit = Some("m\u{1}m".into());
        assert!(unit.validate().is_err());
        let mut set = def("S", AttrType::Text);
        set.set = Some(String::new());
        assert!(set.validate().is_err());
        let mut bad_name = def("9x", AttrType::Text);
        assert!(bad_name.validate().is_err());
        bad_name.name = "ok".into();
        assert!(bad_name.validate().is_ok());
    }

    #[test]
    fn values_admit_their_own_form() {
        assert!(AttrValue::Text("hot dip".into()).validate().is_ok());
        assert!(AttrValue::Text(String::new()).validate().is_err());
        assert!(AttrValue::Text("x\x7f".into()).validate().is_err());
        assert!(AttrValue::Text("a".repeat(257)).validate().is_err());
        assert!(AttrValue::Number(1.5).validate().is_ok());
        assert!(AttrValue::Number(f64::NAN).validate().is_err());
        assert!(AttrValue::Number(1.0e10).validate().is_err());
        assert!(AttrValue::Integer(i32::MIN).validate().is_ok());
        assert!(AttrValue::Boolean(true).validate().is_ok());
        assert!(AttrValue::Choice("plain".into()).validate().is_ok());
    }

    #[test]
    fn fingerprints_separate_commands_and_values() {
        let a = DefineAttribute {
            def: def("A", AttrType::Text),
        };
        let mut b = a.clone();
        b.def.ty = AttrType::Number;
        assert_ne!(
            a.content_fingerprint().unwrap(),
            b.content_fingerprint().unwrap()
        );
        let mut unit = a.clone();
        unit.def.unit = Some("mm".into());
        assert_ne!(
            a.content_fingerprint().unwrap(),
            unit.content_fingerprint().unwrap()
        );
        let set = |v: AttrValue| SetAttribute {
            target_id: ElementId::parse("019a7bf0-0000-7000-8000-000000400001").unwrap(),
            name: "A".into(),
            value: v,
        };
        let one = set(AttrValue::Text("x".into()));
        let choice = set(AttrValue::Choice("x".into()));
        assert_ne!(
            one.content_fingerprint().unwrap(),
            choice.content_fingerprint().unwrap()
        );
        assert_ne!(
            set(AttrValue::Number(1.0)).content_fingerprint().unwrap(),
            set(AttrValue::Integer(1)).content_fingerprint().unwrap()
        );
    }
}
