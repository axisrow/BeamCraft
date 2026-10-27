// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Full UUIDv7 identities (F04 section 2) and a deterministic generator.
//!
//! Every identity is a complete 128-bit UUID in canonical lowercase `8-4-4-4-12` form. Version 7
//! and the RFC 9562 variant are validated; a shortened, upper-case, braced, other-version or
//! other-variant text is [`IdError`] (`INVALID_ID`). Nothing in this module abbreviates an
//! identity, derives one from another or uses it as an authority: clock order gives neither
//! rights nor conflict priority.
//!
//! [`UuidV7Generator`] follows [RFC 9562 section 5.7](https://www.rfc-editor.org/rfc/rfc9562.html#section-5.7)
//! with the "dedicated counter bits" method of section 6.2: the 12 `rand_a` bits are a counter,
//! so identities made by one generator are strictly increasing even inside one millisecond. The
//! module reads no clock and no randomness (it is a `domain` module that must also build for
//! `wasm32`): the caller injects a Unix-millisecond reading and ten entropy bytes. A caller that
//! has no better source supplies per-process random bytes; uniqueness across processes then
//! rests on 62 random bits per identity plus the actual uniqueness check that F04 requires at
//! creation time (`DUPLICATE_ID`).

use crate::error::ErrorCode;
use core::fmt;

/// The text or number is not a full canonical UUIDv7, or a clock or counter is exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdError;

impl IdError {
    /// The F04 code of this failure: `INVALID_ID`.
    pub const fn code(self) -> ErrorCode {
        ErrorCode::InvalidId
    }
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a full canonical UUIDv7")
    }
}

impl core::error::Error for IdError {}

/// Largest Unix millisecond a UUIDv7 can hold (48 bits).
pub const MAX_UNIX_MS: u64 = (1 << 48) - 1;

const HEX: &[u8; 16] = b"0123456789abcdef";

/// A validated UUIDv7: the 16 big-endian bytes. Ordering is the numeric (and canonical ASCII)
/// order, which is the creation order for one generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uuid7([u8; 16]);

impl Uuid7 {
    /// Parses the canonical `8-4-4-4-12` lowercase form and checks version 7 and the RFC variant.
    pub fn parse(text: &str) -> Result<Self, IdError> {
        let bytes = text.as_bytes();
        if bytes.len() != 36 {
            return Err(IdError);
        }
        let mut out = [0u8; 16];
        let mut nibble = 0usize;
        for (i, b) in bytes.iter().enumerate() {
            if matches!(i, 8 | 13 | 18 | 23) {
                if *b != b'-' {
                    return Err(IdError);
                }
                continue;
            }
            let value = match *b {
                b'0'..=b'9' => *b - b'0',
                b'a'..=b'f' => *b - b'a' + 10,
                _ => return Err(IdError),
            };
            let slot = out.get_mut(nibble / 2).ok_or(IdError)?;
            *slot = if nibble.is_multiple_of(2) {
                value << 4
            } else {
                *slot | value
            };
            nibble += 1;
        }
        Self::from_bytes(out)
    }

    /// Validates 16 raw big-endian bytes: version 7 and the RFC variant.
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, IdError> {
        if bytes.get(6).map(|b| b >> 4) != Some(7) || bytes.get(8).map(|b| b >> 6) != Some(2) {
            return Err(IdError);
        }
        Ok(Self(bytes))
    }

    /// The 16 raw big-endian bytes.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// The 128-bit value.
    pub const fn as_u128(&self) -> u128 {
        u128::from_be_bytes(self.0)
    }

    /// The 48-bit Unix millisecond timestamp. It is a creation hint, never an authority.
    pub fn unix_ms(&self) -> u64 {
        self.0
            .iter()
            .take(6)
            .fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
    }

    /// All 32 hexadecimal digits without hyphens, the form of a USD prim name suffix.
    pub fn hex32(&self) -> String {
        let mut out = String::with_capacity(32);
        for b in self.0 {
            out.push(char::from(*HEX.get(usize::from(b >> 4)).unwrap_or(&b'0')));
            out.push(char::from(*HEX.get(usize::from(b & 15)).unwrap_or(&b'0')));
        }
        out
    }
}

impl fmt::Display for Uuid7 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

macro_rules! typed_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid7);

        impl $name {
            /// Parses the full canonical UUIDv7 text; anything else is `INVALID_ID`.
            pub fn parse(text: &str) -> Result<Self, IdError> {
                Uuid7::parse(text).map(Self)
            }
            /// Wraps an already validated UUIDv7.
            pub const fn from_uuid(uuid: Uuid7) -> Self {
                Self(uuid)
            }
            /// The underlying UUIDv7.
            pub const fn uuid(&self) -> Uuid7 {
                self.0
            }
            /// The 16 raw big-endian bytes.
            pub const fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

typed_id!(
    /// Identity of an owned element (F04 section 2). Its prim path is derived from it.
    ElementId
);
typed_id!(
    /// Identity of a project.
    ProjectId
);
typed_id!(
    /// Identity of a workspace.
    WorkspaceId
);
typed_id!(
    /// Identity of one atomic logical command.
    OpId
);
typed_id!(
    /// Identity of an acting user, taken from the trusted caller context (F04 section 6). It is
    /// a recorded author, never a credential.
    ActorId
);
typed_id!(
    /// Identity of one immutable accepted project revision, never a shortened timestamp or an
    /// array index. It is distinct from the content digest of the revision's manifest.
    RevisionId
);
typed_id!(
    /// Identity of an associative drawing, separate from its source model's workspace.
    DrawingId
);
typed_id!(
    /// Identity of a view owned by a drawing.
    DrawingViewId
);
typed_id!(
    /// Identity of an associative dimension owned by a drawing.
    DimensionId
);
typed_id!(
    /// Identity of one drawn object owned by a drawing document (D07). It never names anything
    /// in the engineering model.
    DraftingEntityId
);
typed_id!(
    /// Identity of an immutable published drawing version, separate from its source revision.
    DocumentVersionId
);

impl ElementId {
    /// The canonical USD path, using the immutable category and the complete UUIDv7.
    pub fn prim_path(&self, category: crate::prim::ElementCategory) -> String {
        format!("{}{}", category.prefix(), self.0.hex32())
    }
}

/// Deterministic UUIDv7 generator with injected time and entropy.
///
/// Identities from one generator are strictly increasing: inside a millisecond the 12-bit counter
/// grows, and if it would overflow or the injected clock stands still or goes back, the generator
/// moves its own logical millisecond forward instead of repeating or shortening anything.
#[derive(Debug, Clone, Default)]
pub struct UuidV7Generator {
    last_ms: u64,
    counter: u16,
    started: bool,
}

impl UuidV7Generator {
    /// A generator that has produced nothing.
    pub const fn new() -> Self {
        Self {
            last_ms: 0,
            counter: 0,
            started: false,
        }
    }

    /// Makes the next identity.
    ///
    /// `unix_ms` is the caller's clock reading in milliseconds since the Unix epoch.
    /// `entropy[0..2]` seeds the counter at the start of a millisecond (only 11 bits are used,
    /// which leaves headroom for 2048 identities before the logical millisecond advances) and
    /// `entropy[2..10]` fills the 62 random bits. Fails with [`IdError`] only if the 48-bit
    /// timestamp would overflow.
    pub fn next(&mut self, unix_ms: u64, entropy: [u8; 10]) -> Result<Uuid7, IdError> {
        let [c0, c1, r0, r1, r2, r3, r4, r5, r6, r7] = entropy;
        let seed = u16::from_be_bytes([c0, c1]) & 0x07ff;
        if unix_ms > MAX_UNIX_MS {
            return Err(IdError);
        }
        if !self.started || unix_ms > self.last_ms {
            self.last_ms = unix_ms.max(self.last_ms);
            self.counter = seed;
            self.started = true;
        } else if self.counter < 0x0fff {
            self.counter += 1;
        } else {
            self.last_ms = self
                .last_ms
                .checked_add(1)
                .filter(|m| *m <= MAX_UNIX_MS)
                .ok_or(IdError)?;
            self.counter = seed;
        }
        let ms = self.last_ms.to_be_bytes();
        let [_, _, m0, m1, m2, m3, m4, m5] = ms;
        let [k0, k1] = self.counter.to_be_bytes();
        Uuid7::from_bytes([
            m0,
            m1,
            m2,
            m3,
            m4,
            m5,
            0x70 | (k0 & 0x0f),
            k1,
            0x80 | (r0 & 0x3f),
            r1,
            r2,
            r3,
            r4,
            r5,
            r6,
            r7,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const GOOD: &str = "01990000-0000-7000-8000-000000000001";

    #[test]
    fn parses_and_prints_the_canonical_form() {
        let id = ElementId::parse(GOOD).unwrap();
        assert_eq!(id.to_string(), GOOD);
        assert_eq!(id.as_bytes().len(), 16);
        assert_eq!(
            id.prim_path(crate::prim::ElementCategory::Part),
            "/World/Steel/Parts/PRT_01990000000070008000000000000001"
        );
        let later = ElementId::parse("01990000-0000-7000-8000-00000000000a").unwrap();
        assert!(id < later);
        assert_eq!(id.uuid().unix_ms(), 0x0199_0000_0000);
    }

    #[test]
    fn the_prim_path_has_all_32_digits() {
        let id = ElementId::parse("0199e7a0-1234-7abc-bdef-0123456789ab").unwrap();
        assert_eq!(
            id.prim_path(crate::prim::ElementCategory::Part),
            "/World/Steel/Parts/PRT_0199e7a012347abcbdef0123456789ab"
        );
        assert_eq!(id.uuid().hex32().len(), 32);
    }

    #[test]
    fn rejects_every_malformed_id() {
        for bad in [
            "",
            "01990000-0000-7000-8000-00000000000",
            "01990000-0000-7000-8000-0000000000012",
            "01990000_0000-7000-8000-000000000001",
            "01990000-0000-7000-8000-00000000000A",
            "01990000-0000-7000-8000-00000000000g",
            "01990000-0000-4000-8000-000000000001",
            "01990000-0000-7000-c000-000000000001",
            "01990000-0000-7000-7000-000000000001",
            "{01990000-0000-7000-8000-000000000001}",
            "０1990000-0000-7000-8000-000000000001",
            "019900000000700080000000000000001",
            "019a7bf00000",
        ] {
            assert_eq!(
                ElementId::parse(bad).unwrap_err().code(),
                ErrorCode::InvalidId
            );
            assert!(Uuid7::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn raw_bytes_need_version_7_and_the_rfc_variant() {
        let mut bytes = *Uuid7::parse(GOOD).unwrap().as_bytes();
        assert!(Uuid7::from_bytes(bytes).is_ok());
        bytes[6] = 0x40;
        assert!(Uuid7::from_bytes(bytes).is_err());
        bytes[6] = 0x70;
        bytes[8] = 0xc0;
        assert!(Uuid7::from_bytes(bytes).is_err());
    }

    #[test]
    fn typed_ids_are_distinct_types_with_the_same_text_form() {
        assert_eq!(OpId::parse(GOOD).unwrap().to_string(), GOOD);
        assert_eq!(RevisionId::parse(GOOD).unwrap().to_string(), GOOD);
        assert_eq!(
            ProjectId::parse(GOOD).unwrap().uuid(),
            WorkspaceId::parse(GOOD).unwrap().uuid()
        );
    }

    fn entropy(seed: u8) -> [u8; 10] {
        [seed, seed, 1, 2, 3, 4, 5, 6, 7, seed]
    }

    #[test]
    fn identities_in_one_millisecond_are_complete_distinct_and_increasing() {
        let mut generator = UuidV7Generator::new();
        let mut seen = BTreeSet::new();
        let mut previous = None;
        for i in 0..10_000u32 {
            // The same millisecond and the same entropy: only the counter may differ.
            let id = generator.next(1_790_000_000_000, entropy(0x11)).unwrap();
            let text = id.to_string();
            assert_eq!(Uuid7::parse(&text).unwrap(), id, "round trip {i}");
            assert_eq!(text.len(), 36);
            assert!(seen.insert(id));
            if let Some(p) = previous {
                assert!(p < id, "strictly increasing at {i}");
            }
            previous = Some(id);
        }
        assert_eq!(seen.len(), 10_000);
    }

    #[test]
    fn a_stopped_or_backward_clock_never_repeats() {
        let mut generator = UuidV7Generator::new();
        let a = generator.next(5_000, entropy(1)).unwrap();
        let b = generator.next(4_000, entropy(1)).unwrap();
        let c = generator.next(5_000, entropy(1)).unwrap();
        assert!(a < b && b < c);
        assert_eq!(b.unix_ms(), 5_000);
    }

    #[test]
    fn counter_overflow_advances_the_logical_millisecond() {
        let mut generator = UuidV7Generator::new();
        let mut last = generator
            .next(7, [0x07, 0xff, 0, 0, 0, 0, 0, 0, 0, 0])
            .unwrap();
        for _ in 0..5000 {
            let next = generator
                .next(7, [0x07, 0xff, 0, 0, 0, 0, 0, 0, 0, 0])
                .unwrap();
            assert!(last < next);
            last = next;
        }
        assert!(last.unix_ms() > 7);
    }

    #[test]
    fn a_new_millisecond_reseeds_and_different_entropy_differs() {
        let mut a = UuidV7Generator::new();
        let mut b = UuidV7Generator::new();
        let x = a.next(10, entropy(1)).unwrap();
        let y = b.next(10, entropy(2)).unwrap();
        assert_ne!(x, y);
        assert_eq!(x.unix_ms(), y.unix_ms());
        let later = a.next(11, entropy(1)).unwrap();
        assert_eq!(later.unix_ms(), 11);
        assert!(x < later);
    }

    #[test]
    fn the_timestamp_overflow_is_an_error_not_a_wrap() {
        let mut generator = UuidV7Generator::new();
        assert!(generator.next(MAX_UNIX_MS + 1, entropy(0)).is_err());
        let top = generator
            .next(MAX_UNIX_MS, [0x07, 0xff, 0, 0, 0, 0, 0, 0, 0, 0])
            .unwrap();
        assert_eq!(top.unix_ms(), MAX_UNIX_MS);
        let mut last = top;
        let mut failed = false;
        for _ in 0..5000 {
            match generator.next(MAX_UNIX_MS, [0x07, 0xff, 0, 0, 0, 0, 0, 0, 0, 0]) {
                Ok(next) => {
                    assert!(last < next);
                    last = next;
                }
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed, "an exhausted 48-bit clock must refuse");
    }

    #[test]
    fn the_layout_matches_rfc_9562_section_5_7() {
        let mut generator = UuidV7Generator::new();
        let id = generator
            .next(
                0x0123_4567_89ab,
                [0, 5, 0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88],
            )
            .unwrap();
        let bytes = id.as_bytes();
        assert_eq!(&bytes[..6], &[0x01, 0x23, 0x45, 0x67, 0x89, 0xab]);
        assert_eq!(bytes[6] >> 4, 7);
        assert_eq!(u16::from_be_bytes([bytes[6] & 0x0f, bytes[7]]), 5);
        assert_eq!(bytes[8] >> 6, 0b10);
        assert_eq!(bytes[8] & 0x3f, 0x3f);
        assert_eq!(&bytes[9..], &[0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88]);
    }
}
