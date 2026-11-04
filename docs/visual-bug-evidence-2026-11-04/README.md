# CLI visual bug evidence, 2026-11-04

Visual evidence for findings in the headless CLI's drawing and report outputs. Every
finding here is visible in an exported file; nothing is inferred from code alone. The
desktop application was out of scope. This directory changes no application code.

## Audited build

- BeamCraft `0.0.1`, unmodified commit `b614bb871e7c9d94d247e53b9b0d201a3213cfe6`
  (`master` on 2026-11-04), built with `cargo build --locked -p beamcraft-cli --bins`
  (debug profile, rustc 1.97.0) on Linux x86_64.
- SVG and HTML were rendered with headless Chromium (Playwright 1.63.0, Chromium build
  1243); PDF pages with `pdftoppm` (Poppler 24.02.0) at 100, 200 and 600 dpi.
- The drawings carry no font: the CLI draws every glyph as strokes. The HTML schedule
  was rendered with Liberation Serif 2.1.5 (`OFL-1.1`) embedded for the capture, so no
  other system font appears. Panel captions use Liberation Sans 2.1.5 (`OFL-1.1`).

## Reproduce

```sh
./reproduce.sh /path/to/beamcraft OUT_DIR
```

The script builds one synthetic model (a 200 x 300 mm column 2500 mm high, a beam 3000 mm
long and two plates, one of them 100.1 x 100.1 x 10 mm), three drawing documents and one
schedule, and writes `col.svg`, `col.pdf`, `sheet.svg`, `sheet.pdf`, `glyphs.svg`,
`schedule.html`, `schedule.csv` and `bom.csv`. Every dimension, name and identity is
invented; no catalogue or client data is used. The CLI refuses a relative `--store` path,
so the script resolves `OUT_DIR` to an absolute path.

## How to read the images

Each image is a captioned composite of renders of the files above. A panel marked
**flipped** is the CLI's SVG wrapped by hand in `matrix(1 0 0 -1 0 420)` (a vertical flip
about the A3 sheet height). It shows the intended layout for comparison, so the findings
that remain after the orientation is corrected (5, 6 and 7) can be seen on their own. The
red outlines in image 05 were added afterwards at the coordinates given to
`edit-drafting`; everything else is unretouched render output, cropped and scaled.

## Findings

Code references are permalinks to the audited commit.

| # | Finding | Image | Where |
| --- | --- | --- | --- |
| 1 | The SVG export is mirrored vertically: the 2500 mm dimension reads **5200**, a title block `SCALE 1:20` reads **1:50**, the title block sits at the top and all text is upside down. The PDF of the same version is upright. | [01](01-svg-mirrored-2500-reads-5200.png), [02](02-sheet-svg-upside-down.png), [03](03-title-block-scale-reads-1-50.png) | Paper coordinates are Y-up ([`projection.rs:103`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/projection.rs#L103)); the SVG writers emit them unflipped ([`svg.rs:507`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/svg.rs#L507), [`sheet.rs:283`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/sheet.rs#L283)) into SVG's Y-down space |
| 2 | The PDF of a sheet document renders only the primary view: no frame, title block, placed view, section, detail or view titles. `docs/design/d05-drawing-sheets.md` says every export surface renders the same fixed sheet. | [04](04-pdf-drops-sheet-content.png) | The `pdf` branch passes only the primary projection to `print::export` ([`drawing_cmd.rs:1104`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/apps/beamcraft-cli/src/drawing_cmd.rs#L1104)) |
| 3 | Objects added with `edit-drafting` on a visible layer (text, line, circle, rectangle; the line is drawn towards the text as a leader would be) appear in neither the SVG nor the PDF, although `inspect-drawing` lists them under `visible_drafting`. | [05](05-drafting-not-rendered.png) | No exporter reads [`Drawing::visible_drafting`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing.rs#L587) |
| 4 | A vertical dimension's label is struck through by its own dimension line ("25\|00"), in SVG and PDF. | [06](06-dimension-line-through-label.png) | The label is always horizontal and centred on the line's midpoint ([`svg.rs:266`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/svg.rs#L266)) |
| 5 | View titles overlap the lowest edge of their view (ELEVATION, PLAN, SECTION A-A, DETAIL B). | [07](07-view-title-overlaps-geometry.png) | The 3.0 mm gap is smaller than the 3.5 mm glyph height ([`sheet.rs:34`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/sheet.rs#L34)) |
| 6 | The title block has no top edge; its column divider sits 30 mm from the right edge instead of after the label column and strikes through an accepted 40-character project name. | [08](08-title-block-layout.png) | [`sheet.rs:152-163`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/sheet.rs#L152-L163) |
| 7 | The letters B, S and O are drawn like the digits 8, 5 and 0: "B1 B8" reads "81 88", "S5" reads "55". | [09](09-letters-look-like-digits.png) | Glyphs [`B`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/svg.rs#L136), [`O`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/svg.rs#L203), [`S`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/svg.rs#L229) in `svg.rs` |
| 8 | `schedule-export` (HTML and CSV) and `export-bom` print raw floating-point noise: `10020.009999999998`, `0.7865707849999998`. | [10](10-report-float-noise.png) | [`bom.rs:257`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/bom.rs#L257) formats with `{value}`; JSON already carries rounded `*_display` fields |
| 9 | The colon glyph is a single dot, so "1:20" reads "1·20". | [03](03-title-block-scale-reads-1-50.png), [09](09-letters-look-like-digits.png) | [`svg.rs:260`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/svg.rs#L260) |

Two text-only findings need no image: `COMMANDS.md` documents `--decimals 0..255` on
`preview-drawing`, `create-drawing` and `refresh-drawing` (from
[`registry.rs:2300`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/commands/src/registry.rs#L2300)),
while the commands refuse 4 and above with "use zero through three decimal places"; and
`beamcraft create-plate --help` answers with the JSON refusal "options are KEY VALUE pairs"
instead of help.

Checked and not counted as findings: A3 is portrait by contract; IFC export encodes
non-ASCII names correctly (`\X2\...\X0\`); characters without a glyph (Cyrillic, `/`, `(`)
advance a blank gap. No contract documents that gap; it is intended exporter behaviour stated
only in the doc comments of
[`glyph`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/svg.rs#L63-L64)
and
[`text_strokes`](https://github.com/axisrow/BeamCraft/blob/b614bb871e7c9d94d247e53b9b0d201a3213cfe6/crates/engine/src/drawing/svg.rs#L311-L314)
in `svg.rs`.

## Checksums (SHA-256)

```text
6f634f2b4083d20a3732910f2ac47e786250480b4b8ebc450ebc2bca94e6b677  01-svg-mirrored-2500-reads-5200.png
baf670da7ae55be54246aed42dfb96d8418579914fd2622e1084a6d3e8f535ee  02-sheet-svg-upside-down.png
4a71a6ec331c55aca0a292b2f721154b5ec9d98113ce4317a8a4deb98579604c  03-title-block-scale-reads-1-50.png
07ce8cc9cbff4e2ff366dc79a39ed41e74699826ae98350df7e23d5ca76583dc  04-pdf-drops-sheet-content.png
545ec1f9e16654d9d74da986873a98091149fe3fade06a232fbcc0c698a78f7f  05-drafting-not-rendered.png
3ae7bbe09ef337fa8bfce89c930d3e126e557918204296c2fc636df7acae00f3  06-dimension-line-through-label.png
ae682bfd2d9d35ac3da32e4dc0772e4608bb9555a41636065c35498a330010a3  07-view-title-overlaps-geometry.png
006a9be3a75014fc93abc3e73a65037c1522d9408c671e2718c12ed08bcc50ab  08-title-block-layout.png
e0d0710208eedf3664930219eba9dd28f06ae03e5951daa3cac0071576fa8704  09-letters-look-like-digits.png
807e0fae3596f9444f30c3e9883bd0450bc48621803ca164c719c17e6ff29a64  10-report-float-noise.png
```
