// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The 22-character `IfcGloballyUniqueId` (E01 section 3).
//!
//! `E(u)` writes the 128 bits of a UUID as 22 base-64 digits, most significant first: the first
//! digit holds the top two bits (so it is `0` to `3`) and each other digit six bits. The alphabet
//! is `0-9 A-Z a-z _ $`. Entities that are not elements get a synthetic id from a SHA-256 of a
//! fixed role and fixed-width keys ([`synthetic`]).

use beamcraft_model::digest::Sha256;
use beamcraft_model::id::Uuid7;
use core::fmt;

/// The digit alphabet, value 0 to 63.
pub const ALPHABET: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";

/// Domain separation prefix of a synthetic GlobalId preimage.
const SYNTHETIC_PREFIX: &[u8] = b"BCIFC-GUID";

/// A GlobalId: 22 ASCII characters of [`ALPHABET`], the first one `0` to `3`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlobalId([u8; 22]);

impl GlobalId {
    /// The text of the id.
    pub fn as_str(&self) -> &str {
        // Every byte is an ASCII character of the alphabet by construction.
        core::str::from_utf8(&self.0).unwrap_or_default()
    }

    /// The 22 characters.
    pub const fn as_bytes(&self) -> &[u8; 22] {
        &self.0
    }
}

impl fmt::Display for GlobalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A text that is not an `IfcGloballyUniqueId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuidError;

impl fmt::Display for GuidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a 22-character IfcGloballyUniqueId")
    }
}

impl core::error::Error for GuidError {}

/// The encoding `E` of 16 raw bytes (big-endian, first byte most significant).
pub fn encode_bytes(bytes: &[u8; 16]) -> GlobalId {
    let n = u128::from_be_bytes(*bytes);
    let mut out = [b'0'; 22];
    for (i, slot) in out.iter_mut().enumerate() {
        let shift = 126u32.saturating_sub(6 * i as u32);
        let digit = ((n >> shift) & 63) as usize;
        *slot = ALPHABET.get(digit).copied().unwrap_or(b'0');
    }
    GlobalId(out)
}

/// The GlobalId of an element or level identity (`IFCX-GUID-001`).
pub fn encode(id: &Uuid7) -> GlobalId {
    encode_bytes(id.as_bytes())
}

/// The decoder `D` (`IFCX-GUID-003`): exactly 22 characters of the alphabet, the first `0` to
/// `3`. Anything else is an error. The result is the 128-bit value, not yet an ElementId.
pub fn decode(text: &str) -> Result<[u8; 16], GuidError> {
    let bytes = text.as_bytes();
    if bytes.len() != 22 {
        return Err(GuidError);
    }
    let mut n: u128 = 0;
    for (i, b) in bytes.iter().enumerate() {
        let digit = ALPHABET.iter().position(|a| a == b).ok_or(GuidError)? as u128;
        if i == 0 && digit > 3 {
            return Err(GuidError);
        }
        n = (n << 6) | digit;
    }
    Ok(n.to_be_bytes())
}

/// A synthetic GlobalId: the first 16 bytes of `SHA-256("BCIFC-GUID" 0x00 role 0x00 key...)`
/// encoded with `E` (`IFCX-GUID-004`). Each key must have a fixed width for its role, so the
/// preimage is unambiguous.
pub fn synthetic(role: &str, keys: &[&[u8]]) -> GlobalId {
    let mut hasher = Sha256::new();
    // The preimage is a few hundred bytes at most, far below the hasher's length limit; an
    // overflow would leave the digest of what was absorbed so far, which still is deterministic.
    let _ = hasher.update(SYNTHETIC_PREFIX);
    let _ = hasher.update(&[0]);
    let _ = hasher.update(role.as_bytes());
    let _ = hasher.update(&[0]);
    for key in keys {
        let _ = hasher.update(key);
    }
    let digest = hasher.finalize();
    let mut head = [0u8; 16];
    for (slot, byte) in head.iter_mut().zip(digest.as_bytes()) {
        *slot = *byte;
    }
    encode_bytes(&head)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(text: &str) -> [u8; 16] {
        let hex: Vec<u8> = text.bytes().filter(|b| *b != b'-').collect();
        let mut out = [0u8; 16];
        for (slot, pair) in out.iter_mut().zip(hex.chunks(2)) {
            let value = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
            *slot = value(pair[0]) << 4 | value(pair[1]);
        }
        out
    }

    #[test]
    fn the_encoder_vectors_of_the_contract() {
        let cases = [
            (
                "00000000-0000-0000-0000-000000000000",
                "0000000000000000000000",
            ),
            (
                "ffffffff-ffff-ffff-ffff-ffffffffffff",
                "3$$$$$$$$$$$$$$$$$$$$$",
            ),
            (
                "80000000-0000-0000-0000-000000000000",
                "2000000000000000000000",
            ),
            (
                "00000000-0000-0000-0000-00000000003f",
                "000000000000000000000$",
            ),
            (
                "00000000-0000-0000-0000-000000000040",
                "0000000000000000000010",
            ),
            (
                "019a7bf0-0000-7000-8000-000000200001",
                "01cdlm001m080000008001",
            ),
            (
                "019a7bf0-0000-7000-8000-000000000005",
                "01cdlm001m080000000005",
            ),
            (
                "01234567-89ab-7def-bfed-cba987654321",
                "018qLdYQjzxx$jowc7PKCX",
            ),
            (
                "0197c3a8-5e21-7b4f-9a06-3d8c1e72f4b9",
                "01byEeNY5xJve6FOmUSlIv",
            ),
            (
                "019a7bf0-0000-7000-8000-000000000024",
                "01cdlm001m08000000000a",
            ),
            (
                "019a7bf0-0000-7000-8000-00000000003e",
                "01cdlm001m08000000000_",
            ),
        ];
        for (uuid, expected) in cases {
            let bytes = raw(uuid);
            assert_eq!(encode_bytes(&bytes).as_str(), expected, "{uuid}");
            assert_eq!(decode(expected), Ok(bytes), "{uuid}");
        }
    }

    #[test]
    fn element_ids_encode_through_uuid7() {
        let id = Uuid7::parse("019a7bf0-0000-7000-8000-000000200001").unwrap();
        assert_eq!(encode(&id).as_str(), "01cdlm001m080000008001");
    }

    #[test]
    fn the_decoder_is_strict() {
        for bad in [
            "",
            "01cdlm001m08000000000",
            "01cdlm001m0800000000051",
            "01cdlm001m0800000000+5",
            "01cdlm001m0800000000/5",
            "01cdlm001m080000000005=",
            "41cdlm001m080000008001",
            "01cdlm001m08000000000\u{e9}",
        ] {
            assert_eq!(decode(bad), Err(GuidError), "{bad:?}");
        }
    }

    #[test]
    fn decoding_inverts_encoding_over_a_spread_of_values() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for _ in 0..2000 {
            let mut bytes = [0u8; 16];
            for chunk in bytes.chunks_mut(8) {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                chunk.copy_from_slice(&state.to_be_bytes());
            }
            assert_eq!(decode(encode_bytes(&bytes).as_str()), Ok(bytes));
        }
    }

    #[test]
    fn the_synthetic_vectors_of_the_contract() {
        let project = raw("019a7bf0-0000-7000-8000-000000000001");
        let level = raw("019a7bf0-0000-7000-8000-000000000101");
        assert_eq!(
            synthetic("site", &[&project]).as_str(),
            "2ghTuGpW1EXNtW6xLArrhQ"
        );
        assert_eq!(
            synthetic("storey", &[&project, &level]).as_str(),
            "2886jimw3h7h75lUh8nRj7"
        );
    }

    #[test]
    fn ascii_order_is_not_numeric_order() {
        let larger = encode_bytes(&raw("00000000-0000-0000-0000-00000000003e"));
        let smaller = encode_bytes(&raw("00000000-0000-0000-0000-000000000024"));
        // The larger number ends in `_` (0x5F), which sorts before `a` (0x61).
        assert!(larger.as_str() < smaller.as_str());
    }
}
