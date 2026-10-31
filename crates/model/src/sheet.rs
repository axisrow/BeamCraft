// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D05 sheet entities of a drawing document: one fixed paper size, its frame and the
//! title block fields. A sheet is pure bounded intent — no geometry, no renderer and no
//! model reference lives here; the engine lays the admitted values out on the paper.

use crate::command::{CommandError, label_is_valid};
use crate::error::ErrorCode;

/// Smallest frame inset in millimetres; the frame must stay a visible rectangle.
pub const MIN_FRAME_INSET_MM: f64 = 1.0;

/// Fixed international sheet sizes in portrait orientation, in increasing area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sheet {
    /// 210 × 297 mm.
    A4,
    /// 297 × 420 mm.
    A3,
    /// 420 × 594 mm.
    A2,
    /// 594 × 841 mm.
    A1,
    /// 841 × 1189 mm.
    A0,
}

impl Sheet {
    /// Portrait sheet size in millimetres, width then height.
    pub fn size_mm(self) -> [f64; 2] {
        match self {
            Self::A4 => [210.0, 297.0],
            Self::A3 => [297.0, 420.0],
            Self::A2 => [420.0, 594.0],
            Self::A1 => [594.0, 841.0],
            Self::A0 => [841.0, 1189.0],
        }
    }

    /// Exact lowercase wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A4 => "a4",
            Self::A3 => "a3",
            Self::A2 => "a2",
            Self::A1 => "a1",
            Self::A0 => "a0",
        }
    }

    /// Parses a fixed sheet token; every other size is refused, never approximated.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "a4" => Some(Self::A4),
            "a3" => Some(Self::A3),
            "a2" => Some(Self::A2),
            "a1" => Some(Self::A1),
            "a0" => Some(Self::A0),
            _ => None,
        }
    }
}

/// The drawing frame: one uniform inset from every sheet edge, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SheetFrame {
    /// Distance from each sheet edge to the frame line.
    pub inset_mm: f64,
}

/// Authored title block fields. The drawing name and the primary view scale are derived
/// from the document at render time and are deliberately not fields here.
#[derive(Debug, Clone, PartialEq)]
pub struct TitleBlock {
    /// Project name under the model's bounded label rule.
    pub project: String,
    /// Author name under the same rule; attribution, never authorization.
    pub author: String,
}

/// One sheet of a document: chosen size, frame and title block. A document carries at
/// most one sheet; absent means the loose unbounded paper of the earlier contract.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetSpec {
    /// Fixed catalogue size; arbitrary millimetre sizes are refused.
    pub size: Sheet,
    /// Frame inset from every sheet edge.
    pub frame: SheetFrame,
    /// Authored title block fields.
    pub title: TitleBlock,
}

impl SheetSpec {
    /// Admits one sheet: supported size, a frame strictly inside the sheet and bounded
    /// title block labels. Refuses with no partial result.
    pub fn admit(&self) -> Result<(), CommandError> {
        let invalid = |field: &'static str, message: &'static str| {
            CommandError::new(ErrorCode::InvalidArgument, field, message)
        };
        let [width, height] = self.size.size_mm();
        if !self.frame.inset_mm.is_finite() {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "frame_inset_mm",
                "frame inset must be finite",
            ));
        }
        if self.frame.inset_mm < MIN_FRAME_INSET_MM
            || self.frame.inset_mm * 2.0 >= width.min(height)
        {
            return Err(invalid(
                "frame_inset_mm",
                "the frame must leave a visible interior on the chosen sheet",
            ));
        }
        for (field, label) in [
            ("project", &self.title.project),
            ("author", &self.title.author),
        ] {
            if !label_is_valid(label) {
                return Err(invalid(
                    field,
                    "title block field must satisfy the bounded label rule",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet() -> SheetSpec {
        SheetSpec {
            size: Sheet::A4,
            frame: SheetFrame { inset_mm: 10.0 },
            title: TitleBlock {
                project: "Bridge".to_owned(),
                author: "Drafter".to_owned(),
            },
        }
    }

    #[test]
    fn sheet_sizes_are_fixed_and_tokens_round_trip() {
        assert_eq!(Sheet::A4.size_mm(), [210.0, 297.0]);
        assert_eq!(Sheet::A0.size_mm(), [841.0, 1189.0]);
        for sheet in [Sheet::A4, Sheet::A3, Sheet::A2, Sheet::A1, Sheet::A0] {
            assert_eq!(Sheet::parse(sheet.as_str()), Some(sheet));
        }
        // Unsupported sizes refuse; nothing is approximated onto the catalogue.
        for hostile in ["b4", "letter", "a4-landscape", "", "A4"] {
            assert_eq!(Sheet::parse(hostile), None);
        }
    }

    #[test]
    fn a_supported_sheet_admits_and_bad_frames_or_labels_refuse() {
        sheet().admit().unwrap();
        let mut wide = sheet();
        wide.frame.inset_mm = 104.0;
        assert!(wide.admit().is_ok());
        wide.frame.inset_mm = 105.0;
        // Two insets of 105 exactly consume the 210 mm width: no interior, refused.
        assert_eq!(wide.admit().unwrap_err().field, "frame_inset_mm");
        wide.frame.inset_mm = f64::NAN;
        assert_eq!(wide.admit().unwrap_err().code, ErrorCode::NonFiniteNumber);
        let mut labeled = sheet();
        labeled.title.author.clear();
        assert_eq!(labeled.admit().unwrap_err().field, "author");
        labeled.title.project = "a\u{7f}b".to_owned();
        assert_eq!(labeled.admit().unwrap_err().field, "project");
    }
}
