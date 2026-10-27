// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! A03/A04 monotone counters and retired identities; restored atomically by undo.
use super::*;
use beamcraft_model::id::OpId;
/// Next ordinal after all cuts ever authored on this branch of the model's history.
pub const NEXT: &str = "steel:cut:nextOrdinal";
/// Sorted identities of explicitly removed cut records.
pub const RETIRED: &str = "steel:cut:retiredOperationIds";
/// Next ordinal for an owned hole record.
pub const HOLE_NEXT: &str = "steel:hole:nextOrdinal";
/// Sorted identities of explicitly removed hole records.
pub const HOLE_RETIRED: &str = "steel:hole:retiredOperationIds";
/// Bounded cut history, separate from live shape inputs and geometric signatures.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CutHistory {
    /// First ordinal that may be allocated, 1000 means exhausted.
    pub next_ordinal: u16,
    /// UUIDs that cannot be used by a new record.
    pub retired: Vec<OpId>,
}
pub(super) fn ids(p: &Prim, name: &str) -> Result<Vec<OpId>, Diagnostic> {
    let Some(value) = validate::optional(p, name, "string[]")? else {
        return Ok(Vec::new());
    };
    let Value::Array(values) = value else {
        return Err(invalid());
    };
    if values.len() > 1000 {
        return Err(limit());
    }
    values
        .iter()
        .map(|v| {
            let Value::Text(s) = v else {
                return Err(invalid());
            };
            OpId::parse(s).map_err(|_| invalid())
        })
        .collect()
}
/// The same bounded history type for hole records.
pub type HoleHistory = CutHistory;
pub(super) fn fields(hole: bool) -> (&'static str, &'static str, &'static str, &'static str) {
    if hole {
        (
            HOLE_NEXT,
            HOLE_RETIRED,
            "steel:hole:operationIds",
            "steel:hole:ordinals",
        )
    } else {
        (NEXT, RETIRED, "steel:cut:operationId", "steel:cut:ordinal")
    }
}
pub(super) fn ordinals(p: &Prim, name: &str) -> Result<Vec<u16>, Diagnostic> {
    let Some(value) = validate::optional(p, name, "int[]")? else {
        return Ok(Vec::new());
    };
    let Value::Array(values) = value else {
        return Err(invalid());
    };
    if values.len() > 1000 {
        return Err(limit());
    }
    let result = values
        .iter()
        .map(|v| {
            let Value::Int(n) = v else {
                return Err(invalid());
            };
            let n = u16::try_from(*n).map_err(|_| invalid())?;
            if n > 999 {
                return Err(invalid());
            }
            Ok(n)
        })
        .collect::<Result<Vec<_>, Diagnostic>>()?;
    if result.windows(2).any(|w| matches!(w,[a,b] if a>=b)) {
        return Err(invalid());
    }
    Ok(result)
}
/// Read explicit cut history or derive the initial counter from the live records.
pub fn read(p: &Prim) -> Result<CutHistory, Diagnostic> {
    read_for(p, false)
}
/// Read bounded hole history. Unsupported or malformed metadata is never repaired on read.
pub fn read_holes(p: &Prim) -> Result<HoleHistory, Diagnostic> {
    read_for(p, true)
}
fn read_for(p: &Prim, hole: bool) -> Result<CutHistory, Diagnostic> {
    let (next_name, retired_name, id_name, ordinal_name) = fields(hole);
    let ordinals = ordinals(p, ordinal_name)?;
    let mut next = ordinals.last().map_or(0, |n| n + 1);
    if let Some(v) = validate::optional(p, next_name, "int")? {
        let Value::Int(n) = v else {
            return Err(invalid());
        };
        let n = u16::try_from(*n).map_err(|_| invalid())?;
        if n < next || n > 1000 {
            return Err(invalid());
        }
        next = n;
    }
    let retired = ids(p, retired_name)?;
    if retired.windows(2).any(|w| matches!(w,[a,b] if a>=b)) || retired.len() > usize::from(next) {
        return Err(invalid());
    }
    if !retired.is_empty() && validate::optional(p, next_name, "int")?.is_none() {
        return Err(invalid());
    }
    let live = ids(p, id_name)?;
    if live.len() != ordinals.len() || live.iter().any(|id| retired.binary_search(id).is_ok()) {
        return Err(invalid());
    }
    Ok(CutHistory {
        next_ordinal: next,
        retired,
    })
}
