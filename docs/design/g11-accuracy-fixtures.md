# G11: accuracy fixtures

Date: 2026-11-01. Status: delivered with issue #490. Depends on G06 (measurement) and F07
(fixture tooling).

## What it is

G11 is a set of 50 original fixtures, [`docs/fixtures/g11`](../fixtures/g11/README.md), in four
classes:

- rounded-mesh cross-checks;
- subnormal mass;
- very large coordinates;
- cancellation-heavy inputs.

Each fixture carries references computed by exact rational arithmetic from its binary64 inputs.
A harness in the steel test binary holds the real builders to the F06 comparison rule
(CR-TOL-001, F04 section 10) on every fixture.

The covered bodies are:

- plates: rectangles and contours;
- members: every parametric section family, sharp and with radii, and rectangles;
- the four components of a bolt;
- rigid placement up to the coordinate cap.

## How the references are made

`beamcraft-fixtures g11` (`tools/fixtures/src/g11.rs`) holds the inputs. It derives the
references with `tools/fixtures/src/rational.rs`, an arbitrary-size rational type with no
dependency:

- every binary64 value converts exactly;
- sums, products and quotients stay exact;
- π is a 64-digit decimal;
- square roots are bounded below within 2^-200.

The generator calls no kernel or engine code. `g11 verify` rebuilds the files byte for byte, and
`tools/fixtures/tests/g11.rs` checks it on every test run, together with the decimal roundings and
hand-derived values. The project retired Python from its tooling (#178), so the generator is Rust
and stays runnable, unlike the one-time G03 script.

## What it found

- **A subnormal mass was reported as current.** `analytic_mass_kg` refused only a zero product.
  A product between zero and `2^-1022` kept a relative error up to about 1e-5. Positive-density
  masses below the normal range are now `GEO_NUMERIC_UNCERTAIN`. The fix comes with a regression
  case in `crates/kernel/tests/g06_measure.rs` and the corrected sentence in the F04 density
  amendment.
- **No other defect.** Quantities are exact to the last bits on the `mesh`, `far` and `subnormal`
  classes. The worst cancellation case, a thin-wall RHS or CHS of 1e6 mm, keeps about 1e-10. A
  mesh near 1e9 mm deviates from its solid only by vertex rounding (1.4e-7 on a 0.5 mm section),
  which the bound covers. The pinned margins are in the harness and the fixture README.
- **An inconsistency between two admission scales.** It is recorded, not changed. A wall thinner
  than the outline's own floor passes the member's minimum-dimension check and is then refused by
  the G05 outline admission. The fix belongs to the contract (F06 section 4), not to G11.

## Owner decisions

- The F06 section 4 constants are unchanged: `tolerance_policy` stays 3. The measured margins
  would allow tightening, which is a separate contract change.
- Defects found are fixed in the same change.

## Not covered

- WASM and cross-target conformance.
- Holed, cut, bent, swept or lofted bodies.
- Arcs beyond their chord rule.
