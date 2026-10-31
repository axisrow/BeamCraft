// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The initial operating caps of F04 section 3, in one place.
//!
//! These are original first-slice constants, not vendor tolerances or promises of kernel
//! accuracy. The geometric caps are the `beamcraft_geom::tolerance` constants (F06 section 4);
//! this module adds the ones that belong to commands, labels and footprints. Callers may lower a
//! limit, never raise it.

pub use beamcraft_geom::tolerance::{
    CONTOUR_POINT_CAP, COORDINATE_CAP_MM, DIMENSION_CAP_MM, PLATE_SIZE_CAP_MM, ROLL_CAP_RAD,
};

/// Encoded logical command, in bytes, before parsing.
pub const MAX_COMMAND_BYTES: usize = 1_048_576;
/// Nesting depth of a logical command.
pub const MAX_COMMAND_DEPTH: usize = 16;
/// One string, in UTF-8 bytes, unless a narrower field cap applies.
pub const MAX_STRING_BYTES: usize = 4096;
/// Shortest element label, in UTF-8 bytes.
pub const MIN_LABEL_BYTES: usize = 1;
/// Longest element label, in UTF-8 bytes.
pub const MAX_LABEL_BYTES: usize = 256;
/// Fewest contour points of a plate.
pub const MIN_CONTOUR_POINTS: usize = 3;
/// Largest material density in kg/m3, when authored in a definition.
pub const MAX_DENSITY_KG_M3: f64 = 1_000_000.0;
/// Largest declared section area in mm2 (the square of the dimension cap).
pub const MAX_AREA_MM2: f64 = 1e12;
/// Largest declared mass per metre of a section, in kg/m.
pub const MAX_MASS_PER_METER_KG: f64 = 1_000_000.0;
/// Largest declared surface per metre of a section, in m2/m.
pub const MAX_SURFACE_PER_METER_M2: f64 = 10_000.0;
/// Relative tolerance between a declared section value and the value the profile family's
/// own formula gives. A larger gap is refused, never silently resolved to one of the two.
pub const DECLARED_RELATIVE_TOLERANCE: f64 = 1e-3;
/// Relative gap between a declared area of a section family (A07) and the net area of the
/// generated outline above which a warning is reported: 1 percent. Owner decision D3
/// (2026-10-18): catalogue values are authoritative, so the gap is a warning and never a
/// refusal; the rectangle keeps its refusing [`DECLARED_RELATIVE_TOLERANCE`].
pub const DECLARED_SECTION_AREA_TOLERANCE: f64 = 1e-2;
/// Most relationship targets of one element.
pub const MAX_RELATIONSHIP_TARGETS: usize = 1024;
/// Most derived footprint entries of one command.
pub const MAX_FOOTPRINT_ENTRIES: usize = 4096;
/// Largest deviation of a placement quaternion's squared norm from one.
pub const ORIENTATION_NORM_TOLERANCE: f64 = 1e-6;
/// Largest weld size (A06): the nominal fillet size in millimetres.
pub const WELD_SIZE_CAP_MM: f64 = 1_000.0;
/// Fewest points of a weld path (A15): a straight edge.
pub const MIN_WELD_PATH_POINTS: usize = 2;
/// Most points of a weld path (A15).
pub const MAX_WELD_PATH_POINTS: usize = 1024;
/// Shortest weld path segment (A15), in millimetres: below it a prism segment would fall under
/// the kernel's geometric tolerance.
pub const MIN_WELD_SEGMENT_MM: f64 = 0.001;
/// Largest total weld path length (A15), in millimetres.
pub const MAX_WELD_LENGTH_MM: f64 = 1_000_000.0;
/// Fewest points of a member path (P12); a two-point path is a straight member (P03), so a
/// polybeam command refuses it as an argument.
pub const MIN_MEMBER_PATH_POINTS: usize = 3;
/// Most points of a member path (P12); the kernel's sweep mesh budget refuses earlier.
pub const MAX_MEMBER_PATH_POINTS: usize = 1024;
/// Most copies of one array (T03). An implementation bound that keeps one revision's element
/// creation light; callers may lower it, never raise it.
pub const MAX_ARRAY_COPIES: usize = 100;
/// Largest nominal bolt diameter (A05), in millimetres.
pub const BOLT_DIAMETER_CAP_MM: f64 = 1_000.0;
/// Most bolts of one bolt group (A05).
pub const MAX_BOLT_GROUP_SIZE: usize = 512;
/// Most standard lengths of one fastener definition (A14.1).
pub const MAX_FASTENER_LENGTHS: usize = 64;
/// Largest declared mass of one fastener (bolt, nut and washers), in kg (A14.1).
pub const MAX_FASTENER_MASS_KG: f64 = 10_000.0;
/// A user-attribute name is a USD identifier, at most this many ASCII bytes (I04).
pub const MAX_ATTR_NAME_BYTES: usize = 64;
/// A user-attribute text value or choice option, at most this many UTF-8 bytes, the label
/// rule (I04).
pub const MAX_ATTR_TEXT_BYTES: usize = 256;
/// A user-attribute unit label, at most this many bytes (I04).
pub const MAX_ATTR_UNIT_BYTES: usize = 16;
/// An attribute-set name, at most this many bytes and without control characters (I04).
pub const MAX_ATTR_SET_BYTES: usize = 64;
/// At most this many options on one choice attribute (I04).
pub const MAX_ATTR_OPTIONS: usize = 64;
/// The magnitude cap of a number attribute value (I04).
pub const MAX_ATTR_NUMBER: f64 = 1_000_000_000.0;
/// At most this many values on one element (I04).
pub const MAX_ATTRS_PER_ELEMENT: usize = 256;
/// At most this many attribute definitions in one model (I04).
pub const MAX_ATTRIBUTE_DEFINITIONS: usize = 1024;
