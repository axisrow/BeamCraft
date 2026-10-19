// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Pure wire-level rules of the private worker protocol (F05 0.1.0, draft).
//!
//! This crate holds only what the contract fixes in bytes and numbers: the 16-byte frame header,
//! the 44-byte binary attachment prefix, the negotiable limits with their maxima, the bundle size
//! budget and the `IPC_` transport codes with their F04 mapping. It performs no I/O, owns no
//! process or pipe, parses no JSON and spawns nothing; the native host does that (see
//! `docs/contracts/module-boundaries-v0.1.md`). Every decoder validates before it trusts a length,
//! uses checked arithmetic and never indexes or slices by an input-derived position.
//!
//! U04 addition: the original framing-only description above predates `json` and `message`,
//! which now decode bounded experimental read-profile control values without performing I/O.

pub mod code;
pub mod frame;
pub mod json;
pub mod limits;
pub mod message;
pub mod refresh;

pub use code::TransportCode;
pub use frame::{BinaryPrefix, BundleBudget, FrameError, FrameHeader, FrameKind};
pub use limits::Limits;
