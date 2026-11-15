// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Q03 recorded-input admission: what `create-record` proves against the head model.

use crate::footprint::Footprint;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use beamcraft_model::record::{CreateRecord, MAX_RECORDS, Record};
use std::collections::BTreeSet;

/// Admits one record creation against the head model: the values, the identity and the
/// project cap. The declared scope and connection ids are stored verbatim and are not
/// required to exist: the contract plans for targets that become missing after an edit,
/// and the check reports such a record as `target_unresolved` instead of blocking the
/// declaration.
pub fn create(
    command: &CreateRecord,
    records: &[Record],
    elements: &BTreeSet<ElementId>,
) -> Result<Footprint, CommandError> {
    command.validate_arguments()?;
    let error = |code, field, message| CommandError::new(code, field, message);
    let mut ids = elements.clone();
    for record in records {
        ids.insert(record.id);
    }
    if !ids.insert(command.record.id) {
        return Err(error(
            ErrorCode::DuplicateId,
            "new_id",
            "record identity is already in use",
        ));
    }
    if records.len() >= MAX_RECORDS {
        return Err(error(
            ErrorCode::LimitExceeded,
            "record",
            "project exceeds the recorded-input cap",
        ));
    }
    Footprint::for_create(command.record.id, None)
}
