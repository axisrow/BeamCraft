// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Presentation-only request admission and bounded JSON transport.
use crate::plate::PlateError;
use beamcraft_json::{Json, Limits};
use beamcraft_model::{error::ErrorCode, id::ElementId};
/// Maximum complete display metadata: 32 MiB with bounded nesting and token count.
pub const LIMITS: Limits = Limits {
    bytes: 32 * 1024 * 1024,
    tokens: 2_097_152,
    depth: 16,
    string_bytes: 4096,
    scratch_bytes: 128 * 1024 * 1024,
};
/// Read a large explicit selection without unbounded command-line arguments.
pub fn selection_file(path: &str) -> Result<Vec<ElementId>, PlateError> {
    let bytes = crate::source_file::read_file(path, 4 * 1024 * 1024)
        .map_err(|c| PlateError::new(c, "selection file is not usable"))?;
    let j = beamcraft_json::decode(&bytes, &LIMITS)
        .map_err(|_| PlateError::new(ErrorCode::InvalidArgument, "invalid selection JSON"))?;
    let a = j.array().map_err(|_| {
        PlateError::new(ErrorCode::InvalidArgument, "selection must be a UUID array")
    })?;
    if a.len() > 65_536 {
        return Err(PlateError::new(
            ErrorCode::LimitExceeded,
            "selection exceeds 65536 IDs",
        ));
    }
    a.iter()
        .map(|v| {
            ElementId::parse(v.string().map_err(|_| {
                PlateError::new(ErrorCode::InvalidArgument, "selection UUID must be text")
            })?)
            .map_err(|_| PlateError::new(ErrorCode::InvalidArgument, "invalid selection UUID"))
        })
        .collect()
}
/// Encode global mode settings; appearance rules may override base colour/transparency later.
pub fn settings(p: &beamcraft_engine::render::Presentation) -> Json {
    Json::object([
        ("mode", Json::text(p.mode.token())),
        ("surfaces", Json::Bool(p.surfaces)),
        ("edges", Json::Bool(p.edges)),
        ("lit", Json::Bool(p.lit)),
        (
            "color",
            Json::Array(
                p.color
                    .into_iter()
                    .map(|n| Json::Number(n.to_string()))
                    .collect(),
            ),
        ),
        ("transparency", Json::Number(p.transparency.to_string())),
    ])
}
