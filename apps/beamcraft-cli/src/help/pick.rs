// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the pick command family.

pub(super) const COMMANDS: &[&str] = &["pick", "lookup-feature", "snap"];

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
    "snap --store ROOT --point X,Y,Z [--radius MM] [--kinds vertex,axis_end,axis] [--max-hits ",
    "1..64] (box vertices and member axis start, end and nearest point)",
);
