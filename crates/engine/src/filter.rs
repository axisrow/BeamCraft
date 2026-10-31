// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V14 one bounded evaluator for object queries, saved filters and view visibility.
//!
//! A rule is admitted against an immutable [`Dataset`] before any element is evaluated, so an
//! unknown field, an unknown attribute or an incompatible operator refuses before matching, and
//! a missing value never satisfies a comparison — not even `ne`. The evaluator is pure: it reads
//! the admitted records and writes nothing, which is what lets the CLI query, a saved personal
//! set and a later view binding share one semantics.

use crate::attribute::Schema;
use beamcraft_model::attribute::{AttrDef, AttrType, AttrValue};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use beamcraft_model::limits::MAX_ATTRS_PER_ELEMENT;
use std::collections::BTreeMap;

/// A standard metadata field or an explicitly known user definition name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field {
    /// Element type token.
    Kind,
    /// Readable name.
    Name,
    /// Optional part class.
    Class,
    /// User definition name; unknown names are admission errors.
    Attribute(String),
}
/// Closed comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    /// Equal.
    Eq,
    /// Not equal; missing values still do not match.
    Ne,
    /// Less than.
    Lt,
    /// Less or equal.
    Le,
    /// Greater than.
    Gt,
    /// Greater or equal.
    Ge,
    /// Case-sensitive substring.
    Contains,
    /// Case-sensitive prefix.
    StartsWith,
}
impl Operator {
    /// Stable JSON token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Eq => "eq",
            Self::Ne => "ne",
            Self::Lt => "lt",
            Self::Le => "le",
            Self::Gt => "gt",
            Self::Ge => "ge",
            Self::Contains => "contains",
            Self::StartsWith => "starts-with",
        }
    }
    /// Parse an exact token.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "eq" => Some(Self::Eq),
            "ne" => Some(Self::Ne),
            "lt" => Some(Self::Lt),
            "le" => Some(Self::Le),
            "gt" => Some(Self::Gt),
            "ge" => Some(Self::Ge),
            "contains" => Some(Self::Contains),
            "starts-with" => Some(Self::StartsWith),
            _ => None,
        }
    }
}
/// A rule is admitted before any element is evaluated; at most 256 nodes and depth eight.
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    /// Logical AND; an empty group is true.
    All(Vec<Rule>),
    /// Logical OR; an empty group is false.
    Any(Vec<Rule>),
    /// Typed scalar comparison.
    Compare {
        /// Field selector.
        field: Field,
        /// Operator compatible with the field type.
        op: Operator,
        /// Literal in the field's native unit/type.
        value: AttrValue,
    },
}
/// Metadata of one admitted display/query target.
#[derive(Debug, Clone)]
pub struct Record {
    /// Persistent identity.
    pub id: ElementId,
    /// Model type token.
    pub kind: String,
    /// Readable label.
    pub name: Option<String>,
    /// Part class; absent for element categories that carry no part class.
    pub class: Option<i32>,
    /// Present typed values; missing, unknown or blocked values are omitted explicitly.
    pub attributes: BTreeMap<String, AttrValue>,
}
/// Immutable admitted context; predicates cannot outlive or silently switch their schema.
#[derive(Debug, Clone)]
pub struct Dataset {
    schema: Schema,
    records: BTreeMap<ElementId, Record>,
}
fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "filter", message)
}
/// Whether a value may be stored under a definition: the type matches and a choice value is one
/// of the definition's options. A value that fails this check is not a filter target: readers
/// omit it, so it never matches, instead of refusing the whole dataset. Admission of the
/// value's own form remains the caller's step.
pub fn admits(def: &AttrDef, value: &AttrValue) -> bool {
    if def.ty != value.attr_type() {
        return false;
    }
    match value {
        AttrValue::Choice(option) => def.options.iter().any(|o| o == option),
        _ => true,
    }
}
impl Dataset {
    /// Admit a capped immutable set and all known scalar values before querying it.
    pub fn new(schema: Schema, records: Vec<Record>) -> Result<Self, CommandError> {
        for def in schema.iter() {
            def.validate()?;
        }
        if schema.len() > beamcraft_model::limits::MAX_ATTRIBUTE_DEFINITIONS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "filter",
                "too many attribute definitions",
            ));
        }
        if records.len() > crate::limits::MAX_ELEMENTS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "filter",
                "too many filter targets",
            ));
        }
        let mut map = BTreeMap::new();
        for r in records {
            if r.kind.is_empty()
                || r.kind.len() > 256
                || r.kind.chars().any(char::is_control)
                || r.name.as_ref().is_some_and(|s| s.len() > 256)
                || r.attributes.len() > MAX_ATTRS_PER_ELEMENT
            {
                return Err(invalid("filter metadata exceeds its bounded profile"));
            }
            for (name, v) in &r.attributes {
                v.validate()?;
                if let Some(def) = schema.get(name)
                    && !admits(def, v)
                {
                    return Err(invalid(
                        "a filter target value does not match its definition",
                    ));
                }
            }
            if map.insert(r.id, r).is_some() {
                return Err(invalid("duplicate filter target"));
            }
        }
        Ok(Self {
            schema,
            records: map,
        })
    }
    /// Compile/admit a rule against this exact immutable schema.
    pub fn predicate<'a>(&'a self, rule: &'a Rule) -> Result<Predicate<'a>, CommandError> {
        let mut count = 0;
        self.admit(rule, 0, &mut count)?;
        Ok(Predicate {
            dataset: self,
            rule,
        })
    }
    /// IDs in stable sorted order; there is no implicit missing-to-zero or name/class fallback.
    pub fn query(&self, rule: &Rule) -> Result<Vec<ElementId>, CommandError> {
        Ok(self.predicate(rule)?.ids())
    }
    fn admit(&self, rule: &Rule, depth: usize, count: &mut usize) -> Result<(), CommandError> {
        *count = count.saturating_add(1);
        if depth >= 8 || *count > 256 {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "filter",
                "filter exceeds 256 nodes or depth eight",
            ));
        }
        match rule {
            Rule::All(rules) | Rule::Any(rules) => {
                if rules.len() > 256 {
                    return Err(invalid("filter group is too large"));
                }
                for r in rules {
                    self.admit(r, depth + 1, count)?;
                }
            }
            Rule::Compare { field, op, value } => {
                value.validate()?;
                let (def, kind, text) = match field {
                    Field::Kind | Field::Name => (None, AttrType::Text, true),
                    Field::Class => (None, AttrType::Integer, false),
                    Field::Attribute(name) => {
                        let def = self.schema.get(name).ok_or_else(|| {
                            CommandError::new(
                                ErrorCode::NotFound,
                                "attribute",
                                "filter refers to an unknown attribute",
                            )
                        })?;
                        (Some(def), def.ty, def.ty == AttrType::Choice)
                    }
                };
                let scalar_type = matches!(
                    (kind, value),
                    (AttrType::Text, AttrValue::Text(_))
                        | (AttrType::Choice, AttrValue::Choice(_))
                        | (AttrType::Number, AttrValue::Number(_))
                        | (AttrType::Integer, AttrValue::Integer(_))
                        | (AttrType::Boolean, AttrValue::Boolean(_))
                );
                if !scalar_type {
                    return Err(invalid("filter literal does not match its field type"));
                }
                if text {
                    if !matches!(
                        op,
                        Operator::Eq | Operator::Ne | Operator::Contains | Operator::StartsWith
                    ) {
                        return Err(invalid(
                            "text filters support equality, substring and prefix",
                        ));
                    }
                } else if matches!(op, Operator::Contains | Operator::StartsWith)
                    || (kind == AttrType::Boolean && !matches!(op, Operator::Eq | Operator::Ne))
                {
                    return Err(invalid("operator does not match the field type"));
                }
                if let (Some(def), Operator::Eq | Operator::Ne) = (def, *op)
                    && !admits(def, value)
                {
                    return Err(invalid("filter literal is not a value of its attribute"));
                }
            }
        }
        Ok(())
    }
}
/// A rule borrowed together with its admitted schema and target set.
pub struct Predicate<'a> {
    dataset: &'a Dataset,
    rule: &'a Rule,
}
impl Predicate<'_> {
    /// Test an admitted persistent target; an absent target never matches.
    pub fn matches(&self, id: ElementId) -> bool {
        self.dataset
            .records
            .get(&id)
            .is_some_and(|r| matches_rule(self.rule, r))
    }
    /// Every matching target UUID, in canonical order.
    pub fn ids(&self) -> Vec<ElementId> {
        self.dataset
            .records
            .iter()
            .filter(|(_, r)| matches_rule(self.rule, r))
            .map(|(id, _)| *id)
            .collect()
    }
}
fn text_compare(op: Operator, a: &str, b: &str) -> bool {
    match op {
        Operator::Eq => a == b,
        Operator::Ne => a != b,
        Operator::Contains => a.contains(b),
        Operator::StartsWith => a.starts_with(b),
        _ => false,
    }
}
fn compare(op: Operator, a: &AttrValue, b: &AttrValue) -> bool {
    use std::cmp::Ordering as O;
    let order = match (a, b) {
        (AttrValue::Text(a), AttrValue::Text(b)) | (AttrValue::Choice(a), AttrValue::Choice(b)) => {
            return text_compare(op, a, b);
        }
        (AttrValue::Number(a), AttrValue::Number(b)) => a.partial_cmp(b),
        (AttrValue::Integer(a), AttrValue::Integer(b)) => Some(a.cmp(b)),
        (AttrValue::Boolean(a), AttrValue::Boolean(b)) => Some(a.cmp(b)),
        _ => None,
    };
    let Some(order) = order else {
        return false;
    };
    match op {
        Operator::Eq => order == O::Equal,
        Operator::Ne => order != O::Equal,
        Operator::Lt => order == O::Less,
        Operator::Le => order != O::Greater,
        Operator::Gt => order == O::Greater,
        Operator::Ge => order != O::Less,
        _ => false,
    }
}
fn matches_rule(rule: &Rule, r: &Record) -> bool {
    match rule {
        Rule::All(rules) => rules.iter().all(|rule| matches_rule(rule, r)),
        Rule::Any(rules) => rules.iter().any(|rule| matches_rule(rule, r)),
        Rule::Compare { field, op, value } => match field {
            Field::Kind => {
                if let AttrValue::Text(v) = value {
                    text_compare(*op, &r.kind, v)
                } else {
                    false
                }
            }
            Field::Name => {
                if let AttrValue::Text(v) = value {
                    r.name
                        .as_ref()
                        .is_some_and(|name| text_compare(*op, name, v))
                } else {
                    false
                }
            }
            Field::Class => r
                .class
                .is_some_and(|v| compare(*op, &AttrValue::Integer(v), value)),
            Field::Attribute(name) => r
                .attributes
                .get(name)
                .is_some_and(|a| compare(*op, a, value)),
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribute::Schema;
    use beamcraft_model::attribute::AttrDef;

    fn id(n: u64) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
    }

    fn dataset() -> Dataset {
        let mut schema = Schema::default();
        schema.insert(AttrDef {
            name: "Score".into(),
            ty: AttrType::Number,
            unit: None,
            options: Vec::new(),
            set: None,
        });
        schema.insert(AttrDef {
            name: "Status".into(),
            ty: AttrType::Choice,
            unit: None,
            options: vec!["New".into(), "Done".into()],
            set: None,
        });
        Dataset::new(
            schema,
            vec![Record {
                id: id(1),
                kind: "plate".into(),
                name: Some("P".into()),
                class: Some(2),
                attributes: BTreeMap::new(),
            }],
        )
        .unwrap()
    }

    #[test]
    fn missing_values_do_not_match_ne_and_unknown_fields_refuse() {
        let data = dataset();
        let ne = |name: &str, value: AttrValue| {
            data.query(&Rule::Compare {
                field: Field::Attribute(name.into()),
                op: Operator::Ne,
                value,
            })
        };
        assert!(ne("Score", AttrValue::Number(0.)).unwrap().is_empty());
        assert_eq!(
            data.query(&Rule::All(vec![Rule::Compare {
                field: Field::Class,
                op: Operator::Eq,
                value: AttrValue::Integer(2),
            }]))
            .unwrap(),
            vec![id(1)]
        );
        assert!(ne("Missing", AttrValue::Number(0.)).is_err());
    }

    #[test]
    fn choices_compare_by_text_and_foreign_options_refuse() {
        let data = dataset();
        let rule = |value: AttrValue| Rule::Compare {
            field: Field::Attribute("Status".into()),
            op: Operator::Eq,
            value,
        };
        assert_eq!(
            data.query(&rule(AttrValue::Choice("New".into()))).unwrap(),
            Vec::<ElementId>::new()
        );
        assert!(data.query(&rule(AttrValue::Choice("Nope".into()))).is_err());
        assert!(data.query(&rule(AttrValue::Text("New".into()))).is_err());
    }

    #[test]
    fn groups_bounds_and_duplicates_refuse() {
        let deep = |n: usize| {
            let mut rule = Rule::Compare {
                field: Field::Class,
                op: Operator::Eq,
                value: AttrValue::Integer(1),
            };
            for _ in 0..n {
                rule = Rule::All(vec![rule]);
            }
            rule
        };
        assert!(dataset().query(&deep(6)).is_ok());
        assert!(dataset().query(&deep(8)).is_err());
        assert!(
            Dataset::new(
                Schema::default(),
                vec![
                    Record {
                        id: id(1),
                        kind: "plate".into(),
                        name: None,
                        class: None,
                        attributes: BTreeMap::new()
                    },
                    Record {
                        id: id(1),
                        kind: "beam".into(),
                        name: None,
                        class: None,
                        attributes: BTreeMap::new()
                    },
                ],
            )
            .is_err()
        );
        assert!(
            Dataset::new(Schema::default(), Vec::new())
                .unwrap()
                .query(&Rule::Any(Vec::new()))
                .unwrap()
                .is_empty()
        );
    }
}
