# Original synthetic fixtures (F07)

This directory documents BeamCraft's first set of original test fixtures. The fixtures are small
JSON descriptions of rectangular plates and straight rectangular members. The set also includes
negative inputs, a provenance manifest with full SHA-256 digests, and analytical expectations
that are kept separate from the generator.

Contributor-original, 2026-10-14, `MPL-2.0`. No asset, dimension, value or name in this set
comes from Tekla Structures, Trimble or any other commercial product, from vendor catalogues or
national material tables, or from observation notes such as F02.

| Path | Content | Produced by |
|------|---------|-------------|
| [`tools/fixtures/`](../../tools/fixtures/) | `beamcraft-fixtures` crate: generator API, CLI, validator and the independent analytical test | contributors |
| [`tests/fixtures/f07/`](../../tests/fixtures/f07/) | 33 fixtures (`valid/`, `invalid/`) and `manifest.json` | the generator only |
| [`tests/fixtures/expectations/f07.json`](../../tests/fixtures/expectations/f07.json) | Expected outcomes and analytical values | hand-written |
| [`expectations.md`](expectations.md) | Derivations, units, conversions and tolerances | hand-written |

## What these fixtures are and are not

They are **test inputs** for future engine work: G02 plates, P02–P08 members and copies, and
U02 full assets. `beamcraft.fixture/1` is a test-description format, not the BeamCraft model or
command contract. F04 is frozen at 0.1.0 (2026-10-16). Future tasks will map these
descriptions onto that contract with their own adapters.

They do not show that BeamCraft creates, meshes, copies or saves anything. Nor do they show that
any kernel or USD feature gate passes. They contain no USD asset and no mesh.

## Format `beamcraft.fixture/1`

```json
{
  "format": "beamcraft.fixture/1",
  "id": "f07.member.along-x",
  "description": "optional free text",
  "units": {"length": "mm", "angle": "deg", "density": "kg/m3"},
  "objects": [
    {"key": "p", "kind": "plate", "size": [a, b, t],
     "placement": {"origin": [x, y, z], "axis": [x, y, z], "angle": deg},
     "density": 6000},
    {"key": "m", "kind": "member", "start": [x, y, z], "end": [x, y, z], "up": [x, y, z],
     "profile": {"shape": "rect", "width": w, "height": h}}
  ]
}
```

- The file must be strict JSON (RFC 8259) in UTF-8. These are rejected: duplicate member names,
  unknown fields, `NaN`/`Infinity`, and numbers that overflow f64.
- `id` and `key` use 1–64 characters from `[a-z0-9.-]`. Object keys are unique within a fixture.
- The units must be exactly mm, deg and kg/m3; this format performs no unit conversion. Engine
  internals use radians, so θ_rad = θ_deg · π / 180.
- **Plate**: the local box [0, a] × [0, b] × [0, t] is rotated about the unit vector of `axis`
  through the local origin by `angle` (right-hand rule), then translated by `origin`.
  An `axis` need not be unit length.
- **Member**: a prism along start → end, with a w × h rectangle centred on the axis. Height runs
  along `up` made orthogonal to the axis, and width runs along height × axis. The member has no
  end extensions. `up` need not be unit length or exactly perpendicular, but it must not be
  parallel to the axis.
- `density` is optional. An absent density means unknown density and unknown mass.

### Limits

| Limit | Value |
|-------|-------|
| File size | 65 536 bytes |
| JSON nesting depth | 16 |
| JSON values per file | 4096 |
| String length | 256 bytes |
| Number literal length | 32 characters |
| Members per JSON object | 32 |
| Objects per fixture | 1 to 16 |
| Coordinate or direction component | ±10 000 000 mm (10 km) |
| Plate size, profile width or height | 0.001 mm to 100 000 mm |
| Member axis length | at least 0.001 mm |
| Rotation angle | ±360° |
| Density | greater than 0, at most 1 000 000 kg/m3 |

### Validation order and error codes

The validator reports the first violation, in this order:

1. JSON syntax and parser limits.
2. The root is an object.
3. `format`.
4. Unknown top-level fields.
5. `id`, then `description`, then `units`.
6. The object count.
7. Each object, in document order. Each field is fully checked (presence, type, limit, then sign,
   minimum size or degeneracy) before the next field is read:
   - **Plate**:
     1. `kind` and unknown fields.
     2. Each `size` element in turn: limit, then sign, then minimum.
     3. `placement`: its fields, the `origin` and `axis` limits, the `angle` limit, then the zero
        axis.
     4. `density`.
   - **Member**:
     1. `kind` and unknown fields.
     2. The `start`, `end` and `up` limits.
     3. The profile `shape`, then the profile fields.
     4. `width`, then `height`.
     5. Axis length, then zero or parallel `up`.
     6. `density`.
   - Finally, the object's `key`: its format, then whether it duplicates an earlier key.

| Code | Meaning |
|------|---------|
| `malformed_json` | Invalid syntax or UTF-8, trailing data, or a duplicate member name |
| `non_finite_number` | `NaN`/`Infinity`, or a number that overflows f64 |
| `limit_exceeded` | Any limit in the table above |
| `unsupported_format` | `format` is not `beamcraft.fixture/1` |
| `unsupported_unit` | A unit other than mm, deg or kg/m3 |
| `unsupported_profile` | A profile shape other than `rect` |
| `invalid_field` | A missing, unknown or wrongly typed field, a malformed identifier, or an empty object list |
| `duplicate_object_key` | Two objects share a key |
| `non_positive_dimension` | A size, width or height ≤ 0 |
| `non_positive_density` | A density ≤ 0 |
| `degenerate_geometry` | A dimension or axis below 0.001 mm, a zero rotation axis, or `up` that is zero or parallel to the axis |

## Support status

Every fixture has exactly one status, which appears in both the manifest and the expectations.

| Status | Count | Meaning |
|--------|-------|---------|
| `supported` | 9 | Accepted. Analytical volume, bounds, mass and member length must hold within the stated tolerances. |
| `unverified` | 1 | Accepted with exact analytical values, but no implementation has yet shown that it meets the tolerances at this scale (10⁷ mm coordinates). |
| `unsupported` | 2 | Outside version 1 (circular profile, inch units). It must be rejected with a diagnostic and never approximated. |
| `invalid` | 21 | Malformed, out-of-limit or degenerate input. It must be rejected with the listed code. |

Cases cover plates at the origin, translated, given quarter-turns about Z and X, and given an
oblique 30° turn. Members cover an X-axis member, a vertical member and an oblique 5-12-13
member. The set also includes a two-object independent-copy input and a plate with unknown
density. See [expectations.md](expectations.md) for every case.

## Manifest

`tests/fixtures/f07/manifest.json` (`beamcraft.fixture-manifest/1`) records:

- the fixture set and its version;
- the generator name, version, source and command;
- the units;
- set-wide provenance and the expected-results provenance, including the expectations file,
  the derivation document and the independent analytical test.

Each fixture entry records:

- a stable `id`;
- `path`, relative to the manifest;
- exact `bytes` and a full lowercase `sha256` of the committed bytes;
- `support`, `expected_outcome` and, for rejections, `expected_error`;
- an `expectation` reference;
- `author`, `origin` and `licence` (`MPL-2.0`);
- a summary.

IDs are stable. If the meaning of a case changes, it gets a new ID. Any added, removed or changed
case increases `FIXTURE_SET_VERSION` in `tools/fixtures/src/lib.rs`, and the same change must be
made in the expectations file.

The repository has `* -text` in `.gitattributes`, so checkouts are byte-identical on every
platform and the digests hold everywhere.

## Commands

From the repository root, on PowerShell or a POSIX shell:

```sh
# Regenerate the committed set. The command refuses directories that hold foreign files.
cargo run -p beamcraft-fixtures -- generate --out tests/fixtures/f07
# Compare the directory byte for byte with the generator and recheck every manifest digest.
cargo run -p beamcraft-fixtures -- verify --dir tests/fixtures/f07
# Validate one description. Exit status: 0 accepted, 3 rejected, 1 I/O error, 2 usage error.
cargo run -p beamcraft-fixtures -- check tests/fixtures/f07/valid/member-vertical.json
cargo run -p beamcraft-fixtures -- list
# Independent analytical check (closed forms recomputed from the fixture inputs).
cargo test -p beamcraft-fixtures --test expectations
```

The library API (`beamcraft_fixtures::generate`, `write`, `verify` and `fixture::check`) is what
the CLI calls. CI runs the Rust unit and E2E tests with `cargo test --workspace` (the
independent analytical check included), then runs the `verify` command on Windows and Linux.

## Independence of the expected results

- The generator creates input bytes only. It contains no volume, length, mass or bounds formula.
- The expectations are hand-derived closed forms. The rejection codes follow the format rules
  above.
- `tests/expectations.rs` recomputes every analytical value from the fixture inputs with its own
  Rodrigues' rotation and frame algebra (no production math helpers) and compares the results
  with the hand-written values, so the comparison legs are independent by construction. Its
  mutation cases show that the check fails on a wrong volume and on a mass invented for an
  unknown density; `cli.rs` shows that the outcomes fail on a tampered byte, an unlisted file, a
  wrong oblique rejection code and a missing expectation.
- The SHA-256 implementation follows the public FIPS 180-4 standard and is tested against
  published example digests.

## Provenance

All inputs were chosen for this set. Densities are a round synthetic value. Mathematical sources
are public: elementary vector algebra, Rodrigues' rotation formula, RFC 8259 (JSON), RFC 3629
(UTF-8) and FIPS 180-4 (SHA-256).

## Verification results (2026-10-14, Windows x64, Rust 1.96.0, Python 3.14.3)

| Command | Result |
|---------|--------|
| `cargo fmt --all -- --check` | passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | passed |
| `cargo test --workspace` | passed: 21 unit and 8 CLI E2E fixture tests, plus 3 existing tests |
| `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS=-D warnings` | passed |
| `cargo run -p beamcraft-fixtures -- verify --dir tests/fixtures/f07` | passed: 34 files, 33 manifest digests |
| `python tools/fixtures/verify_expectations.py` (retired 2026-10-22; replaced by `tests/expectations.rs`) | passed: 33 files, 33 SHA-256 digests, 33 cases, 172 analytical quantities |
| `python tools/fixtures/test_verify_expectations.py` (retired 2026-10-22; replaced by the mutation cases in `tests/expectations.rs`) | passed: 8 mutation tests |
| PowerShell `Get-FileHash -Algorithm SHA256` over every fixture | matched all 33 manifest digests |
| Linux CI, WASM check, `cargo deny` | not run locally; left to CI |

## Limitations

- These are parametric descriptions only. They include no meshes, USD layers or immutable
  assets; U02 and G02 own those.
- The set has no holes, cuts, cavities or non-rectangular profiles. G03 extends the set with
  its own cases.
- The copy case is an input description, not evidence that copying works.
- The validator checks this test format. It does not replace future engine validation, which
  must define its own contract (F04/F06).
