// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the pick command family.

pub(super) const HELP: &str = concat!(
    "Features (V04, engine queries over the verified head's view snapshot; read-only; FeatureIds ",
    "fid1:<element>:plate|member:<role> exist for box bodies only, others are listed as ",
    "skipped; an index is never an address): list-feature --store ROOT --id UUID ",
    "(all qualified semantic box features at the exact model pin; unsupported bodies refuse); ",
    "lookup-feature --store ROOT --feature-id FID (kind, owner, local and world geometry); ",
    "element-edges --store ROOT --id UUIDV7 (evaluated world-space boundary edges without ",
    "triangle diagonals or a bounding-box substitute)",
);
