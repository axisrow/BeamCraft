// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! A03 monotone counters and retired identities; model state, restored atomically by undo.
use super::*;
use beamcraft_model::id::OpId;
/// Next ordinal after all cuts ever authored on this branch of the model's history.
pub const NEXT: &str = "steel:cut:nextOrdinal";
/// Sorted identities of explicitly removed cut records.
pub const RETIRED: &str = "steel:cut:retiredOperationIds";
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
/// Read explicit history or derive the initial counter from old live cut ordinals. Unknown,
/// half-typed, regressing or conflicting histories are errors, never repaired on read.
pub fn read(p: &Prim) -> Result<CutHistory, Diagnostic> {
    let mut next = 0;
    if let Some(v) = validate::optional(p, "steel:cut:ordinal", "int[]")? {
        let Value::Array(v) = v else {
            return Err(invalid());
        };
        if v.len() > 1000 {
            return Err(limit());
        }
        for n in v {
            let Value::Int(n) = n else {
                return Err(invalid());
            };
            let n = u16::try_from(*n).map_err(|_| invalid())?;
            if n > 999 {
                return Err(invalid());
            }
            next = next.max(n + 1);
        }
    }
    if let Some(v) = validate::optional(p, NEXT, "int")? {
        let Value::Int(n) = v else {
            return Err(invalid());
        };
        let n = u16::try_from(*n).map_err(|_| invalid())?;
        if n < next || n > 1000 {
            return Err(invalid());
        }
        next = n;
    }
    let retired = ids(p, RETIRED)?;
    if retired.windows(2).any(|w| matches!(w,[a,b] if a>=b)) {
        return Err(invalid());
    }
    if !retired.is_empty() && (validate::optional(p, NEXT, "int")?.is_none() || next == 0) {
        return Err(invalid());
    }
    if retired.len() > usize::from(next) {
        return Err(invalid());
    }
    let live = ids(p, "steel:cut:operationId")?;
    if live.iter().any(|id| retired.binary_search(id).is_ok()) {
        return Err(invalid());
    }
    Ok(CutHistory {
        next_ordinal: next,
        retired,
    })
}
