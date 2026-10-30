// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! I04 user attributes: the engine's decisions on the attribute schema and on the values
//! elements carry, over the model decoded from the pinned head.
//!
//! A definition is schema metadata (a name, a type, an optional unit, the options of a choice,
//! an optional set), never an element: defining and undefining write the schema as a whole
//! ([`crate::footprint::Target::AttrSchema`]) and touch no element. A value lives with its
//! element and is written in that element's attributes group alone. An undefine removes only
//! the definition: stored values stay in the model, so an unknown attribute is shown, never
//! dropped.
//!
//! Rules: a defined attribute is defined once (`DUPLICATE_ID`); setting requires a definition
//! (`NOT_FOUND`) whose type matches the value (`INVALID_ARGUMENT`) and, for a choice, one of its
//! options; an element carries at most 256 values; clearing requires a stored value, known or
//! unknown. Every error leaves nothing applied; the layer engine re-checks the same rules on the
//! authored candidate, so a defect in either place fails closed.

use crate::footprint::Footprint;
use beamcraft_model::attribute::{
    AttrDef, AttrValue, ClearAttribute, DefineAttribute, SetAttribute, UndefineAttribute,
};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use beamcraft_model::limits::{MAX_ATTRIBUTE_DEFINITIONS, MAX_ATTRS_PER_ELEMENT};
use std::collections::{BTreeMap, BTreeSet};

fn error(code: ErrorCode, field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(code, field, message)
}

/// An accepted schema change: the definition the model holds after it and the declared
/// footprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaChange {
    /// The declared footprint.
    pub footprint: Footprint,
}

/// The attribute schema of a model, in name order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schema {
    defs: BTreeMap<String, AttrDef>,
}

impl Schema {
    /// Records one decoded definition.
    pub fn insert(&mut self, def: AttrDef) {
        self.defs.insert(def.name.clone(), def);
    }

    /// The definition of an attribute name.
    pub fn get(&self, name: &str) -> Option<&AttrDef> {
        self.defs.get(name)
    }

    /// Whether the name is defined.
    pub fn contains(&self, name: &str) -> bool {
        self.defs.contains_key(name)
    }

    /// How many definitions the schema holds.
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// Whether the schema holds no definition.
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Every definition, in name order.
    pub fn iter(&self) -> impl Iterator<Item = &AttrDef> {
        self.defs.values()
    }

    /// Decides a definition: the form is admitted, the name is free and the schema stays
    /// bounded. Errors leave nothing applied.
    pub fn define(&self, command: &DefineAttribute) -> Result<SchemaChange, CommandError> {
        command.validate_arguments()?;
        if self.contains(&command.def.name) {
            return Err(error(
                ErrorCode::DuplicateId,
                "name",
                "the attribute is already defined",
            ));
        }
        if self.len() >= MAX_ATTRIBUTE_DEFINITIONS {
            return Err(error(
                ErrorCode::LimitExceeded,
                "name",
                "too many attribute definitions",
            ));
        }
        Ok(SchemaChange {
            footprint: Footprint::for_attr_schema()?,
        })
    }

    /// Decides an undefinition: the name must be defined. Stored values are never touched, so
    /// the footprint writes the schema alone.
    pub fn undefine(&self, command: &UndefineAttribute) -> Result<SchemaChange, CommandError> {
        command.validate_arguments()?;
        if !self.contains(&command.name) {
            return Err(error(
                ErrorCode::NotFound,
                "name",
                "the attribute is not defined",
            ));
        }
        Ok(SchemaChange {
            footprint: Footprint::for_attr_schema()?,
        })
    }
}

/// The user-attribute values of a model's elements, and the elements themselves.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ElementValues {
    elements: BTreeSet<ElementId>,
    values: BTreeMap<(ElementId, String), AttrValue>,
}

/// An accepted value change: the value the element had before, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueChange {
    /// The former value; `None` when the attribute had no stored value.
    pub former: Option<AttrValue>,
    /// The declared footprint.
    pub footprint: Footprint,
}

impl ElementValues {
    /// Records one element of any kind, so that its identity counts as used.
    pub fn insert_element(&mut self, id: ElementId) {
        self.elements.insert(id);
    }

    /// Records one stored value; the element is recorded too.
    pub fn insert_value(&mut self, id: ElementId, name: &str, value: AttrValue) {
        self.elements.insert(id);
        self.values.insert((id, name.to_owned()), value);
    }

    /// Whether any element has this identity.
    pub fn contains(&self, id: &ElementId) -> bool {
        self.elements.contains(id)
    }

    /// The stored value of one attribute of one element.
    pub fn value_of(&self, id: &ElementId, name: &str) -> Option<&AttrValue> {
        self.values.get(&(*id, name.to_owned()))
    }

    /// Removes one stored value, as a clear does to the model.
    pub fn remove_value(&mut self, id: &ElementId, name: &str) {
        self.values.remove(&(*id, name.to_owned()));
    }

    /// How many values one element carries.
    pub fn count_of(&self, id: &ElementId) -> usize {
        self.values
            .range((*id, String::new())..)
            .take_while(|((e, _), _)| e == id)
            .count()
    }

    /// Decides a value store: the element exists, the attribute is defined, the value's type and
    /// option match the definition and the element stays bounded. Setting the stored value again
    /// is refused (`INVALID_ARGUMENT`), as a patch that changes nothing.
    pub fn set(
        &self,
        schema: &Schema,
        command: &SetAttribute,
    ) -> Result<ValueChange, CommandError> {
        command.validate_arguments()?;
        if !self.contains(&command.target_id) {
            return Err(error(
                ErrorCode::NotFound,
                "target_id",
                "the element is not in the head revision",
            ));
        }
        let def = schema
            .get(&command.name)
            .ok_or_else(|| error(ErrorCode::NotFound, "name", "the attribute is not defined"))?;
        if def.ty != command.value.attr_type() {
            return Err(error(
                ErrorCode::InvalidArgument,
                "value",
                "the value's type does not match the attribute's definition",
            ));
        }
        if let AttrValue::Choice(option) = &command.value
            && !def.options.iter().any(|o| o == option)
        {
            return Err(error(
                ErrorCode::InvalidArgument,
                "value",
                "the value is not one of the attribute's options",
            ));
        }
        let former = self.value_of(&command.target_id, &command.name).cloned();
        if former.as_ref().is_some_and(|f| *f == command.value) {
            return Err(error(
                ErrorCode::InvalidArgument,
                "value",
                "the value is already stored",
            ));
        }
        if former.is_none() && self.count_of(&command.target_id) >= MAX_ATTRS_PER_ELEMENT {
            return Err(error(
                ErrorCode::LimitExceeded,
                "value",
                "the element carries too many attribute values",
            ));
        }
        Ok(ValueChange {
            former,
            footprint: Footprint::for_attr_value(command.target_id)?,
        })
    }

    /// Decides a value clear: the element exists and carries the value, whose definition may be
    /// gone (an unknown attribute clears by name). An absent value is refused.
    pub fn clear(&self, command: &ClearAttribute) -> Result<ValueChange, CommandError> {
        command.validate_arguments()?;
        if !self.contains(&command.target_id) {
            return Err(error(
                ErrorCode::NotFound,
                "target_id",
                "the element is not in the head revision",
            ));
        }
        let former = self.value_of(&command.target_id, &command.name).cloned();
        let Some(former) = former else {
            return Err(error(
                ErrorCode::InvalidArgument,
                "name",
                "the element carries no such value",
            ));
        };
        Ok(ValueChange {
            former: Some(former),
            footprint: Footprint::for_attr_value(command.target_id)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::attribute::AttrType;

    fn element(n: u64) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
    }

    fn def(name: &str, ty: AttrType) -> AttrDef {
        AttrDef {
            name: name.into(),
            ty,
            unit: None,
            options: Vec::new(),
            set: None,
        }
    }

    fn schema() -> Schema {
        let mut s = Schema::default();
        s.insert(def("Note", AttrType::Text));
        let mut finish = def("Finish", AttrType::Choice);
        finish.options = vec!["plain".into(), "galvanized".into()];
        s.insert(finish);
        let mut len = def("Len", AttrType::Number);
        len.unit = Some("mm".into());
        s.insert(len);
        s.insert(def("Count", AttrType::Integer));
        s.insert(def("Hot", AttrType::Boolean));
        s
    }

    fn set(id: ElementId, name: &str, value: AttrValue) -> SetAttribute {
        SetAttribute {
            target_id: id,
            name: name.into(),
            value,
        }
    }

    #[test]
    fn a_definition_is_defined_once_and_undefined_only_when_present() {
        let s = schema();
        let defined = s
            .define(&DefineAttribute {
                def: def("Extra", AttrType::Text),
            })
            .unwrap();
        assert_eq!(defined.footprint.writes().len(), 1);
        assert_eq!(
            s.define(&DefineAttribute {
                def: def("Note", AttrType::Text),
            })
            .unwrap_err()
            .code,
            ErrorCode::DuplicateId
        );
        assert_eq!(
            s.undefine(&UndefineAttribute {
                name: "Missing".into(),
            })
            .unwrap_err()
            .code,
            ErrorCode::NotFound
        );
        assert!(
            s.undefine(&UndefineAttribute {
                name: "Note".into()
            })
            .is_ok()
        );
        assert_eq!(
            s.define(&DefineAttribute {
                def: AttrDef {
                    name: "Bad name".into(),
                    ty: AttrType::Text,
                    unit: None,
                    options: Vec::new(),
                    set: None,
                },
            })
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_value_is_set_only_defined_typed_and_changed() {
        let s = schema();
        let mut values = ElementValues::default();
        values.insert_element(element(1));
        let stored = values
            .set(&s, &set(element(1), "Len", AttrValue::Number(3000.0)))
            .unwrap();
        assert_eq!(stored.former, None);
        assert_eq!(stored.footprint.writes().len(), 1);
        values.insert_value(element(1), "Len", AttrValue::Number(3000.0));
        let code = |r: Result<ValueChange, CommandError>| r.unwrap_err().code;
        assert_eq!(
            code(values.set(&s, &set(element(1), "Len", AttrValue::Number(3000.0)))),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code(values.set(&s, &set(element(1), "Len", AttrValue::Integer(3)))),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code(values.set(&s, &set(element(1), "Nope", AttrValue::Text("x".into())))),
            ErrorCode::NotFound
        );
        assert_eq!(
            code(values.set(&s, &set(element(9), "Len", AttrValue::Number(1.0)))),
            ErrorCode::NotFound
        );
        let off = set(element(1), "Finish", AttrValue::Choice("painted".into()));
        assert_eq!(code(values.set(&s, &off)), ErrorCode::InvalidArgument);
        let on = set(element(1), "Finish", AttrValue::Choice("galvanized".into()));
        assert!(values.set(&s, &on).is_ok());
    }

    #[test]
    fn an_unknown_value_survives_an_undefine_and_clears_by_name() {
        let s = schema();
        let mut values = ElementValues::default();
        values.insert_element(element(2));
        values.insert_value(element(2), "Note", AttrValue::Text("kept".into()));
        s.undefine(&UndefineAttribute {
            name: "Note".into(),
        })
        .unwrap();
        // The value outlives the definition: it is still stored and still clears.
        assert_eq!(
            values.value_of(&element(2), "Note"),
            Some(&AttrValue::Text("kept".into()))
        );
        let cleared = values
            .clear(&ClearAttribute {
                target_id: element(2),
                name: "Note".into(),
            })
            .unwrap();
        assert_eq!(cleared.former, Some(AttrValue::Text("kept".into())));
        assert_eq!(cleared.footprint.writes().len(), 1);
        values.remove_value(&element(2), "Note");
        let code = |r: Result<ValueChange, CommandError>| r.unwrap_err().code;
        assert_eq!(
            code(values.clear(&ClearAttribute {
                target_id: element(2),
                name: "Note".into(),
            })),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code(values.clear(&ClearAttribute {
                target_id: element(8),
                name: "Note".into(),
            })),
            ErrorCode::NotFound
        );
    }

    #[test]
    fn an_element_carries_at_most_256_values() {
        let mut s = Schema::default();
        let mut values = ElementValues::default();
        values.insert_element(element(3));
        for i in 0..MAX_ATTRS_PER_ELEMENT {
            let name = format!("A{i:02}");
            s.insert(def(&name, AttrType::Integer));
            values
                .set(&s, &set(element(3), &name, AttrValue::Integer(i as i32)))
                .unwrap();
            values.insert_value(element(3), &name, AttrValue::Integer(i as i32));
        }
        s.insert(def("Overflow", AttrType::Integer));
        assert_eq!(
            values
                .set(&s, &set(element(3), "Overflow", AttrValue::Integer(0)))
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
    }
}
