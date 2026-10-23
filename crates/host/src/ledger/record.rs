// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The ledger line format: one closed, canonical, checksummed text line per record.
//!
//! ```text
//! L1 genesis <head sequence>                                         <sha256>
//! L1 intent <op> <project> <revision> <sequence> <base|none>
//!    <content fingerprint> <created unix s> <attempt sha256> <n> <layer sha256>*n  <sha256>
//! L1 committed <op> <revision> <sequence> <manifest sha256>          <sha256>
//! L1 aborted <op> <intent id> <reason>                               <sha256>
//! ```
//!
//! Each record is one line (shown wrapped above) that ends with a single `\n`. The last token
//! is the SHA-256 of every byte before the final space, so a torn, truncated or flipped record
//! never parses. A parsed record is re-encoded and must reproduce the input exactly: there is
//! one byte form per record. The checksum is an integrity check against accidents, not an
//! authentication of the writer: whoever can write the file can forge a valid line (the same
//! namespace-protection requirement as the blob store).

use std::fmt::Write as _;

use beamcraft_model::digest::{ContentDigest, Sha256};

use crate::revision::{MAX_ENTRIES, Uuid7};

/// Longest record line the parser accepts, including its newline.
pub const MAX_RECORD_BYTES: usize = 8192;
const VERSION: &str = "L1";
// At most MAX_ENTRIES USD layers plus the optional operation-evidence blob of manifest v2.
const MAX_INTENT_BLOBS: usize = MAX_ENTRIES + 1;

/// The write-ahead record of one commit attempt, written and synced before anything of the
/// attempt becomes visible in the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    /// Operation that asked for the commit; the retry key.
    pub op: Uuid7,
    /// Project of the revision.
    pub project: Uuid7,
    /// Revision id the attempt will publish.
    pub revision: Uuid7,
    /// Head-log sequence the attempt will take: the root's sequence plus one.
    pub sequence: u64,
    /// Revision that was the root when the attempt started (`None` for the first revision).
    pub base: Option<Uuid7>,
    /// Fingerprint of the content the op id is bound to
    /// ([`Content::fingerprint`](crate::revision::Content::fingerprint)).
    pub fingerprint: ContentDigest,
    /// Seconds since the Unix epoch when the attempt started; used only to tell a crashed
    /// attempt from one that is still running (see [`RecoveryPolicy`](super::RecoveryPolicy)).
    pub created: u64,
    /// Identity of this one attempt: a digest over process, clock and a process-wide counter, so
    /// two identical commits started in the same second still have different intents and one
    /// attempt's closing record can never close another's.
    pub attempt: ContentDigest,
    /// Digests of the layer blobs in manifest order, followed by the optional operation-evidence
    /// digest. Both are protected while this intent is unresolved.
    pub blobs: Vec<ContentDigest>,
}

/// Why an attempt is recorded as not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortReason {
    /// A [`CommitHook`](crate::revision::CommitHook) cancelled before the root switch.
    Cancelled,
    /// The commit failed validation or I/O before the root switch.
    Rejected,
    /// Another committer took the sequence first and it was not this operation.
    LostRace,
    /// Recovery proved that the root never switched to this attempt.
    Recovered,
}

impl AbortReason {
    /// Stable ledger spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Rejected => "rejected",
            Self::LostRace => "lost-race",
            Self::Recovered => "recovered",
        }
    }

    /// Parses the stable spelling.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "cancelled" => Some(Self::Cancelled),
            "rejected" => Some(Self::Rejected),
            "lost-race" => Some(Self::LostRace),
            "recovered" => Some(Self::Recovered),
            _ => None,
        }
    }
}

/// The record that closes an operation as accepted: the root holds its revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedRecord {
    /// Operation id.
    pub op: Uuid7,
    /// Revision it created.
    pub revision: Uuid7,
    /// Head-log sequence of that revision.
    pub sequence: u64,
    /// Digest of the revision's manifest blob.
    pub manifest: ContentDigest,
}

/// One ledger record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// First record of a ledger: the head-log sequence that existed before the ledger did.
    /// Operations at or below it were never journaled, so their absence proves nothing.
    Genesis {
        /// Head-log sequence (0 when no revision existed).
        head: u64,
    },
    /// Write-ahead record of one commit attempt.
    Intent(Intent),
    /// The operation is accepted and its revision is in the root.
    Committed(CommittedRecord),
    /// One attempt is closed as not accepted.
    Aborted {
        /// Operation id.
        op: Uuid7,
        /// Checksum of the intent line this closes (its identity).
        intent: ContentDigest,
        /// Why.
        reason: AbortReason,
    },
}

/// Canonical unsigned decimal (no sign, no leading zero, at most 20 digits).
fn decimal(text: &str) -> Option<u64> {
    let canonical = !text.is_empty()
        && text.len() <= 20
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if canonical { text.parse().ok() } else { None }
}

impl Record {
    fn body(&self) -> String {
        match self {
            Self::Genesis { head } => format!("{VERSION} genesis {head}"),
            Self::Intent(i) => {
                let base = i.base.as_ref().map_or("none", Uuid7::as_str);
                let mut text = format!(
                    "{VERSION} intent {} {} {} {} {base} {} {} {} {}",
                    i.op,
                    i.project,
                    i.revision,
                    i.sequence,
                    i.fingerprint,
                    i.created,
                    i.attempt,
                    i.blobs.len()
                );
                for digest in &i.blobs {
                    // Writing to a String cannot fail.
                    let _ = write!(text, " {digest}");
                }
                text
            }
            Self::Committed(c) => format!(
                "{VERSION} committed {} {} {} {}",
                c.op, c.revision, c.sequence, c.manifest
            ),
            Self::Aborted { op, intent, reason } => {
                format!("{VERSION} aborted {op} {intent} {}", reason.as_str())
            }
        }
    }

    /// The complete record line with checksum and newline, and its identity: the checksum.
    /// An [`Aborted`](Self::Aborted) record names the intent it closes by this identity.
    pub fn encode(&self) -> (Vec<u8>, ContentDigest) {
        let body = self.body();
        // A record body is far below the SHA-256 length domain; the fallback is never used.
        let sum = Sha256::digest(body.as_bytes()).unwrap_or(ContentDigest::from_bytes([0; 32]));
        (format!("{body} {sum}\n").into_bytes(), sum)
    }

    /// Parses one record line (without its newline). `None` for anything that is not exactly
    /// the canonical encoding of a valid record: bad checksum, wrong field, extra bytes.
    pub fn parse(line: &str) -> Option<(Self, ContentDigest)> {
        if line.len() >= MAX_RECORD_BYTES {
            return None;
        }
        let (body, sum) = line.rsplit_once(' ')?;
        let sum = sum.parse::<ContentDigest>().ok()?;
        if Sha256::digest(body.as_bytes()).ok()? != sum {
            return None;
        }
        let mut tokens = body.split(' ');
        if tokens.next()? != VERSION {
            return None;
        }
        let uuid = |token: Option<&str>| token.and_then(|t| Uuid7::parse(t).ok());
        let digest = |token: Option<&str>| token.and_then(|t| t.parse::<ContentDigest>().ok());
        let record = match tokens.next()? {
            "genesis" => Self::Genesis {
                head: decimal(tokens.next()?)?,
            },
            "intent" => {
                let op = uuid(tokens.next())?;
                let project = uuid(tokens.next())?;
                let revision = uuid(tokens.next())?;
                let sequence = decimal(tokens.next()?)?;
                let base = match tokens.next()? {
                    "none" => None,
                    other => Some(uuid(Some(other))?),
                };
                let fingerprint = digest(tokens.next())?;
                let created = decimal(tokens.next()?)?;
                let attempt = digest(tokens.next())?;
                let count = usize::try_from(decimal(tokens.next()?)?).ok()?;
                if count == 0 || count > MAX_INTENT_BLOBS {
                    return None;
                }
                let mut blobs = Vec::with_capacity(count);
                for _ in 0..count {
                    blobs.push(digest(tokens.next())?);
                }
                Self::Intent(Intent {
                    op,
                    project,
                    revision,
                    sequence,
                    base,
                    fingerprint,
                    created,
                    attempt,
                    blobs,
                })
            }
            "committed" => Self::Committed(CommittedRecord {
                op: uuid(tokens.next())?,
                revision: uuid(tokens.next())?,
                sequence: decimal(tokens.next()?)?,
                manifest: digest(tokens.next())?,
            }),
            "aborted" => Self::Aborted {
                op: uuid(tokens.next())?,
                intent: digest(tokens.next())?,
                reason: AbortReason::parse(tokens.next()?)?,
            },
            _ => return None,
        };
        if tokens.next().is_some() || record.body() != body {
            return None;
        }
        Some((record, sum))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn id(n: u32) -> Uuid7 {
        Uuid7::parse(&format!("0199e7a0-0000-7000-8000-{n:012x}")).unwrap()
    }
    fn digest(text: &str) -> ContentDigest {
        Sha256::digest(text.as_bytes()).unwrap()
    }
    fn intent(blobs: usize) -> Intent {
        Intent {
            op: id(1),
            project: id(2),
            revision: id(3),
            sequence: 7,
            base: Some(id(4)),
            fingerprint: digest("content"),
            created: 1_700_000_000,
            attempt: digest("attempt"),
            blobs: (0..blobs).map(|n| digest(&format!("blob {n}"))).collect(),
        }
    }
    fn samples() -> Vec<Record> {
        vec![
            Record::Genesis { head: 0 },
            Record::Genesis { head: u64::MAX },
            Record::Intent(intent(1)),
            Record::Intent(intent(MAX_ENTRIES)),
            Record::Intent(intent(MAX_INTENT_BLOBS)),
            Record::Intent(Intent {
                base: None,
                ..intent(2)
            }),
            Record::Committed(CommittedRecord {
                op: id(1),
                revision: id(3),
                sequence: 7,
                manifest: digest("manifest"),
            }),
            Record::Aborted {
                op: id(1),
                intent: digest("intent"),
                reason: AbortReason::Recovered,
            },
        ]
    }

    #[test]
    fn every_record_round_trips_and_stays_inside_the_line_cap() {
        for record in samples() {
            let (bytes, sum) = record.encode();
            assert!(bytes.len() < MAX_RECORD_BYTES);
            assert_eq!(bytes.last(), Some(&b'\n'));
            let line = std::str::from_utf8(&bytes[..bytes.len() - 1]).unwrap();
            assert_eq!(Record::parse(line), Some((record, sum)));
        }
    }

    #[test]
    fn every_proper_prefix_and_every_single_byte_change_is_rejected() {
        for record in samples() {
            let (bytes, _) = record.encode();
            let line = &bytes[..bytes.len() - 1];
            for cut in 0..line.len() {
                if let Ok(text) = std::str::from_utf8(&line[..cut]) {
                    assert_eq!(Record::parse(text), None, "prefix {cut}");
                }
            }
            for at in 0..line.len() {
                let mut changed = line.to_vec();
                changed[at] = if changed[at] == b'0' { b'1' } else { b'0' };
                let text = String::from_utf8(changed).unwrap();
                assert_eq!(Record::parse(&text), None, "flip at {at}");
            }
        }
    }

    #[test]
    fn closed_fields_reject_noncanonical_text() {
        let good = Record::Genesis { head: 3 }.body();
        let resign = |body: &str| {
            let sum = Sha256::digest(body.as_bytes()).unwrap();
            format!("{body} {sum}")
        };
        for body in [
            good.replace("genesis 3", "genesis 03"),
            good.replace("genesis 3", "genesis +3"),
            good.replace("L1", "L2"),
            format!("{good} extra"),
            "L1 genesis".to_owned(),
            "L1 unknown 3".to_owned(),
            "L1 genesis 99999999999999999999999".to_owned(),
        ] {
            assert_eq!(Record::parse(&resign(&body)), None, "{body}");
        }
        assert!(Record::parse(&resign(&good)).is_some());
        // An intent with zero or too many blobs, a bad reason and a short UUID are refused even
        // though their checksum is valid.
        let mut zero = intent(1);
        zero.blobs.clear();
        let mut many = intent(1);
        many.blobs = (0..=MAX_INTENT_BLOBS)
            .map(|n| digest(&n.to_string()))
            .collect();
        for body in [Record::Intent(zero).body(), Record::Intent(many).body()] {
            assert_eq!(Record::parse(&resign(&body)), None);
        }
        let aborted = Record::Aborted {
            op: id(1),
            intent: digest("i"),
            reason: AbortReason::Cancelled,
        }
        .body();
        assert_eq!(
            Record::parse(&resign(&aborted.replace("cancelled", "forgotten"))),
            None
        );
        assert_eq!(
            Record::parse(&resign(
                &aborted.replace("0199e7a0-0000-7000-8000-000000000001", "1")
            )),
            None
        );
        assert_eq!(Record::parse(&"x".repeat(MAX_RECORD_BYTES)), None);
    }
}
