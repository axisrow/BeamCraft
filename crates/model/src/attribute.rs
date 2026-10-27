// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! I04 typed user attributes; attribute values never feed engineering shape keys.
use crate::{
    command::CommandError,
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
    id::ElementId,
};
use std::collections::{BTreeMap, BTreeSet};
/// Maximum definitions in one model.
pub const MAX_DEFINITIONS: usize = 256;
/// Maximum values on one element.
pub const MAX_VALUES: usize = 64;
/// Supported user-property types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeType {
    /// UTF-8 text.
    Text,
    /// Finite number in the definition's unit.
    Number,
    /// Exact signed integer.
    Integer,
    /// Boolean.
    Boolean,
    /// One declared text choice.
    Choice,
}
impl AttributeType {
    /// Stable definition token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Choice => "choice",
        }
    }
    /// Parse an exact type token.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "text" => Some(Self::Text),
            "number" => Some(Self::Number),
            "integer" => Some(Self::Integer),
            "boolean" => Some(Self::Boolean),
            "choice" => Some(Self::Choice),
            _ => None,
        }
    }
}
/// A scalar value; text also represents a choice after validation against its definition.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeValue {
    /// UTF-8 text, at most 256 bytes.
    Text(String),
    /// Finite number, absolute value at most 1e9 in this static USD profile.
    Number(f64),
    /// Exact signed integer.
    Integer(i64),
    /// Boolean.
    Boolean(bool),
}
/// Immutable user-property definition. Sets are bounded labels used to group properties.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeDefinition {
    /// Persistent UUID.
    pub id: ElementId,
    /// Unique case-sensitive UTF-8 label (1..256 bytes).
    pub name: String,
    /// Value type.
    pub kind: AttributeType,
    /// Numeric unit label; no implicit conversion is performed.
    pub unit: String,
    /// Nonempty allowed choice list only for choice definitions.
    pub choices: Vec<String>,
    /// Membership in named attribute sets.
    pub sets: Vec<String>,
}
fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "attribute", message)
}
impl AttributeValue {
    /// Check scalar text/numeric bounds before allocating persisted values.
    pub fn validate(&self) -> Result<(), CommandError> {
        match self {
            Self::Text(s) => {
                if s.len() > 256 || s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                    return Err(invalid("attribute text exceeds the bounded text profile"));
                }
            }
            Self::Number(n) => {
                if !n.is_finite() {
                    return Err(CommandError::new(
                        ErrorCode::NonFiniteNumber,
                        "attribute",
                        "attribute number must be finite",
                    ));
                }
                if n.abs() > 1e9 {
                    return Err(CommandError::new(
                        ErrorCode::LimitExceeded,
                        "attribute",
                        "attribute number exceeds the static USD scalar cap",
                    ));
                }
            }
            Self::Integer(_) | Self::Boolean(_) => {}
        }
        Ok(())
    }
    /// Native USD scalar type, preserving integer precision.
    pub const fn usd_type(&self) -> &'static str {
        match self {
            Self::Text(_) => "string",
            Self::Number(_) => "double",
            Self::Integer(_) => "int64",
            Self::Boolean(_) => "bool",
        }
    }
    fn bytes(&self, out: &mut Vec<u8>) {
        match self {
            Self::Text(s) => {
                out.push(0);
                field(out, s.as_bytes());
            }
            Self::Number(v) => {
                out.push(1);
                out.extend(if *v == 0. {
                    0.0_f64.to_le_bytes()
                } else {
                    v.to_le_bytes()
                });
            }
            Self::Integer(v) => {
                out.push(2);
                out.extend(v.to_le_bytes());
            }
            Self::Boolean(v) => {
                out.extend([3, u8::from(*v)]);
            }
        }
    }
}
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend((bytes.len() as u64).to_le_bytes());
    out.extend(bytes);
}
impl AttributeDefinition {
    /// Admit names, types, units, choices and set membership.
    pub fn validate(&self) -> Result<(), CommandError> {
        if self.name.trim().is_empty()
            || self.name.len() > 256
            || self.name.chars().any(char::is_control)
        {
            return Err(invalid("attribute name must be a bounded nonempty label"));
        }
        if self.unit.len() > 32
            || self.unit.chars().any(char::is_control)
            || (!self.unit.is_empty()
                && !matches!(self.kind, AttributeType::Number | AttributeType::Integer))
        {
            return Err(invalid(
                "only numeric attributes can declare a bounded unit",
            ));
        }
        if self.choices.len() > 128
            || (self.kind == AttributeType::Choice) == self.choices.is_empty()
            || self.sets.len() > 16
        {
            return Err(invalid(
                "attribute choices or sets do not match the declared type",
            ));
        }
        for values in [&self.choices, &self.sets] {
            let mut seen = BTreeSet::new();
            for s in values {
                AttributeValue::Text(s.clone()).validate()?;
                if s.trim().is_empty() || s.chars().any(char::is_control) || !seen.insert(s) {
                    return Err(invalid(
                        "attribute choices and sets must be unique nonempty labels",
                    ));
                }
            }
        }
        Ok(())
    }
    /// Admit a value exactly, without conversion, fallback or treating missing values as zero.
    pub fn admits(&self, v: &AttributeValue) -> Result<(), CommandError> {
        self.validate()?;
        v.validate()?;
        let valid = match (self.kind, v) {
            (AttributeType::Text, AttributeValue::Text(_))
            | (AttributeType::Number, AttributeValue::Number(_))
            | (AttributeType::Integer, AttributeValue::Integer(_))
            | (AttributeType::Boolean, AttributeValue::Boolean(_)) => true,
            (AttributeType::Choice, AttributeValue::Text(s)) => self.choices.contains(s),
            _ => false,
        };
        if !valid {
            return Err(invalid("attribute value has the wrong type or choice"));
        }
        Ok(())
    }
    /// Stable create-request fingerprint.
    pub fn fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate()?;
        let mut out = b"BeamCraft/I04/DefineAttribute/v1\0".to_vec();
        field(&mut out, self.id.to_string().as_bytes());
        field(&mut out, self.name.as_bytes());
        field(&mut out, self.kind.token().as_bytes());
        field(&mut out, self.unit.as_bytes());
        for list in [&self.choices, &self.sets] {
            out.extend((list.len() as u64).to_le_bytes());
            for v in list {
                field(&mut out, v.as_bytes());
            }
        }
        Sha256::digest(&out).map_err(|_| invalid("attribute hash limit"))
    }
}
/// Stable patch fingerprint. None is an explicit removal, not an unknown or empty value.
pub fn patch_fingerprint(
    id: ElementId,
    values: &BTreeMap<ElementId, Option<AttributeValue>>,
) -> Result<ContentDigest, CommandError> {
    if values.is_empty() || values.len() > MAX_VALUES {
        return Err(invalid("attribute patch must contain 1..64 values"));
    }
    let mut out = b"BeamCraft/I04/PatchAttributes/v1\0".to_vec();
    field(&mut out, id.to_string().as_bytes());
    for (key, value) in values {
        field(&mut out, key.to_string().as_bytes());
        match value {
            Some(v) => {
                v.validate()?;
                out.push(1);
                v.bytes(&mut out);
            }
            None => out.push(0),
        }
    }
    Sha256::digest(&out).map_err(|_| invalid("attribute hash limit"))
}
/// Namespaced scalar property for a stable attribute identity.
pub fn property_name(id: ElementId) -> String {
    format!("user:a_{}", id.to_string().replace('-', ""))
}
