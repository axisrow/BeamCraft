# D05: drawing sheets

Scope: fork #229 (epic #224, roadmap M4). One sheet per document: chosen catalogue size,
frame, title block fields filled from authored attributes, several views side by side, and
the UI surface that moves and aligns them by hand. Baseline: verified fork `master`
`e7e7a436` (2026-10-31), which already carries D04's placed views.

## Sheet entities

The sheet data model lives in `beamcraft_model::sheet`, pure bounded intent: the fixed
A4–A0 portrait catalogue (moved here from the D08 print pipeline, whose `print::Sheet`
path re-exports it so both call sites keep one catalogue), a uniform frame inset
(`SheetFrame`, at least 1 mm and strictly inside the sheet) and two authored title block
fields (`TitleBlock`). Sizes outside the catalogue refuse — there is no nearest-size
substitution and no arbitrary millimetre paper. The drawing name and the primary view's
scale are derived from the document at render time and are deliberately not fields.

`DrawingSpec.sheet` is optional: absent keeps the loose unbounded paper of the earlier
contract, present, every export surface renders the same fixed sheet. The sheet holds no
UUID — it is furniture, not a document object — so admission gains no identity constraints,
only one more optional scope in the one-page budget (`MAX_INTENT_PAGE_ITEMS` stays 64 and
now binds documents whose sheet, views and drawn objects together exceed it). `Drawing::
with_sheet` / `without_sheet` are whole-document intent operations, exactly like
`with_views`.

## Whole-sheet output

`drawing::sheet::export_svg` renders one exact published version as stroke-only SVG bound
to its version id: frame, title block, every resolved placed view in document order and the
primary view's dimension groups. Text is contributor-original glyph strokes — the digit
glyphs of D03 extended with angular capitals on the same 2.0 × 3.5 box, with ASCII
lowercase folded onto its capitals and a drawn dot, so admissible title text keeps every
mark — and no font, image or `<text>` element is embedded. The same sheet reproduces
byte-identical output across runs and processes; anything unresolved, beyond the frame, or
over the byte cap refuses instead of being rescaled or clipped. A document without a sheet
keeps the D03 loose-paper export, and the pdf print defaults its sheet to the document's
sheet size.

## Wire format and CLI

The canonical intent gains one optional `Scope "Sheet"` (size token, frame inset, project,
author); the schema descriptor gains the sheet entries in place, without a version bump. The
CLI mirrors `edit-views` with `edit-sheet`
(set/replace, or `--sheet-remove true`), `create-drawing`/`refresh-drawing` accept the
sheet options, and `inspect-drawing` reports `name`, `view` and `sheet` beside the placed
views. `export-drawing` renders a sheet document through the whole-sheet export.

## UI sheet editor

`apps/beamcraft-ui/src/sheet.rs` is the hand-placement surface: it reads the document
through the same `inspect-drawing` command a user runs, shows paper, frame, title block and
one marker per view, and applies moved origins through the same `edit-views` command with
fresh identities from the shared `IdSource`. Dropping a marker within the 0.5 mm tolerance
of another view's axis snaps it onto that axis. The UI computes no engineering geometry;
the reproducible output is the engine's. Visual acceptance stays with the owner's eyes.
