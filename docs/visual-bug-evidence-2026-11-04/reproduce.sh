#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# SPDX-FileCopyrightText: 2026 Apibim
#
# Synthetic reproducer for the CLI drawing and report findings recorded in README.md.
# Usage: ./reproduce.sh /path/to/beamcraft OUT_DIR
# Every dimension, name and identity below is invented; no catalogue or client data.
# Open the produced files side by side: col.svg/col.pdf, sheet.svg/sheet.pdf,
# glyphs.svg, linetypes.svg/linetypes.pdf and schedule.html (also schedule.csv and bom.csv).
set -euo pipefail
B=$1
OUT=$(mkdir -p "$2" && cd "$2" && pwd)   # the CLI refuses a relative --store path
S=$OUT/store
mkdir -p "$S"
id() { printf '019a7bf0-0000-7000-8000-%012d' "$1"; }
U=$(id 1); COL=$(id 10); BEAM=$(id 20); L=$(id 900)
q() { "$B" "$@" >/dev/null; }

q define-material --store "$S" --grade S355 --density 7850
q define-profile --store "$S" --designation R200x300 --width 200 --height 300
q create-member --store "$S" --new-id "$COL" --kind column --start 0,0,0 --end 0,0,2500 \
  --profile R200x300 --material S355 --name C1
q create-member --store "$S" --new-id "$BEAM" --kind beam --start 0,0,0 --end 3000,0,0 \
  --profile R200x300 --material S355 --name B1
q create-plate --store "$S" --thickness 12 --width 333.3 --length 250 --material S355 --name PL1
q create-plate --store "$S" --thickness 10 --width 100.1 --length 100.1 --material S355 --name PL2

# Findings 1 and 4: a loose (sheet-less) drawing of the 2500 mm column.
D=$OUT/col-doc
q create-drawing --store "$S" --document-dir "$D" --op-id "$(id 101)" --revision-id "$(id 102)" \
  --actor-id "$U" --id "$COL" --drawing-id "$(id 103)" --view-id "$(id 104)" \
  --dimension-id "$(id 105)" --anchor-a "fid1:$COL:member:v.x0.y0.z0" \
  --anchor-b "fid1:$COL:member:v.x0.y0.z1" --name "Column 2500" --scale 20 --offset 10
q publish-drawing --document-dir "$D" --expected-head "$(id 102)" --op-id "$(id 106)" \
  --revision-id "$(id 107)" --actor-id "$U" --version-id "$(id 108)"
q export-drawing --document-dir "$D" --version-id "$(id 108)" --out "$OUT/col.svg"
q export-drawing --document-dir "$D" --version-id "$(id 108)" --out "$OUT/col.pdf" --format pdf

# Findings 1, 2, 3, 5, 6 and 9: an A3 sheet with a placed view, a section, a detail and
# four drawn objects on a visible layer.
D=$OUT/sheet-doc
q create-drawing --store "$S" --document-dir "$D" --op-id "$(id 201)" --revision-id "$(id 202)" \
  --actor-id "$U" --id "$BEAM" --drawing-id "$(id 203)" --view-id "$(id 204)" \
  --dimension-id "$(id 205)" --anchor-a "fid1:$BEAM:member:v.x0.y0.z0" \
  --anchor-b "fid1:$BEAM:member:v.x0.y0.z1" --name "Beam B1" --view-title ELEVATION \
  --scale 20 --origin 40,250 --offset 15
q edit-views --document-dir "$D" --expected-head "$(id 202)" --op-id "$(id 206)" \
  --revision-id "$(id 207)" --actor-id "$U" \
  --placed-view "$(id 208):top:20:40,330:PLAN" \
  --placed-section "$(id 209):right:1500:positive:5:240,250:SECTION A-A" \
  --placed-detail "$(id 210):$(id 204):40,250:12:5:220,330:DETAIL B"
q edit-sheet --document-dir "$D" --expected-head "$(id 207)" --op-id "$(id 211)" \
  --revision-id "$(id 212)" --actor-id "$U" --sheet-size a3 --sheet-frame 10 \
  --sheet-project "NORTH WAREHOUSE EXTENSION PHASE TWO 2026" --sheet-author "J. Smith"
q edit-drafting --document-dir "$D" --expected-head "$(id 212)" --op-id "$(id 213)" \
  --revision-id "$(id 214)" --actor-id "$U" --layer "$L:Notes:on" \
  --drafting "$(id 901):text:30,80,5,0:$L:continuous:MARK B-1 (2 PCS)" \
  --drafting "$(id 902):line:60,95,90,150:$L:continuous" \
  --drafting "$(id 903):circle:200,110,10:$L:dashed" \
  --drafting "$(id 904):rectangle:25,70,120,95:$L:continuous"
q publish-drawing --document-dir "$D" --expected-head "$(id 214)" --op-id "$(id 215)" \
  --revision-id "$(id 216)" --actor-id "$U" --version-id "$(id 217)"
q export-drawing --document-dir "$D" --version-id "$(id 217)" --out "$OUT/sheet.svg"
q export-drawing --document-dir "$D" --version-id "$(id 217)" --out "$OUT/sheet.pdf" --format pdf

# Finding 7: letters next to the digits they are drawn like, in the title block.
D=$OUT/glyphs-doc
q create-drawing --store "$S" --document-dir "$D" --op-id "$(id 301)" --revision-id "$(id 302)" \
  --actor-id "$U" --id "$BEAM" --drawing-id "$(id 303)" --view-id "$(id 304)" \
  --dimension-id "$(id 305)" --anchor-a "fid1:$BEAM:member:v.x0.y0.z0" \
  --anchor-b "fid1:$BEAM:member:v.x0.y0.z1" --name "B1 B8 81 88" --scale 20 \
  --origin 40,250 --offset 15 --sheet-size a3 --sheet-frame 10 \
  --sheet-project "S5 55 SS O0 00 OO" --sheet-author "Z2 G6 I1"
q publish-drawing --document-dir "$D" --expected-head "$(id 302)" --op-id "$(id 306)" \
  --revision-id "$(id 307)" --actor-id "$U" --version-id "$(id 308)"
q export-drawing --document-dir "$D" --version-id "$(id 308)" --out "$OUT/glyphs.svg"

# Verification, 2026-11-05: every non-continuous line type on a circle and on a rectangle.
D=$OUT/linetypes-doc
T=$(id 950)
q create-drawing --store "$S" --document-dir "$D" --op-id "$(id 401)" --revision-id "$(id 402)" \
  --actor-id "$U" --id "$BEAM" --drawing-id "$(id 403)" --view-id "$(id 404)" \
  --dimension-id "$(id 405)" --anchor-a "fid1:$BEAM:member:v.x0.y0.z0" \
  --anchor-b "fid1:$BEAM:member:v.x0.y0.z1" --name "LINE TYPES" --scale 20 \
  --origin 30,240 --offset 15 --sheet-size a4 --sheet-frame 10 \
  --sheet-project "LINE TYPE CHECK" --sheet-author "QA"
q edit-drafting --document-dir "$D" --expected-head "$(id 402)" --op-id "$(id 406)" \
  --revision-id "$(id 407)" --actor-id "$U" --layer "$T:Types:on" \
  --drafting "$(id 951):circle:45,160,12:$T:dashed" \
  --drafting "$(id 952):circle:105,160,12:$T:dotted" \
  --drafting "$(id 953):circle:165,160,12:$T:dashdot" \
  --drafting "$(id 954):rectangle:30,95,60,125:$T:dashed" \
  --drafting "$(id 955):rectangle:90,95,120,125:$T:dotted" \
  --drafting "$(id 956):rectangle:150,95,180,125:$T:dashdot" \
  --drafting "$(id 957):text:33,180,4,0:$T:continuous:DASHED" \
  --drafting "$(id 958):text:93,180,4,0:$T:continuous:DOTTED" \
  --drafting "$(id 959):text:150,180,4,0:$T:continuous:DASHDOT"
q publish-drawing --document-dir "$D" --expected-head "$(id 407)" --op-id "$(id 408)" \
  --revision-id "$(id 409)" --actor-id "$U" --version-id "$(id 410)"
q export-drawing --document-dir "$D" --version-id "$(id 410)" --out "$OUT/linetypes.svg"
q export-drawing --document-dir "$D" --version-id "$(id 410)" --out "$OUT/linetypes.pdf" --format pdf

# Finding 8: float noise in the human-facing reports.
cat > "$OUT/template.json" <<'JSON'
{"version":1,"columns":[{"column":"name"},{"column":"kind"},{"column":"profile"},
 {"column":"length_mm","total":true},{"column":"area_mm2","total":true},
 {"column":"mass_kg","total":true}],"group_by":["kind"],"sort_by":["name"]}
JSON
q schedule-export --store "$S" --template "$OUT/template.json" --out "$OUT/schedule.csv" \
  --html "$OUT/schedule.html"
q export-bom --store "$S" --out "$OUT/bom.csv"
echo "Done: $OUT"
