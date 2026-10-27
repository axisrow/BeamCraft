// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Native construction form and bounded reads; all geometry is admitted by the shared model.
use crate::tools::{FormError, Prepared, ToolError, ToolKind, text};
use beamcraft_engine::helper::Helpers;
use beamcraft_json::Json;
use beamcraft_model::{
    helper::{Helper, HelperGeometry},
    id::ElementId,
};
use std::path::Path;

/// Inputs of the construction object tool, in world coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    /// Geometry kind token.
    pub kind: &'static str,
    /// User label.
    pub name: String,
    /// Comma-separated explicit geometry inputs.
    pub parameters: String,
}
impl Default for Form {
    fn default() -> Self {
        Self {
            kind: "point",
            name: "Construction point".into(),
            parameters: "0,0,0".into(),
        }
    }
}
impl Form {
    /// Calls the same admission used by the CLI, then prepares its exact command.
    pub fn prepare(&self, store: &Path) -> Result<Prepared, FormError> {
        let invalid = |message: String| FormError {
            field: "Construction object",
            message,
        };
        if self.parameters.len() > 256 || self.name.len() > 256 {
            return Err(invalid("input exceeds the text limit".into()));
        }
        let values = self
            .parameters
            .split(',')
            .take(8)
            .map(|v| {
                v.trim()
                    .parse::<f64>()
                    .map_err(|_| invalid("enter comma-separated numbers".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        HelperGeometry::from_parameters(self.kind, &values)
            .map_err(|e| invalid(e.message.into()))?;
        Ok(Prepared {
            kind: ToolKind::Helper,
            args: vec![
                "create-helper".into(),
                "--store".into(),
                store.to_string_lossy().into_owned(),
                "--name".into(),
                self.name.clone(),
                "--kind".into(),
                self.kind.into(),
                "--parameters".into(),
                self.parameters.clone(),
            ],
        })
    }
}

/// Reads helpers from refresh metadata. Absence is an empty set for older CLI replies.
pub fn parse_helpers(reply: &Json) -> Result<Helpers, ToolError> {
    let invalid = || ToolError::local("UNREADABLE_REPLY", "invalid construction object metadata");
    let Ok(records) = reply.get("helpers") else {
        return Ok(Helpers::default());
    };
    let records = records.array().map_err(|_| invalid())?;
    if records.len() > beamcraft_model::helper::MAX_HELPERS {
        return Err(invalid());
    }
    let mut helpers = Helpers::default();
    for record in records {
        let id =
            ElementId::parse(&text(record, "id").ok_or_else(invalid)?).map_err(|_| invalid())?;
        let values = record
            .get("parameters")
            .and_then(Json::array)
            .map_err(|_| invalid())?;
        if values.len() > 7 {
            return Err(invalid());
        }
        let values = values
            .iter()
            .map(|v| v.float().map_err(|_| invalid()))
            .collect::<Result<Vec<_>, _>>()?;
        helpers
            .insert(Helper {
                id,
                name: text(record, "name").ok_or_else(invalid)?,
                geometry: HelperGeometry::from_parameters(
                    &text(record, "kind").ok_or_else(invalid)?,
                    &values,
                )
                .map_err(|_| invalid())?,
            })
            .map_err(|_| invalid())?;
    }
    Ok(helpers)
}
