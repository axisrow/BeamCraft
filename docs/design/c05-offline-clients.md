# C05: two offline clients, retries and ID integrity

Scope: fork #12 / upstream #126, after C04. This specifies the stress harness before its code.
The proposed harness's supported profile, commands and recorded reader checks are in
[`c03-c06-acceptance.md`](c03-c06-acceptance.md).
The production API remains explicit C01/C02 publication and C03 rebase; no new service or
automatic network synchronization is needed.

## Harness and budgets

Create two distinct personal workspaces for the same user from one accepted project. Allocate
1000 element IDs per workspace while disconnected from each other's draft state. Reopen each
workspace and alternate publication/rebase rounds, including a stale-base attempt, a repeated
request, an interrupted commit and a reply lost after a successful commit.

Keep production budgets unchanged. A 1000-part creation interval exceeds the current aggregate
dependency cap because one create writes several groups. Split the 1000+1000 scenario into
bounded offline rounds, while keeping both workspaces and all prior accepted parts throughout.
Record the actual batch/round sizes in the harness. A budget refusal of an oversized unsent
interval is safe and separately tested; do not silently drop dependencies to admit it.

A test-only fixture may group admitted typed creates into a bounded synthetic operation to avoid
thousands of process launches and repeated whole-stage rewrites. It must call the real engine
for each create, union actual footprints, use the closed USD author and strict host commit, and
identify itself as synthetic batch evidence. It must not claim to test a production bulk CLI
that does not exist. Publication, rebase, recovery and replay must use production code.

The oracle receives explicit expected IDs, names, dimensions and placements from fixture inputs,
not a copy of the engine's final output. Materialize the actual final accepted revision for the
independent reader and ordinary OpenUSD. The independent reader is additional evidence; it does
not substitute for the plan's ordinary OpenUSD requirement. SDK provenance follows ATTRIBUTION.

## Executable acceptance

| Case | Required observable result |
|---|---|
| C05-01 1000+1000 | Both clients contribute 1000 distinct new element IDs; all survive explicit publication; no duplicate ID or USD path and no lost earlier round |
| C05-02 real offline basis | Both drafts exist before either round is accepted; stale publication refuses; explicit rebase retains both sides and unsent operation order |
| C05-03 repeat/reuse | Publication/rebase retries create no extra revision; original operation/result identity is returned; changed payload under an old ID refuses |
| C05-04 ambiguous result | Interrupt at a real commit boundary and discard a completed response; recover/retry in a fresh process; exactly one candidate is accepted |
| C05-05 same user, distinct drafts | One workspace's publication metadata does not advance the other workspace's unsent prefix; later local work remains unsent |
| C05-06 budgets | At-cap inputs are handled; cap-plus-one intervals refuse before partial metadata/head changes; no raised production cap |
| C05-07 final readers | Reopen final accepted snapshot with exact count, identities, closed-form geometry/quantities and placements; independent reader and ordinary OpenUSD agree |

Keep timing assertions out of correctness tests. Record fixture size and actual checks in the PR;
performance budgets belong to their dedicated tasks. CI must run the supported headless harness;
an SDK-dependent ordinary OpenUSD check can be an explicit additional acceptance command.
