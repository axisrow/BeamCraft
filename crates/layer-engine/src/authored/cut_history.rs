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
/// Next ordinal for an owned member-hole record (A04.2).
pub const MHOLE_NEXT: &str = "steel:mhole:nextOrdinal";
/// Sorted identities of explicitly removed member-hole records.
pub const MHOLE_RETIRED: &str = "steel:mhole:retiredOperationIds";
/// Bounded cut history, separate from live shape inputs and geometric signatures.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CutHistory {
    /// First ordinal that may be allocated, 1000 means exhausted.
    pub next_ordinal: u16,
    /// UUIDs that cannot be used by a new record.
    pub retired: Vec<OpId>,
}
/// The operation identities stored under one id attribute; the seam a discipline's record
/// history reads its stored ids through.
pub fn ids(p: &Prim, name: &str) -> Result<Vec<OpId>, Diagnostic> {
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
/// Which record family a counter, a retired list and a record list belong to: the fields stay
/// parallel within one family, so a member hole (A04.2) never mixes with a plate hole (A04) or
/// a plane cut (A03) even though the history shape is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// The owned plane cuts of a plate, beam or column.
    Cut,
    /// The hole and polygon-cut records of a plate.
    PlateHole,
    /// The member-hole records of a beam or column.
    MemberHole,
}
/// The attribute names of one record family: the next-ordinal and retired fields plus the
/// record list's own id and ordinal attributes; the seam's history field map.
pub fn fields(family: Family) -> (&'static str, &'static str, &'static str, &'static str) {
    match family {
        Family::Cut => (NEXT, RETIRED, "steel:cut:operationId", "steel:cut:ordinal"),
        Family::PlateHole => (
            HOLE_NEXT,
            HOLE_RETIRED,
            "steel:hole:operationIds",
            "steel:hole:ordinals",
        ),
        Family::MemberHole => (
            MHOLE_NEXT,
            MHOLE_RETIRED,
            "steel:mhole:operationIds",
            "steel:mhole:ordinals",
        ),
    }
}
/// The ordinals stored under one ordinal attribute, ascending; the seam a discipline's record
/// history reads its stored ordinals through.
pub fn ordinals(p: &Prim, name: &str) -> Result<Vec<u16>, Diagnostic> {
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
    read_for(p, Family::Cut)
}
/// Read bounded hole history. Unsupported or malformed metadata is never repaired on read.
pub fn read_holes(p: &Prim) -> Result<HoleHistory, Diagnostic> {
    read_for(p, Family::PlateHole)
}
/// Read bounded member-hole history (A04.2).
pub fn read_member_holes(p: &Prim) -> Result<HoleHistory, Diagnostic> {
    read_for(p, Family::MemberHole)
}
fn read_for(p: &Prim, family: Family) -> Result<CutHistory, Diagnostic> {
    let (next_name, retired_name, id_name, ordinal_name) = fields(family);
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
