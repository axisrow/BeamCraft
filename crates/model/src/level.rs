// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D2 batch input: several levels created as one operation. The elevations come from one spacing
//! expression read as a regular grid family reads its coordinates (`-1000 1000 6*3500`), and the
//! names are typed or continued from the last typed one.

use crate::command::{CommandError, CreateLevel, label_is_valid};
use crate::digest::{ContentDigest, Sha256};
use crate::error::ErrorCode;
use crate::grid::{GridExpressionError, MAX_GRID_LINES_PER_FAMILY, parse_grid_expression};
use crate::id::ElementId;
use std::collections::BTreeSet;

/// The most levels one `CreateLevels` creates; the cap of a regular grid family.
pub const MAX_LEVELS_PER_COMMAND: usize = MAX_GRID_LINES_PER_FAMILY;

/// The name the generated names start from when none is typed.
pub const DEFAULT_LEVEL_NAME: &str = "Level 1";

/// The longest name list read, in bytes.
const MAX_NAMES_TEXT_BYTES: usize = 65_536;

/// D2 `CreateLevels`: several new levels in one revision, as the user typed them. The engine
/// expands the request against the names the base revision uses (see [`CreateLevels::levels`]),
/// so the same operation with the same input replays even after its names became used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateLevels {
    /// One identity per elevation, in the order of the elevations.
    pub new_ids: Vec<ElementId>,
    /// The spacing expression of the elevations ([`level_elevations`]).
    pub elevations: String,
    /// The typed names, separated by commas or line breaks; empty to generate every name
    /// ([`level_names`]).
    pub names: String,
}

impl CreateLevels {
    /// Admits the request: a valid spacing expression of at most [`MAX_LEVELS_PER_COMMAND`]
    /// elevations, one distinct identity per elevation (`INVALID_ARGUMENT`, `DUPLICATE_ID`) and
    /// valid typed names. Whether the identities are free is the engine's decision.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        self.levels(&BTreeSet::new()).map(|_| ())
    }

    /// The levels this request creates in a model whose levels use the names in `used`:
    /// the elevations lowest first, each with its identity and its typed or generated name.
    pub fn levels(&self, used: &BTreeSet<String>) -> Result<Vec<CreateLevel>, CommandError> {
        let elevations = level_elevations(&self.elevations)?;
        if self.new_ids.len() != elevations.len() {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "new_ids",
                "one identity is required per elevation",
            ));
        }
        let distinct: BTreeSet<_> = self.new_ids.iter().collect();
        if distinct.len() != self.new_ids.len() {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "new_ids",
                "two levels have the same identity",
            ));
        }
        let names = level_names(&self.names, elevations.len(), used)?;
        let levels: Vec<CreateLevel> = self
            .new_ids
            .iter()
            .zip(names)
            .zip(elevations)
            .map(|((new_id, name), elevation_mm)| CreateLevel {
                new_id: *new_id,
                name,
                elevation_mm,
            })
            .collect();
        for level in &levels {
            level.validate_arguments()?;
        }
        Ok(levels)
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated input text. The
    /// identities are not part of it: they are chosen per call, and the same operation id with
    /// the same input replays whatever the first call created.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let elevations = self.elevations.trim();
        let mut bytes = Vec::with_capacity(
            48usize
                .saturating_add(elevations.len())
                .saturating_add(self.names.len()),
        );
        bytes.extend_from_slice(b"BeamCraft/D2/CreateLevels/v1\0");
        for text in [elevations, self.names.as_str()] {
            bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}
/// The elevations of a spacing expression, lowest first: the first number is an elevation, every
/// later one a spacing of at least 1 mm, and `N*D` stands for N equal spacings. `-1000 1000 6*3500`
/// gives -1000, 0, 3500, ..., 21000.
pub fn level_elevations(expression: &str) -> Result<Vec<f64>, CommandError> {
    let invalid = |message| CommandError::new(ErrorCode::InvalidArgument, "elevations", message);
    parse_grid_expression(expression.trim())
        .map(|(_, elevations)| elevations)
        .map_err(|error| match error {
            GridExpressionError::Invalid => invalid(
                "write the first elevation, then spacings such as 3500 or 6*3500, in millimetres",
            ),
            GridExpressionError::Spacing => invalid("a spacing must be at least 1 mm"),
            GridExpressionError::NonFinite => CommandError::new(
                ErrorCode::NonFiniteNumber,
                "elevations",
                "an elevation is not finite",
            )
            .with_rule("DV-ATTR-003"),
            GridExpressionError::Limit => CommandError::new(
                ErrorCode::LimitExceeded,
                "elevations",
                "at most 200 levels, each within 1,000,000 mm",
            )
            .with_rule("DV-LIM-001"),
        })
}

/// `count` level names. The typed names (separated by commas or line breaks) are taken in order;
/// the rest continue from the last typed one, or from [`DEFAULT_LEVEL_NAME`] when none is typed,
/// skipping every name in `used` and every name already given. A trailing number counts up and
/// keeps its width (`Level 09` → `Level 10`); a name without one gains ` 2`.
pub fn level_names(
    text: &str,
    count: usize,
    used: &BTreeSet<String>,
) -> Result<Vec<String>, CommandError> {
    let invalid = |message| CommandError::new(ErrorCode::InvalidArgument, "names", message);
    if text.len() > MAX_NAMES_TEXT_BYTES || count > MAX_LEVELS_PER_COMMAND {
        return Err(
            CommandError::new(ErrorCode::LimitExceeded, "names", "too many names")
                .with_rule("DV-LIM-001"),
        );
    }
    let mut names: Vec<String> = text
        .split([',', '\n'])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .take(count)
        .map(str::to_owned)
        .collect();
    if names.iter().any(|name| !label_is_valid(name)) {
        return Err(invalid(
            "a name must be 1 to 256 bytes without control characters",
        ));
    }
    let mut taken: BTreeSet<&str> = used.iter().map(String::as_str).collect();
    let mut generated = Vec::with_capacity(count.saturating_sub(names.len()));
    taken.extend(names.iter().map(String::as_str));
    let mut seed = names.last().cloned();
    for _ in names.len()..count {
        // Each step leaves one more name taken; at most `taken.len() + 1` candidates are tried.
        let mut found = None;
        for _ in 0..=taken.len().saturating_add(generated.len()) {
            let candidate = match &seed {
                Some(previous) => next_level_name(previous)?,
                None => DEFAULT_LEVEL_NAME.to_owned(),
            };
            if !label_is_valid(&candidate) {
                return Err(invalid("a generated name is longer than 256 bytes"));
            }
            seed = Some(candidate.clone());
            if !taken.contains(candidate.as_str()) && !generated.contains(&candidate) {
                found = Some(candidate);
                break;
            }
        }
        generated.push(found.ok_or_else(|| invalid("no free name could be generated"))?);
    }
    names.extend(generated);
    Ok(names)
}

fn next_level_name(seed: &str) -> Result<String, CommandError> {
    let digits = seed.bytes().rev().take_while(u8::is_ascii_digit).count();
    let Some((prefix, number)) = seed.split_at_checked(seed.len().saturating_sub(digits)) else {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "names",
            "a name cannot be continued",
        ));
    };
    if number.is_empty() {
        return Ok(format!("{seed} 2"));
    }
    let next = number
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::LimitExceeded,
                "names",
                "a name's number is too large to continue",
            )
        })?;
    Ok(format!("{prefix}{next:0width$}", width = number.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn used(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn elevations_read_like_a_grid_family() {
        assert_eq!(
            level_elevations("-1000 1000 6*3500").unwrap(),
            vec![
                -1000.0, 0.0, 3500.0, 7000.0, 10500.0, 14000.0, 17500.0, 21000.0
            ]
        );
        assert_eq!(level_elevations(" 0 ").unwrap(), vec![0.0]);
        assert_eq!(level_elevations("0, 3000, 2*2800").unwrap().len(), 4);
    }

    #[test]
    fn bad_elevations_are_refused_with_their_reason() {
        let code = |text| level_elevations(text).unwrap_err().code;
        assert_eq!(code(""), ErrorCode::InvalidArgument);
        assert_eq!(code("0 abc"), ErrorCode::InvalidArgument);
        assert_eq!(code("-1000 0 6*3500"), ErrorCode::InvalidArgument);
        assert_eq!(code("0 201*1000"), ErrorCode::LimitExceeded);
        assert_eq!(code("0 2*900000"), ErrorCode::LimitExceeded);
    }

    #[test]
    fn names_continue_from_the_last_typed_one() {
        assert_eq!(
            level_names("B1, Level 0", 4, &BTreeSet::new()).unwrap(),
            ["B1", "Level 0", "Level 1", "Level 2"]
        );
        assert_eq!(
            level_names("Floor 09", 3, &BTreeSet::new()).unwrap(),
            ["Floor 09", "Floor 10", "Floor 11"]
        );
        assert_eq!(
            level_names("Roof", 3, &BTreeSet::new()).unwrap(),
            ["Roof", "Roof 2", "Roof 3"]
        );
        assert_eq!(
            level_names("A, B, C", 2, &BTreeSet::new()).unwrap(),
            ["A", "B"]
        );
    }

    #[test]
    fn generated_names_skip_the_names_in_use() {
        assert_eq!(
            level_names("", 3, &used(&["Level 1", "Level 3"])).unwrap(),
            ["Level 2", "Level 4", "Level 5"]
        );
        // A typed name is taken as typed, even when another level has it.
        assert_eq!(
            level_names("Level 1", 2, &used(&["Level 1", "Level 2"])).unwrap(),
            ["Level 1", "Level 3"]
        );
    }

    #[test]
    fn bad_names_are_refused() {
        let none = BTreeSet::new();
        assert_eq!(
            level_names("a\u{7}b", 1, &none).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            level_names(&format!("L{}", "9".repeat(30)), 2, &none)
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
        assert_eq!(
            level_names(&"x".repeat(256), 2, &none).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            level_names("", 201, &none).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("01900000-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    fn batch(ids: &[u8], elevations: &str, names: &str) -> CreateLevels {
        CreateLevels {
            new_ids: ids.iter().map(|n| id(*n)).collect(),
            elevations: elevations.to_owned(),
            names: names.to_owned(),
        }
    }

    #[test]
    fn a_batch_expands_against_the_names_in_use_and_its_identities_do_not_change_its_fingerprint() {
        let typed = batch(&[1, 2, 3], "0 2*3000", "Ground");
        let levels = typed.levels(&used(&["Ground 2"])).unwrap();
        assert_eq!(
            levels
                .iter()
                .map(|l| (l.new_id, l.name.as_str(), l.elevation_mm))
                .collect::<Vec<_>>(),
            [
                (id(1), "Ground", 0.0),
                (id(2), "Ground 3", 3000.0),
                (id(3), "Ground 4", 6000.0)
            ]
        );
        assert_eq!(
            typed.content_fingerprint().unwrap(),
            batch(&[7, 8, 9], " 0 2*3000 ", "Ground")
                .content_fingerprint()
                .unwrap()
        );
        assert_ne!(
            typed.content_fingerprint().unwrap(),
            batch(&[1, 2, 3], "0 2*3100", "Ground")
                .content_fingerprint()
                .unwrap()
        );
        let code = |b: CreateLevels| b.validate_arguments().unwrap_err().code;
        assert_eq!(code(batch(&[1], "0 3000", "")), ErrorCode::InvalidArgument);
        assert_eq!(code(batch(&[1, 1], "0 3000", "")), ErrorCode::DuplicateId);
        assert_eq!(code(batch(&[], "", "")), ErrorCode::InvalidArgument);
        assert_eq!(
            code(batch(&[1, 2], "0 3000", "A\u{7}")),
            ErrorCode::InvalidArgument
        );
    }
}
