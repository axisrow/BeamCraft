// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the bom command family.

pub(super) const HELP: &str = concat!(
    "BOM (E03): export-bom --store ROOT --out ELEMENTS_CSV [--summary SUMMARY_CSV] (the ",
    "verified head's bill of materials as two CSV files: one row per plate, beam and column ",
    "and one summary group per kind, profile and material; the mark is the element's approved ",
    "numbering mark (A09), or the element id for an element without one; a mass without a ",
    "density stays an empty cell and is counted in mass_unknown, never zero; both files are ",
    "reserved before anything is written, so a refusal on either path leaves nothing on disk)",
);
