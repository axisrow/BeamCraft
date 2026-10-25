// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the blobs command family.

pub(super) const COMMANDS: &[&str] = &["put-blob", "verify-blob"];

pub(super) const HELP: &str = concat!(
    "Blobs: put-blob --store ROOT --file FILE (stores the bytes under their SHA-256); ",
    "verify-blob --store ROOT --digest SHA256",
);
