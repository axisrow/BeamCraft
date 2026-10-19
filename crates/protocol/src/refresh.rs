// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 BeamCraft contributors

//! Opt-in CLI refresh envelope, independent of the worker protocol. `BCREFR01` is followed
//! by two little-endian u64 lengths, the unchanged engine bundle and bounded definitions JSON.
//! Payloads are borrowed on decode; their respective consumers still validate all contents.

/// Fixed envelope header size.
pub const HEADER_BYTES: usize = 24;
/// Definitions retain the existing CLI JSON output cap.
pub const MAX_METADATA_BYTES: usize = 1024 * 1024;

/// Maximum pipe output, including the envelope, or `None` on overflow.
pub fn max_bytes(snapshot_cap: usize) -> Option<usize> {
    snapshot_cap
        .checked_add(MAX_METADATA_BYTES)?
        .checked_add(HEADER_BYTES)
}

/// Encode a refresh, enforcing both payload caps before allocation.
pub fn encode(
    snapshot: &[u8],
    metadata: &[u8],
    snapshot_cap: usize,
) -> Result<Vec<u8>, &'static str> {
    let cap = max_bytes(snapshot_cap).ok_or("refresh size overflow")?;
    if snapshot.is_empty()
        || snapshot.len() > snapshot_cap
        || metadata.is_empty()
        || metadata.len() > MAX_METADATA_BYTES
    {
        return Err("refresh payload size");
    }
    let size = snapshot
        .len()
        .checked_add(metadata.len())
        .and_then(|n| n.checked_add(HEADER_BYTES))
        .filter(|n| *n <= cap)
        .ok_or("refresh size overflow")?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| "refresh allocation")?;
    bytes.extend_from_slice(b"BCREFR01");
    bytes.extend_from_slice(&(snapshot.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(metadata.len() as u64).to_le_bytes());
    bytes.extend_from_slice(snapshot);
    bytes.extend_from_slice(metadata);
    Ok(bytes)
}

/// Decode an exact envelope; reject overflows, oversized sections, truncation and trailing bytes.
pub fn decode(bytes: &[u8], snapshot_cap: usize) -> Result<(&[u8], &[u8]), &'static str> {
    let cap = max_bytes(snapshot_cap).ok_or("refresh size overflow")?;
    if bytes.len() > cap || bytes.get(..8) != Some(b"BCREFR01".as_slice()) {
        return Err("refresh header");
    }
    let length = |start: usize, end: usize| {
        let array = bytes
            .get(start..end)
            .and_then(|b| <[u8; 8]>::try_from(b).ok())
            .ok_or("refresh header")?;
        usize::try_from(u64::from_le_bytes(array)).map_err(|_| "refresh size overflow")
    };
    let snapshot_len = length(8, 16)?;
    let metadata_len = length(16, 24)?;
    if snapshot_len == 0
        || snapshot_len > snapshot_cap
        || metadata_len == 0
        || metadata_len > MAX_METADATA_BYTES
    {
        return Err("refresh payload size");
    }
    let middle = HEADER_BYTES
        .checked_add(snapshot_len)
        .ok_or("refresh size overflow")?;
    let end = middle
        .checked_add(metadata_len)
        .ok_or("refresh size overflow")?;
    if end != bytes.len() {
        return Err("refresh length");
    }
    Ok((
        bytes.get(HEADER_BYTES..middle).ok_or("refresh length")?,
        bytes.get(middle..end).ok_or("refresh length")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_borrowed_sections_and_malformed_envelopes() {
        let bytes = encode(b"snapshot", b"{}", 8).unwrap();
        assert_eq!(
            decode(&bytes, 8).unwrap(),
            (b"snapshot".as_slice(), b"{}".as_slice())
        );
        for n in 0..bytes.len() {
            assert!(decode(&bytes[..n], 8).is_err());
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(decode(&extra, 8).is_err());
        let mut overflow = bytes.clone();
        overflow[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode(&overflow, usize::MAX).is_err());
        overflow[8..16].copy_from_slice(&8u64.to_le_bytes());
        overflow[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode(&overflow, 8).is_err());
        assert!(encode(b"snapshot", b"{}", 7).is_err());
        assert!(decode(&bytes, 7).is_err());
        assert!(encode(b"", b"{}", 8).is_err());
        assert!(encode(b"snapshot", &vec![0; MAX_METADATA_BYTES + 1], 8).is_err());
    }
}
