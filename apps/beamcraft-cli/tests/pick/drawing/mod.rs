// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The D02–D07 drawing suites as ordinary submodules of the `pick` suite: every file in this
//! directory is declared below by a plain `mod` line and shares the `pick` helpers through
//! `use super::*;`.

use super::*;

mod archive;
mod derived;
mod drafting;
mod export;
mod lifecycle;
mod projection;
mod sections;
mod sheet;
mod views;
