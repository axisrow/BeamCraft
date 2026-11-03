// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the pick command family.

pub(super) const HELP: &str = concat!(
    "Pick (V04, engine queries over the verified head's view snapshot; read-only; FeatureIds ",
    "fid1:<element>:plate|member:<role> exist for box bodies only, others are listed as ",
    "skipped; an index is never an address): pick --store ROOT (--origin X,Y,Z --direction ",
    "X,Y,Z [--t-max MM] [--include-backfaces true|false] | --point X,Y,Z) --targets ",
    "face,edge,vertex [--radius MM] [--max-hits 1..64] [--exclude parts,assemblies,bolts,",
    "welds,holes,grids,construction_objects,items] (the selection switches: a class that is ",
    "off is never a hit, nothing is written); list-feature --store ROOT --id UUID ",
    "(all qualified semantic box features at the exact model pin; unsupported bodies refuse); ",
    "lookup-feature --store ROOT --feature-id FID (kind, owner, local and world geometry); ",
    "element-edges --store ROOT --id UUIDV7 (evaluated world-space boundary edges without triangle diagonals or a bounding-box substitute); ",
    "snap --store ROOT --point X,Y,Z [--radius MM] [--kinds vertex,axis_end,axis,edge,midpoint,center,grid_intersection,grid_end,grid_line] [--max-hits ",
    "1..64] [--plane OX,OY,OZ,XX,XY,XZ,YX,YY,YZ] [--origin X,Y,Z --direction DX,DY,DZ [--aperture TAN_HALF_ANGLE | --cursor-radius MM]] [--reference-snaps true|false] [--geometry-snaps true|false] [--grid-snaps true|false] [--nearest true|false] (cursor-radius is the constant orthographic aperture; source and point-kind gates apply together before truncation; nearest=false excludes arbitrary points on every line family while retaining requested discrete points; grids also require reference-snaps)",
);
