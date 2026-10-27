// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! I04 pure registry/patch admission and semantic footprints, without geometry evaluation.
use crate::footprint::{Entry, Footprint, Group, Target};
use beamcraft_model::{
    attribute::{AttributeDefinition, AttributeValue, MAX_DEFINITIONS, MAX_VALUES},
    command::CommandError,
    error::ErrorCode,
    id::ElementId,
};
use std::collections::{BTreeMap, BTreeSet};
/// Admit an immutable definition registry and unique names.
pub fn validate(defs: &BTreeMap<ElementId, AttributeDefinition>) -> Result<(), CommandError> {
    if defs.len() > MAX_DEFINITIONS {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "attributes",
            "too many attribute definitions",
        ));
    }
    let mut names = BTreeSet::new();
    for (id, d) in defs {
        d.validate()?;
        if *id != d.id || !names.insert(&d.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "attributes",
                "duplicate attribute name or inconsistent identity",
            ));
        }
    }
    Ok(())
}
/// Validate a typed patch and derive its coarse metadata footprint. Existing ShapeInputs is used
/// conservatively for conflict detection; engineering signatures still exclude user properties.
pub fn patch(
    defs: &BTreeMap<ElementId, AttributeDefinition>,
    target: ElementId,
    values: &BTreeMap<ElementId, Option<AttributeValue>>,
) -> Result<Footprint, CommandError> {
    validate(defs)?;
    if values.is_empty() || values.len() > MAX_VALUES {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "attributes",
            "patch needs 1..64 properties",
        ));
    }
    let entry = |id, group| Entry {
        target: Target::Element(id),
        group,
    };
    let mut reads = vec![
        entry(target, Group::Lifecycle),
        entry(target, Group::ShapeInputs),
    ];
    for (id, value) in values {
        reads.push(entry(*id, Group::Lifecycle));
        if let Some(value) = value {
            defs.get(id)
                .ok_or_else(|| {
                    CommandError::new(
                        ErrorCode::NotFound,
                        "attribute",
                        "attribute definition is absent",
                    )
                })?
                .admits_value(value)?;
            reads.push(entry(*id, Group::ShapeInputs));
        }
    }
    Footprint::from_entries(&reads, &[entry(target, Group::ShapeInputs)])
}
#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::attribute::AttributeType;
    #[test]
    fn exact_types_choices_and_registry_names_are_enforced() {
        let id = ElementId::parse("019a7bf0-0000-7000-8000-000000d00001").unwrap();
        let d = AttributeDefinition {
            id,
            name: "Status".into(),
            kind: AttributeType::Choice,
            unit: String::new(),
            choices: vec!["New".into(), "Done".into()],
            sets: vec!["Review".into()],
        };
        let defs = BTreeMap::from([(id, d)]);
        assert!(
            patch(
                &defs,
                id,
                &BTreeMap::from([(id, Some(AttributeValue::Text("New".into())))])
            )
            .is_ok()
        );
        assert!(
            patch(
                &defs,
                id,
                &BTreeMap::from([(id, Some(AttributeValue::Text("Other".into())))])
            )
            .is_err()
        );
        assert!(
            patch(
                &defs,
                id,
                &BTreeMap::from([(id, Some(AttributeValue::Integer(1)))])
            )
            .is_err()
        );
    }
}
