# CI duration baseline for #239

Contributor-original analysis and GitHub Actions timing exports from Apibim-com/BeamCraft.
Collected 2026-10-26. This records the measurement prerequisite; it does not close #239.

## Observed results

The last 30 successful CI runs span **2026-10-25 09:37:14 UTC to 2026-10-26 06:09:56 UTC**:
20 pull-request runs and 10 pushes, all attempt 1, with the same 34-job matrix.

| Population | Runs | Median run wall | p95 run wall |
| --- | ---: | ---: | ---: |
| Combined | 30 | 245.5 s | 510 s |
| Pull request | 20 | 248.5 s | 510 s |
| Push | 10 | 244 s | 307 s |
| #239 target | — | ≤180 s | ≤300 s |

Top three individual jobs by median execution wall (30 samples each):

| Job | Median | p95 |
| --- | ---: | ---: |
| build test archive (windows-latest) | 125 s | 165 s |
| cold release (x86_64-unknown-linux-gnu) | 118 s | 128 s |
| cold release (aarch64-apple-darwin) | 113.5 s | 157 s |

**Queue share is 19.92%** of summed job waiting plus execution time. It is a workload-weighted
share across parallel jobs, not a fraction of the run's critical path. The median of the
per-run median macOS shard queue is 20.75 s. The job that finishes the run has a median
68 s queue. These waits materially affect the tail and must not be counted as test work.

## Critical path and next experiment

Every run finishes on a test shard: 21 macOS, 9 Windows. Cold release, licences and quality
are **never the final job** in this sample. In the measured workflow, cold release is already
parallel and quality already uses rust-cache; the original suspects in #239 are not sufficient
evidence for removing/rearranging those jobs.

The Windows archive's median 125 s precedes shards, of which shard 6 has a median 99.5 s
wall. macOS combines archive completion with substantial shard queue waits. Both paths
must be improved to reach three minutes; improving only the currently last job would leave
another platform on the critical path. Subtracting the last job's queue from each run
still leaves a 214.5 s median; that is a diagnostic lower estimate, not a predicted result
of a scheduling change, since another job may then become last.

The next change should instrument archive extraction separately from test execution and
retain per-test durations for a controlled shard-count/balance experiment on Windows and
macOS. Compare median/p95 run wall, archive time, shard execution and queue time with this
baseline. Keep all tests, doctests and cold release coverage. Do not claim the ≤3-minute
goal from a job-level speedup or a single successful run.

No CI behavior is changed by this report. The sample covers less than one day and therefore
cannot establish the required week of sustained results after an optimization lands.

## Sources and reproduction

- [runs.csv](runs.csv): run identity, event, head SHA and timestamps, with derived durations.
- [jobs.csv](jobs.csv): all 1,020 jobs, including IDs, timestamps, waiting and execution wall.
- Run pages are `https://github.com/Apibim-com/BeamCraft/actions/runs/<run_id>`;
  individual job pages append `/job/<job_id>`.
- Source collection: `GET /repos/Apibim-com/BeamCraft/actions/runs?status=success&per_page=100`,
  filter the latest 30 `CI` runs, then `GET /repos/Apibim-com/BeamCraft/actions/runs/<id>/jobs?per_page=100`.
  All 34 jobs fit each first page; no page was silently omitted.
- Run wall is run `created_at` to latest job `completed_at`, including initial waiting and
  dependency chains. Job execution wall is `started_at` to `completed_at`; waiting is
  `created_at` to `started_at`. p95 uses nearest rank, `ceil(0.95*n)`, without interpolation.
- Successful runs only, as requested by #239; cancelled/failed runs are outside this sample.

The two CSV exports carry every input the numbers derive from, and the repository carries no
analysis script: recompute with any local tooling from the timestamps alone. Run wall joins each
run to its latest job `completed_at`; job execution wall and waiting are the `started_at` and
`created_at` differences listed above; medians and the nearest-rank p95 need only those per-run
values. The recomputation reads only these CSV exports and requires no credentials or network
access.
