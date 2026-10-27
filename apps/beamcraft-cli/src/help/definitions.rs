// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the definitions command family.

pub(super) const COMMANDS: &[&str] = &[
    "define-material",
    "define-profile",
    "define-fastener",
    "definitions",
];

pub(super) const HELP: &str = concat!(
    "Definitions (P02, immutable USD assets pinned by full digest; a material without ",
    "--density keeps an unknown density, so mass stays unknown): define-material --store ROOT ",
    "[--density KG_M3] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] ",
    "[--workspace-id UUID]; define-profile --store ROOT --width MM --height MM [same ids] ",
    "(rectangle section); define-profile --store ROOT --family ",
    "i_shape|channel|angle|rhs|chs|lipped_c|plain_c|lipped_z|omega --h MM --b MM ... (A07.1: ",
    "one --<parameter> per family parameter: i_shape/channel h,b,tw,tf,r; angle/rhs/plain_c ",
    "h,b,t,r; chs d,t; lipped_c/lipped_z/omega h,b,c,t,r; r is the root, outer (rhs) or inner ",
    "bend radius, 0 is sharp) [--designation TEXT] [--standard TEXT] [--area-mm2 F] ",
    "[--mass-per-meter-kg F] [--surface-per-meter-m2 F] [same ids]; define-profile --store ",
    "ROOT --family custom --outer X,Y;X,Y;... [--cavity X,Y;...|X,Y;...] (outer ",
    "counter-clockwise, cavities clockwise, kept as authored); define-fastener --store ROOT ",
    "--fastener-library FILE --designation TEXT [--name TEXT] [same ids] (A14.1: every value ",
    "comes from one fastener library entry, so the definition never needs the file again; an ",
    "unknown designation is refused); definitions --store ROOT ",
    "(verified head: digest, kind, units, values, plates that pin it). A definition already ",
    "in the head is not rewritten (definition_exists). create-plate --material-digest pins an ",
    "existing material.",
);
