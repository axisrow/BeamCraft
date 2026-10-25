// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Puts the application icon into `beamcraft-ui.exe` on Windows. The MSVC linker takes the
//! compiled resource `assets/icons/beamcraft-windows.res` (the images of `beamcraft-windows.ico`)
//! as an input, so Explorer, the taskbar and shortcuts pick the exact icon size
//! from the executable and no resource compiler is needed at build time.

use std::path::Path;

fn main() {
    let res =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/beamcraft-windows.res");
    println!("cargo::rerun-if-changed={}", res.display());
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if windows && msvc {
        println!("cargo::rustc-link-arg-bin=beamcraft-ui={}", res.display());
    }
}
