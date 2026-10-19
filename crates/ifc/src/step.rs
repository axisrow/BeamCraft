// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The lexical layer of the STEP physical file (E01 section 11): REAL numbers, strings, UTC time
//! stamps and the bounded, hashing output sink.

use beamcraft_model::digest::{ContentDigest, Sha256};
use std::io::Write;

/// The decimal digits and the scientific exponent of the shortest text that parses back.
fn shortest(scientific: &str) -> Option<(bool, String, i32)> {
    let (negative, body) = match scientific.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, scientific),
    };
    let (mantissa, exponent) = body.split_once('e')?;
    let exponent: i32 = exponent.parse().ok()?;
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    Some((negative, digits, exponent))
}

/// Lays out `digits` (value `d1.d2... * 10^x`) in the notation of section 11.3.
fn layout(negative: bool, digits: &str, x: i32) -> String {
    let mut out = String::with_capacity(digits.len() + 8);
    if negative {
        out.push('-');
    }
    if (-6..=14).contains(&x) {
        if x >= 0 {
            let integral = x as usize + 1;
            if digits.len() <= integral {
                out.push_str(digits);
                out.extend(core::iter::repeat_n('0', integral - digits.len()));
                out.push('.');
            } else {
                let (head, tail) = digits.split_at(integral);
                out.push_str(head);
                out.push('.');
                out.push_str(tail);
            }
        } else {
            out.push_str("0.");
            out.extend(core::iter::repeat_n('0', (-x - 1) as usize));
            out.push_str(digits);
        }
    } else {
        let (head, tail) = digits.split_at(1.min(digits.len()));
        out.push_str(head);
        out.push('.');
        out.push_str(tail);
        out.push('E');
        out.push_str(&x.to_string());
    }
    out
}

/// A binary64 REAL: the shortest decimal that parses back to the same `f64`. `None` for NaN or
/// an infinity, which are never written (`IFCX-DET-003`).
pub fn real64(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    if value == 0.0 {
        return Some("0.".into());
    }
    let (negative, digits, x) = shortest(&format!("{value:e}"))?;
    Some(layout(negative, &digits, x))
}

/// A binary32 REAL: the shortest decimal that parses back to the same `f32`.
pub fn real32(value: f32) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    if value == 0.0 {
        return Some("0.".into());
    }
    let (negative, digits, x) = shortest(&format!("{value:e}"))?;
    Some(layout(negative, &digits, x))
}

/// A STEP string literal with the quotes (`IFCX-DET-004`): apostrophe and backslash doubled, each
/// maximal run of characters outside printable ASCII as one `\X2\...\X0\` or, when it has a
/// character beyond the basic multilingual plane, `\X4\...\X0\` segment.
pub fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    let mut run: Vec<char> = Vec::new();
    for c in text.chars() {
        if (' '..='~').contains(&c) {
            flush_run(&mut out, &mut run);
            match c {
                '\'' => out.push_str("''"),
                '\\' => out.push_str("\\\\"),
                other => out.push(other),
            }
        } else {
            run.push(c);
        }
    }
    flush_run(&mut out, &mut run);
    out.push('\'');
    out
}

fn flush_run(out: &mut String, run: &mut Vec<char>) {
    if run.is_empty() {
        return;
    }
    if run.iter().all(|c| (*c as u32) <= 0xFFFF) {
        out.push_str("\\X2\\");
        for c in run.iter() {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units).iter() {
                out.push_str(&format!("{unit:04X}"));
            }
        }
    } else {
        out.push_str("\\X4\\");
        for c in run.iter() {
            out.push_str(&format!("{:08X}", *c as u32));
        }
    }
    out.push_str("\\X0\\");
    run.clear();
}

/// The UTC calendar time of a Unix millisecond, truncated to whole seconds, as
/// `YYYY-MM-DDTHH:MM:SS` (`IFCX-DET-005`). A pure function: no clock is read.
pub fn utc_timestamp(unix_ms: u64) -> String {
    let seconds = unix_ms / 1000;
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Civil-from-days over the proleptic Gregorian calendar (days since 1970-01-01).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Why a write stopped.
#[derive(Debug)]
pub enum SinkError {
    /// The next write would pass the output cap (`IFCX-LIM-002`).
    CapExceeded,
    /// The underlying writer failed.
    Io(std::io::ErrorKind),
}

/// A sink that counts and hashes every byte, refuses to pass a cap and writes through to `W`.
pub struct BoundedSink<W: Write> {
    inner: W,
    written: u64,
    cap: u64,
    hasher: Sha256,
    hash_failed: bool,
}

impl<W: Write> BoundedSink<W> {
    /// A sink that accepts at most `cap` bytes.
    pub fn new(inner: W, cap: u64) -> Self {
        Self {
            inner,
            written: 0,
            cap,
            hasher: Sha256::new(),
            hash_failed: false,
        }
    }

    /// Writes `text`. The running size is checked with checked arithmetic before the write.
    pub fn put(&mut self, text: &str) -> Result<(), SinkError> {
        let bytes = text.as_bytes();
        let next = self
            .written
            .checked_add(bytes.len() as u64)
            .filter(|n| *n <= self.cap)
            .ok_or(SinkError::CapExceeded)?;
        self.inner
            .write_all(bytes)
            .map_err(|e| SinkError::Io(e.kind()))?;
        if self.hasher.update(bytes).is_err() {
            self.hash_failed = true;
        }
        self.written = next;
        Ok(())
    }

    /// Bytes written so far.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Flushes the writer and returns it with the byte count and the SHA-256 of the bytes.
    pub fn finish(mut self) -> Result<(W, u64, ContentDigest), SinkError> {
        self.inner.flush().map_err(|e| SinkError::Io(e.kind()))?;
        if self.hash_failed {
            return Err(SinkError::CapExceeded);
        }
        Ok((self.inner, self.written, self.hasher.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reals_follow_the_notation_of_the_contract() {
        let cases: [(f64, &str); 14] = [
            (0.0, "0."),
            (-0.0, "0."),
            (1.0, "1."),
            (1000.0, "1000."),
            (0.5, "0.5"),
            (0.000001, "0.000001"),
            (-12.25, "-12.25"),
            (123456.789, "123456.789"),
            (1e14, "100000000000000."),
            (1e15, "1.E15"),
            (1.25e-9, "1.25E-9"),
            (1e-7, "1.E-7"),
            (-3.5e20, "-3.5E20"),
            (0.1 + 0.2, "0.30000000000000004"),
        ];
        for (value, expected) in cases {
            assert_eq!(real64(value).as_deref(), Some(expected), "{value}");
            let parsed: f64 = expected.parse().unwrap();
            assert_eq!(parsed, value + 0.0);
        }
        assert_eq!(real64(f64::NAN), None);
        assert_eq!(real64(f64::INFINITY), None);
        assert_eq!(real64(f64::NEG_INFINITY), None);
    }

    #[test]
    fn binary32_reals_are_the_shortest_text_of_their_own_type() {
        // 0.1f32 widened to f64 is 0.10000000149011612; the file must say 0.1.
        assert_eq!(real32(0.1).as_deref(), Some("0.1"));
        assert_eq!(real32(100.0).as_deref(), Some("100."));
        assert_eq!(real32(-0.0).as_deref(), Some("0."));
        assert_eq!(real32(16_777_216.0).as_deref(), Some("16777216."));
        assert_eq!(real32(f32::NAN), None);
        for value in [0.1_f32, 1234.5678, 3.4e30, 1.2e-10, 199.99] {
            let text = real32(value).unwrap();
            assert_eq!(text.parse::<f32>().unwrap(), value, "{text}");
        }
    }

    #[test]
    fn strings_are_escaped_and_ascii() {
        assert_eq!(string("plain"), "'plain'");
        assert_eq!(string("it's a\\b"), "'it''s a\\\\b'");
        assert_eq!(string("Балка"), "'\\X2\\04110430043B043A0430\\X0\\'");
        assert_eq!(string("a\u{e9}b"), "'a\\X2\\00E9\\X0\\b'");
        assert_eq!(string("\u{1F600}"), "'\\X4\\0001F600\\X0\\'");
        assert_eq!(string("\u{e9}\u{1F600}"), "'\\X4\\000000E90001F600\\X0\\'");
        assert!(string("tab\there").is_ascii());
        assert_eq!(string(""), "''");
    }

    #[test]
    fn the_time_stamp_is_a_pure_calendar_function() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00");
        assert_eq!(utc_timestamp(1_000_000_000_000), "2001-09-09T01:46:40");
        assert_eq!(utc_timestamp(951_782_399_999), "2000-02-28T23:59:59");
        assert_eq!(utc_timestamp(951_782_400_000), "2000-02-29T00:00:00");
        assert_eq!(utc_timestamp(4_102_444_800_000), "2100-01-01T00:00:00");
    }

    #[test]
    fn the_sink_refuses_to_pass_its_cap_and_hashes_what_it_wrote() {
        let mut sink = BoundedSink::new(Vec::new(), 5);
        sink.put("abc").unwrap();
        assert!(matches!(sink.put("def"), Err(SinkError::CapExceeded)));
        sink.put("de").unwrap();
        let (bytes, written, digest) = sink.finish().unwrap();
        assert_eq!(bytes, b"abcde");
        assert_eq!(written, 5);
        assert_eq!(digest, Sha256::digest(b"abcde").unwrap());
    }
}
