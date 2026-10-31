// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the drawing command family.

pub(super) const PREVIEW: &str = concat!(
    "Drawing: preview-drawing --store ROOT --id MEMBER_UUID --drawing-id UUID --view-id UUID ",
    "--dimension-id UUID --anchor-a FEATURE_ID --anchor-b FEATURE_ID [--plane ",
    "front|top|right] [--scale 100] [--origin X,Y] [--offset MM] [--decimals 0..3] [--name ",
    "TEXT] [--intent-out NEW_FILE.usda] (model is read-only; projected model-mm dimension; ",
    "unresolved is explicit)",
);

pub(super) const DOCUMENTS: &str = concat!(
    "Drawing documents (D03): create-drawing --store MODEL_ROOT --document-dir NEW_DIR (with ",
    "drawing/view/dimension/member/anchor IDs, op/revision/actor IDs); refresh-drawing ",
    "--store MODEL_ROOT --document-dir DIR --expected-head UUID (same IDs); edit-drafting ",
    "--document-dir DIR --expected-head UUID --op-id UUID --revision-id UUID --actor-id UUID ",
    "[--layer UUID:name:on|off] [--drafting UUID:kind:x,y,...:layer:linetype[:payload]] (D07; ",
    "layers and drawn objects replace the sets together, kinds line, ",
    "polyline, arc, circle, rectangle, text, hatch, symbol, free_dimension, where every object ",
    "names a layer the same replacement defines and a continuous|dashed|dotted|dashdot ",
    "linetype; text takes anchor,height,rotation plus the payload string, hatch takes outline ",
    "points plus a solid|diagonal|crosshatch payload, symbol takes anchor,scale plus a ",
    "north|section|detail|elevation payload, and free_dimension takes ",
    "x1,y1,x2,y2,offset,decimals; the model store is not read); edit-views --document-dir DIR ",
    "--expected-head UUID --op-id UUID --revision-id UUID --actor-id UUID [--placed-view ",
    "UUID:plane:scale:x,y] [--placed-section UUID:plane:cut:side:scale:x,y] [--placed-detail ",
    "UUID:parent:x,y:radius:scale:x,y] (D04; placed views, sections and details replace their ",
    "sets together in one document operation; plane front|top|right; a section cuts the model ",
    "plane perpendicular to its basis at cut along the basis's depth axis with side ",
    "positive|negative visible; a detail magnifies the primary or placed parent view's circular ",
    "region center,radius at the detail's own scale; the model store is not read; ",
    "create/refresh accept the placement options but ignore them, and a refresh drops existing ",
    "placements); edit-sheet --document-dir DIR --expected-head UUID ",
    "--op-id UUID --revision-id UUID --actor-id UUID (--sheet-size a4|a3|a2|a1|a0 ",
    "--sheet-project TEXT --sheet-author TEXT [--sheet-frame MM] lays the document on that ",
    "sheet, or --sheet-remove true takes it off; D05; the model store is not read; ",
    "create/refresh accept the same sheet options); publish-drawing ",
    "--document-dir DIR --expected-head UUID --op-id UUID --revision-id UUID --actor-id UUID ",
    "--version-id UUID; inspect-drawing --document-dir DIR [--model-store ROOT]; ",
    "inspect-drawing-versions --document-dir DIR; export-drawing --document-dir DIR ",
    "--version-id UUID --out NEW_FILE.svg [--format svg|pdf --sheet a4|a3|a2|a1|a0] ",
    "(one exact published version from its own archived ",
    "source; deterministic bytes; a sheet document renders its frame, title block and every ",
    "placed view, a loose document keeps the tight-bounds export; pdf is a print-ready ",
    "centred 1:1 sheet page that refuses instead of rescaling and defaults to the ",
    "document's sheet size; refuses an existing destination; the live model is not read).",
);
