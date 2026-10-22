// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Structural check of the committed macOS icon: the `icns` is a well-formed family covering
//! every size the packager declares, without needing macOS. The archive wiring itself is
//! verified on real macOS by the release workflow's smoke test (PlistBuddy, iconutil,
//! codesign), which is strictly stronger than the retired synthetic rehearsal.

// Test-only file: helper code outside the assertions may panic on setup failures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fs;
use std::path::PathBuf;

/// Pixel size per `icns` element type. `info` and `toc ` carry no pixels; newer iconutil
/// builds encode the smallest two sizes (`ic04`/`ic05`) as raw ARGB instead of PNG.
const ICNS_TYPES: &[(&[u8; 4], u32)] = &[
    (b"icp4", 16),
    (b"icp5", 32),
    (b"icp6", 64),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic09", 512),
    (b"ic10", 1024),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic13", 256),
    (b"ic14", 512),
    (b"ic04", 16),
    (b"ic05", 32),
];

fn big_endian_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().unwrap())
}

fn icon_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("assets")
        .join("icons")
        .join("BeamCraft.icns")
}

#[test]
fn the_committed_icns_covers_every_declared_size() {
    let data = fs::read(icon_path()).unwrap();
    assert_eq!(&data[..4], b"icns", "wrong icns magic");
    assert_eq!(
        big_endian_u32(&data[4..8]),
        data.len() as u32,
        "the declared total length differs from the file"
    );
    let mut sizes = std::collections::BTreeSet::new();
    let mut offset = 8usize;
    while offset < data.len() {
        assert!(
            offset + 8 <= data.len(),
            "truncated element header at {offset}"
        );
        let kind = &data[offset..offset + 4];
        let length = big_endian_u32(&data[offset + 4..offset + 8]) as usize;
        assert!(length >= 8, "element at {offset} declares {length} bytes");
        assert!(
            offset + length <= data.len(),
            "element at {offset} runs past the file"
        );
        let payload = &data[offset + 8..offset + length];
        let declared = ICNS_TYPES.iter().find(|(name, _)| *name == kind);
        if let Some((name, size)) = declared {
            sizes.insert(*size);
            if payload.len() >= 24 && payload[..8] == *b"\x89PNG\r\n\x1a\n" {
                // PNG: width and height sit in the IHDR at a fixed offset, big endian.
                assert_eq!(
                    big_endian_u32(&payload[16..20]),
                    *size,
                    "{:?} PNG width differs from the declared size",
                    name
                );
                assert_eq!(
                    big_endian_u32(&payload[20..24]),
                    *size,
                    "{:?} PNG height differs from the declared size",
                    name
                );
            } else {
                assert!(
                    matches!(*name, b"ic04" | b"ic05"),
                    "{:?} carries neither PNG nor the raw ARGB small-size encoding",
                    name
                );
            }
        } else {
            let known_no_pixels = kind == b"info" || kind == b"toc ";
            assert!(
                known_no_pixels,
                "unknown icns element type {:?} at {offset}",
                String::from_utf8_lossy(kind)
            );
        }
        offset += length;
    }
    assert_eq!(
        sizes,
        [16, 32, 64, 128, 256, 512, 1024].into_iter().collect(),
        "the icon family does not cover every declared size"
    );
}
