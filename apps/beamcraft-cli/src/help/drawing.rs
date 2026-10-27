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
    "--store MODEL_ROOT --document-dir DIR --expected-head UUID (same IDs); publish-drawing ",
    "--document-dir DIR --expected-head UUID --op-id UUID --revision-id UUID --actor-id UUID ",
    "--version-id UUID; inspect-drawing --document-dir DIR [--model-store ROOT]; ",
    "inspect-drawing-versions --document-dir DIR; export-drawing --document-dir DIR ",
    "--version-id UUID --out NEW_FILE.svg [--format svg|pdf --sheet a4|a3|a2|a1|a0] ",
    "(one exact published version from its own archived ",
    "source; deterministic bytes; pdf is a print-ready centred 1:1 sheet page that refuses ",
    "instead of rescaling; refuses an existing destination; the live model is not read).",
);
