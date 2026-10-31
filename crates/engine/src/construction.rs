// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! T02 construction object admission against a complete pinned project snapshot.
use crate::footprint::Footprint;
use beamcraft_model::command::CommandError;
use beamcraft_model::construction::{
    ConstructionObject, CreateConstructionObject, MAX_CONSTRUCTION_OBJECTS,
};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::BTreeSet;

/// Decide construction object creation without mutating the snapshot. The caller supplies every
/// existing element identity, including the identities of every construction object already in
/// the project. Publication still checks the base revision.
pub fn create(
    command: &CreateConstructionObject,
    objects: &[ConstructionObject],
    elements: &BTreeSet<ElementId>,
) -> Result<Footprint, CommandError> {
    command.validate_arguments()?;
    let error = |code, field, message| CommandError::new(code, field, message);
    let mut ids = elements.clone();
    for object in objects {
        ids.insert(object.id);
    }
    if !ids.insert(command.object.id) {
        return Err(error(
            ErrorCode::DuplicateId,
            "new_id",
            "construction object identity is already in use",
        ));
    }
    if objects.len() >= MAX_CONSTRUCTION_OBJECTS {
        return Err(error(
            ErrorCode::LimitExceeded,
            "construction",
            "project exceeds the construction object cap",
        ));
    }
    Footprint::for_create(command.object.id, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::construction::ConstructionGeometry;

    fn id(n: u32) -> ElementId {
        ElementId::parse(&format!("01990000-0000-7000-8000-{n:012x}")).unwrap()
    }

    fn object(n: u32) -> ConstructionObject {
        ConstructionObject {
            id: id(n),
            name: "P".to_owned(),
            geometry: ConstructionGeometry::Point { point_mm: [0.0; 3] },
        }
    }

    #[test]
    fn creation_admits_a_fresh_identity_and_refuses_reuse_and_the_cap() {
        let request = CreateConstructionObject { object: object(2) };
        assert!(create(&request, &[object(1)], &BTreeSet::new()).is_ok());
        let taken = BTreeSet::from([id(2)]);
        assert_eq!(
            create(&request, &[], &taken).unwrap_err().code,
            ErrorCode::DuplicateId
        );
        assert_eq!(
            create(&request, &[object(1)], &taken).unwrap_err().code,
            ErrorCode::DuplicateId
        );
        let full = (0..MAX_CONSTRUCTION_OBJECTS as u32)
            .map(object)
            .collect::<Vec<_>>();
        assert_eq!(
            create(
                &CreateConstructionObject {
                    object: object(9_999)
                },
                &full,
                &BTreeSet::new()
            )
            .unwrap_err()
            .code,
            ErrorCode::LimitExceeded
        );
    }
}
