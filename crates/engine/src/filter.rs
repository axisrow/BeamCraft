// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V14 one bounded evaluator for object queries, view visibility and representation rules.
use beamcraft_model::{
    attribute::{AttributeDefinition, AttributeType, AttributeValue, MAX_VALUES},
    command::CommandError,
    error::ErrorCode,
    id::ElementId,
};
use std::collections::BTreeMap;
/// A standard metadata field or an explicitly known user definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// Element type token.
    Kind,
    /// Readable name.
    Name,
    /// Optional part class.
    Class,
    /// User definition UUID; unknown IDs are admission errors.
    Attribute(ElementId),
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
        value: AttributeValue,
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
    /// Optional part class.
    pub class: Option<i32>,
    /// Present scalar values; missing/blocked values are omitted explicitly.
    pub attributes: BTreeMap<ElementId, AttributeValue>,
}
/// Immutable admitted context; predicates cannot outlive or silently switch their registry.
#[derive(Debug, Clone)]
pub struct Dataset {
    definitions: BTreeMap<ElementId, AttributeDefinition>,
    records: BTreeMap<ElementId, Record>,
}
fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "filter", message)
}
impl Dataset {
    /// Admit a capped immutable set and all known scalar values before querying it.
    pub fn new(
        definitions: BTreeMap<ElementId, AttributeDefinition>,
        records: Vec<Record>,
    ) -> Result<Self, CommandError> {
        crate::attribute::validate(&definitions)?;
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
                || r.attributes.len() > MAX_VALUES
            {
                return Err(invalid("filter metadata exceeds its bounded profile"));
            }
            for (id, v) in &r.attributes {
                v.validate()?;
                if let Some(d) = definitions.get(id) {
                    d.admits_value(v)?;
                }
            }
            if map.insert(r.id, r).is_some() {
                return Err(invalid("duplicate filter target"));
            }
        }
        Ok(Self {
            definitions,
            records: map,
        })
    }
    /// Compile/admit a rule against this exact immutable registry.
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
                let kind = match field {
                    Field::Kind | Field::Name => AttributeType::Text,
                    Field::Class => AttributeType::Integer,
                    Field::Attribute(id) => {
                        self.definitions
                            .get(id)
                            .ok_or_else(|| {
                                CommandError::new(
                                    ErrorCode::NotFound,
                                    "attribute",
                                    "filter refers to an unknown attribute",
                                )
                            })?
                            .kind
                    }
                };
                let text = matches!(kind, AttributeType::Text | AttributeType::Choice);
                let scalar_type = matches!(
                    (kind, value),
                    (
                        AttributeType::Text | AttributeType::Choice,
                        AttributeValue::Text(_)
                    ) | (AttributeType::Number, AttributeValue::Number(_))
                        | (AttributeType::Integer, AttributeValue::Integer(_))
                        | (AttributeType::Boolean, AttributeValue::Boolean(_))
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
                    || (kind == AttributeType::Boolean
                        && !matches!(op, Operator::Eq | Operator::Ne))
                {
                    return Err(invalid("operator does not match the field type"));
                }
                if let Field::Attribute(id) = field
                    && matches!(op, Operator::Eq | Operator::Ne)
                {
                    self.definitions
                        .get(id)
                        .ok_or_else(|| invalid("unknown filter attribute"))?
                        .admits_value(value)?;
                }
            }
        }
        Ok(())
    }
}
/// A rule borrowed together with its admitted registry and target set.
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
fn compare(op: Operator, a: &AttributeValue, b: &AttributeValue) -> bool {
    use std::cmp::Ordering as O;
    let order = match (a, b) {
        (AttributeValue::Text(a), AttributeValue::Text(b)) => {
            return match op {
                Operator::Eq => a == b,
                Operator::Ne => a != b,
                Operator::Contains => a.contains(b),
                Operator::StartsWith => a.starts_with(b),
                _ => false,
            };
        }
        (AttributeValue::Number(a), AttributeValue::Number(b)) => a.partial_cmp(b),
        (AttributeValue::Integer(a), AttributeValue::Integer(b)) => Some(a.cmp(b)),
        (AttributeValue::Boolean(a), AttributeValue::Boolean(b)) => Some(a.cmp(b)),
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
                if let AttributeValue::Text(v) = value {
                    text_compare(*op, &r.kind, v)
                } else {
                    false
                }
            }
            Field::Name => {
                if let AttributeValue::Text(v) = value {
                    r.name
                        .as_ref()
                        .is_some_and(|name| text_compare(*op, name, v))
                } else {
                    false
                }
            }
            Field::Class => r
                .class
                .is_some_and(|v| compare(*op, &AttributeValue::Integer(i64::from(v)), value)),
            Field::Attribute(id) => r.attributes.get(id).is_some_and(|a| compare(*op, a, value)),
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_values_do_not_match_ne_and_unknown_fields_refuse() {
        let id = ElementId::parse("019a7bf0-0000-7000-8000-000000e00001").unwrap();
        let d = AttributeDefinition {
            id,
            name: "Score".into(),
            kind: AttributeType::Number,
            unit: String::new(),
            choices: Vec::new(),
            sets: Vec::new(),
        };
        let data = Dataset::new(
            BTreeMap::from([(id, d)]),
            vec![Record {
                id,
                kind: "plate".into(),
                name: Some("P".into()),
                class: Some(2),
                attributes: BTreeMap::new(),
            }],
        )
        .unwrap();
        assert!(
            data.query(&Rule::Compare {
                field: Field::Attribute(id),
                op: Operator::Ne,
                value: AttributeValue::Number(0.)
            })
            .unwrap()
            .is_empty()
        );
        assert_eq!(
            data.query(&Rule::All(vec![Rule::Compare {
                field: Field::Class,
                op: Operator::Eq,
                value: AttributeValue::Integer(2)
            }]))
            .unwrap(),
            vec![id]
        );
        let other = ElementId::parse("019a7bf0-0000-7000-8000-000000e00002").unwrap();
        assert!(
            data.query(&Rule::Compare {
                field: Field::Attribute(other),
                op: Operator::Eq,
                value: AttributeValue::Number(0.)
            })
            .is_err()
        );
    }
}
