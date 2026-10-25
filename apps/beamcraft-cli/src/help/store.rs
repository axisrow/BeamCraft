// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the store command family.

pub(super) const HELP: &str = concat!(
    "USD is read and written in this process by the pure-Rust layer engine under its size and ",
    "depth budgets; no other process is started. commit-revision verifies blobs but not USD ",
    "content; create-plate verifies the USD content (U03 index, structural check, kernel ",
    "re-evaluation).",
);
