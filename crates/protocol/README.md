# BeamCraft protocol

The original F05 `frame`, `limits` and `code` APIs remain unchanged. U04 adds `json` and
`message` for an experimental read-only Rust-worker profile. This crate still performs no
I/O, process launch or filesystem access. It is desktop-native, per the WS amendment.

`refresh` is a separate opt-in CLI export envelope, not a worker-protocol frame: ASCII
`BCREFR01`, little-endian u64 snapshot length, little-endian u64 metadata length, exact engine
bundle bytes, then definitions JSON. There are no trailing bytes. The caller supplies the
engine bundle cap; metadata is capped at 1 MiB. Lengths and checked arithmetic are validated
before payload access or allocation. Decode borrows both sections; engine and JSON consumers
still perform their own full validation. This adds no engine-to-protocol dependency.

`json::decode` scans complete capped UTF-8 before constructing a bounded tree. It checks
grammar, depth, decoded strings, tokens, a conservative scratch reservation, numeric
overflow/nonzero underflow, escapes and surrogate pairs; tree construction rejects duplicate
decoded keys. `Json::encode` revalidates output. This is not an RFC 8785/JCS canonicalizer.

`message` defines full pinned identities/contexts, virtual-name and value checks, exact
read method bodies and an experimental profile commitment. No engineering mutation,
authored-layer schema, dynamic plugin method or acceptance result is registered.
Generation and typed integer attributes remain lossless decimal strings. Profile metadata
and static values are revalidated at the native host boundary.

The read profile negotiates zero attachments. U05 digest-addressed files carry asset bytes;
control frames carry only grants and bounded pages. Existing binary-prefix/bundle tests
remain relevant, but U04 does not claim an attachment-stream implementation.
The three advertised features are `private_pipes`, `sha256_buffers` and `cancel`;
full F05's outcome/event feature admission and U02 schema agreement remain open. The
Rust runtime mapping, profile commitment and narrower capability negotiation require review;
F05 itself is unchanged. See [U04 review](../../docs/design/u04-cli-host.md).
