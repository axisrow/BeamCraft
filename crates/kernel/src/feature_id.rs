// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F06 section 5 local semantic roles; element qualification belongs to the engine.

/// Face table in the F06 mesh order, independent of dimensions.
pub const PLATE_FACES: [&str; 6] = [
    "plate:f.x0",
    "plate:f.x1",
    "plate:f.y0",
    "plate:f.y1",
    "plate:f.z0",
    "plate:f.z1",
];

/// The twelve topological edges, with axis roles in x, y, z order.
pub const PLATE_EDGES: [&str; 12] = [
    "plate:e.x0.y0",
    "plate:e.x0.y1",
    "plate:e.x1.y0",
    "plate:e.x1.y1",
    "plate:e.x0.z0",
    "plate:e.x0.z1",
    "plate:e.x1.z0",
    "plate:e.x1.z1",
    "plate:e.y0.z0",
    "plate:e.y0.z1",
    "plate:e.y1.z0",
    "plate:e.y1.z1",
];

/// Semantic vertex ids in bit order i=xb+2*yb+4*zb.
pub const PLATE_VERTICES: [&str; 8] = [
    "plate:v.x0.y0.z0",
    "plate:v.x1.y0.z0",
    "plate:v.x0.y1.z0",
    "plate:v.x1.y1.z0",
    "plate:v.x0.y0.z1",
    "plate:v.x1.y0.z1",
    "plate:v.x0.y1.z1",
    "plate:v.x1.y1.z1",
];

/// Vertex endpoints of each topological edge in [`PLATE_EDGES`] order.
pub const PLATE_EDGE_VERTICES: [[u32; 2]; 12] = [
    [0, 4],
    [2, 6],
    [1, 5],
    [3, 7],
    [0, 2],
    [4, 6],
    [1, 3],
    [5, 7],
    [0, 1],
    [4, 5],
    [2, 3],
    [6, 7],
];

/// Member faces in the same body-role order as their plate counterparts.
pub const MEMBER_FACES: [&str; 6] = [
    "member:f.x0",
    "member:f.x1",
    "member:f.y0",
    "member:f.y1",
    "member:f.z0",
    "member:f.z1",
];

/// Member edges in the same body-role order as their plate counterparts.
pub const MEMBER_EDGES: [&str; 12] = [
    "member:e.x0.y0",
    "member:e.x0.y1",
    "member:e.x1.y0",
    "member:e.x1.y1",
    "member:e.x0.z0",
    "member:e.x0.z1",
    "member:e.x1.z0",
    "member:e.x1.z1",
    "member:e.y0.z0",
    "member:e.y0.z1",
    "member:e.y1.z0",
    "member:e.y1.z1",
];

/// Member vertices in the same body-role order as their plate counterparts.
pub const MEMBER_VERTICES: [&str; 8] = [
    "member:v.x0.y0.z0",
    "member:v.x1.y0.z0",
    "member:v.x0.y1.z0",
    "member:v.x1.y1.z0",
    "member:v.x0.y0.z1",
    "member:v.x1.y0.z1",
    "member:v.x0.y1.z1",
    "member:v.x1.y1.z1",
];
