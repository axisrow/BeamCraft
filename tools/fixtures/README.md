# beamcraft-fixtures

A development tool that generates, verifies and checks BeamCraft's original F07 synthetic test
fixtures. It is not published, has no third-party dependencies and contains no geometry kernel.

```sh
cargo run -p beamcraft-fixtures -- generate --out tests/fixtures/f07
cargo run -p beamcraft-fixtures -- verify --dir tests/fixtures/f07
cargo run -p beamcraft-fixtures -- check <FILE>
cargo run -p beamcraft-fixtures -- list
cargo test -p beamcraft-fixtures --test expectations  # independent analytical check
```

The generator computes no expected values. The analytical expectations are checked by
`tests/expectations.rs`, which recomputes volume, axis length, mass and bounds from the fixture
inputs with its own closed-form vector algebra (no production math helpers) and fails on a
corrupted expectation (mutation cases included).

| Module | Purpose |
|--------|---------|
| `src/catalog.rs` | Original cases. Builds input bytes only, never expected values |
| `src/fixture.rs` | `beamcraft.fixture/1` validator, limits and stable error codes |
| `src/json.rs` | Bounded strict JSON reader and deterministic writer |
| `src/sha256.rs` | SHA-256 (FIPS 180-4) |
| `src/lib.rs` | `generate`, `write` and `verify` APIs and the manifest |
| `src/main.rs` | CLI |
| `tests/cli.rs` | End-to-end tests of the binary, including every committed outcome |
| `tests/expectations.rs` | Independent analytical check and mutation cases |

See [docs/fixtures/README.md](../../docs/fixtures/README.md) for the format, statuses,
provenance and verification results. See
[docs/fixtures/expectations.md](../../docs/fixtures/expectations.md) for the analytical
derivations.
