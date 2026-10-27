<!--
Thanks for contributing to BeamCraft. Base this on `master`, the only integration branch.
Delete any section that does not apply.
-->

## What this changes

<!-- What behaviour is different afterwards, and why. If it fixes an issue, say `Fixes #123`. -->

<!-- Link the task issue and stable task ID. State which acceptance this PR covers;
completing a slice does not accept its parent gate. Record delivery evidence in the issue. -->

## How you verified it

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`

New behaviour needs a test. For a bug fix, check that the test fails before your fix.

## Never-crash

Every model file, IFC/DSTV import and keystroke is untrusted input (`AGENTS.md` §4).

- [ ] No new `unwrap`/`expect`/`panic!`/`todo!` outside tests
- [ ] No indexing or slicing with input-derived positions; allocations sized by input are capped
- [ ] No `unsafe`

## Provenance

Read `AGENTS.md` §1 and §2 before adding any asset.

- [ ] Nothing taken from Tekla Structures or any Trimble product (assets, code, documentation, tables)
- [ ] No GPL/AGPL code copied in
- [ ] Every new asset has an entry in `ATTRIBUTION.md`

## Anything a reviewer should look at first

<!-- The judgement call you are least sure about. -->
