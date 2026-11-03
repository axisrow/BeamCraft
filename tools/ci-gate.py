#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
# SPDX-FileCopyrightText: 2026 Apibim
"""The checks behind CI's single required job, `ci-ok`.

    ci-gate.py tests --list LIST.json --ignored IGNORED.txt [--label NAME] JUNIT.xml...
        LIST.json is `cargo nextest list --message-format json` for the whole workspace on one
        platform; the JUnit files are that platform's shard reports. Passes only when the shards
        together ran every listed test exactly once, nothing was left out by a filter other
        than `#[ignore]`, and the ignored tests are exactly the ones named in IGNORED.txt.

    ci-gate.py jobs --event EVENT [--prose true|false] --needs NEEDS.json --workflow ci.yml
        NEEDS.json is `toJSON(needs)` of the `ci-ok` job. Passes only when every job succeeded,
        except the jobs this event is allowed to skip, and when `ci-ok` waits for every other
        job of the workflow, so a new job cannot bypass the gate. `--prose true` (a pull request
        that changes only prose, tools/ci-changes.py) allows skipping the Rust builds and tests
        and requires `docs-checks` instead.

Standard library only, so it runs on any runner's Python 3 without installing anything.
"""

import argparse
import collections
import json
import os
import re
import sys
import xml.etree.ElementTree as ET

# Jobs a pull request skips by design (owner decision on #180): Windows runs on master and on
# manual runs only. Every other job must succeed, and nothing may be skipped on other events.
ALLOWED_SKIPS = {
    "pull_request": {"test-archive-windows", "test-windows"},
}
# The Rust builds and tests a prose-only pull request skips (tools/ci-changes.py), and the job
# that runs only for such a pull request. Prose is honoured on pull requests only.
PROSE_SKIPS = {
    "quality",
    "test-archive-ubuntu",
    "test-ubuntu",
    "test-archive-macos",
    "test-macos",
    "build-timings",
    "wasm",
}
PROSE_ONLY = {"docs-checks"}


def allowed_skips(event, prose):
    allowed = set(ALLOWED_SKIPS.get(event, set()))
    if event == "pull_request" and prose:
        allowed |= PROSE_SKIPS
    else:
        allowed |= PROSE_ONLY
    return allowed


def fail(message):
    print(f"error: {message}", file=sys.stderr)
    sys.exit(2)


def summary(lines):
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if path:
        with open(path, "a", encoding="utf-8") as out:
            out.write("\n".join(lines) + "\n")


def read_list(path):
    """Return (runnable, ignored, filtered_out) sets of (binary-id, test) pairs."""
    try:
        with open(path, encoding="utf-8") as handle:
            data = json.load(handle)
    except (OSError, ValueError) as error:
        fail(f"cannot read the test list {path}: {error}")
    suites = data.get("rust-suites")
    if not isinstance(suites, dict) or not suites:
        fail(f"{path} has no rust-suites; is it `cargo nextest list --message-format json`?")
    runnable, ignored, filtered = set(), set(), set()
    for binary_id, suite in suites.items():
        cases = suite.get("testcases")
        if not isinstance(cases, dict):
            fail(f"{path}: suite {binary_id} has no testcases object")
        for name, case in cases.items():
            key = (binary_id, name)
            match = case.get("filter-match", {})
            if case.get("ignored") is True:
                ignored.add(key)
            elif match.get("status") == "matches":
                runnable.add(key)
            else:
                filtered.add((binary_id, name, str(match.get("reason", match))))
    return runnable, ignored, filtered


def read_ignored(path):
    entries = set()
    try:
        with open(path, encoding="utf-8") as handle:
            for number, raw in enumerate(handle, 1):
                line = raw.strip()
                if not line or line.startswith("#"):
                    continue
                parts = line.split()
                if len(parts) != 2:
                    fail(f"{path}:{number}: expected `<binary-id> <test-name>`, got {line!r}")
                entries.add((parts[0], parts[1]))
    except OSError as error:
        fail(f"cannot read {path}: {error}")
    return entries


def read_junit(paths):
    """Count every executed (binary-id, test) and collect retry markers."""
    executed = collections.Counter()
    retried = []
    for path in paths:
        try:
            root = ET.parse(path).getroot()
        except (OSError, ET.ParseError) as error:
            fail(f"cannot read the JUnit report {path}: {error}")
        for suite in root.iter("testsuite"):
            for case in suite.iter("testcase"):
                binary_id = case.get("classname") or suite.get("name")
                name = case.get("name")
                if not binary_id or not name:
                    fail(f"{path}: a testcase without a binary id or name")
                children = {child.tag for child in case}
                if "skipped" in children:
                    continue
                if children & {"flakyFailure", "flakyError", "rerunFailure", "rerunError"}:
                    retried.append((binary_id, name))
                executed[(binary_id, name)] += 1
    return executed, retried


def show(title, items, limit=40):
    items = sorted(items)
    print(f"{title}: {len(items)}")
    for item in items[:limit]:
        print("  " + " ".join(str(part) for part in item))
    if len(items) > limit:
        print(f"  ... and {len(items) - limit} more")


def check_tests(args):
    runnable, ignored, filtered = read_list(args.list)
    expected_ignored = read_ignored(args.ignored)
    executed, retried = read_junit(args.junit)
    ran = set(executed)
    missing = runnable - ran
    unexpected = ran - runnable
    duplicated = {key + (count,) for key, count in executed.items() if count > 1}
    newly_ignored = ignored - expected_ignored
    stale_ignored = expected_ignored - ignored
    problems = [
        ("tests listed but not run by any shard", missing),
        ("tests run more than once (binary, test, times)", duplicated),
        ("tests run but absent from the list", unexpected),
        ("tests left out by a filter other than #[ignore] (binary, test, reason)", filtered),
        ("tests retried; CI runs with retries = 0", retried),
        ("tests newly #[ignore]d; name them in the ignored file deliberately", newly_ignored),
        ("entries of the ignored file that are not ignored tests any more", stale_ignored),
    ]
    label = args.label or args.list
    total = sum(executed.values())
    print(
        f"{label}: {len(runnable)} listed, {total} run in {len(args.junit)} reports, "
        f"{len(ignored)} ignored"
    )
    failed = False
    for title, items in problems:
        if items:
            failed = True
            show(title, items)
    verdict = "FAIL" if failed else "ok"
    summary([f"- {label}: {verdict} — {len(runnable)} listed, {total} run, {len(ignored)} ignored"])
    return 1 if failed else 0


def workflow_jobs(path):
    """Top-level job ids of a workflow, read from its two-space-indented `jobs:` block."""
    try:
        with open(path, encoding="utf-8") as handle:
            text = handle.read()
    except OSError as error:
        fail(f"cannot read {path}: {error}")
    block = re.split(r"(?m)^jobs:\s*$", text, maxsplit=1)
    if len(block) != 2:
        fail(f"{path} has no top-level `jobs:` block")
    ids = re.findall(r"(?m)^  ([A-Za-z0-9_-]+):\s*$", block[1])
    if not ids:
        fail(f"{path}: no jobs found under `jobs:`")
    return set(ids)


def check_jobs(args):
    try:
        with open(args.needs, encoding="utf-8") as handle:
            needs = json.load(handle)
    except (OSError, ValueError) as error:
        fail(f"cannot read {args.needs}: {error}")
    if not isinstance(needs, dict):
        fail(f"{args.needs} is not the `needs` object")
    allowed = allowed_skips(args.event, args.prose == "true")
    failed = False
    lines = []
    for job, info in sorted(needs.items()):
        result = info.get("result") if isinstance(info, dict) else None
        ok = result == "success" or (result == "skipped" and job in allowed)
        lines.append(f"- `{job}`: {result}{'' if ok else ' (not allowed)'}")
        if not ok:
            failed = True
            print(f"{job}: {result} is not allowed on a {args.event} run")
    others = workflow_jobs(args.workflow) - {args.gate}
    unwatched = others - set(needs)
    if unwatched:
        failed = True
        show(f"jobs of {args.workflow} that `{args.gate}` does not wait for", [(j,) for j in unwatched])
    summary([f"### Jobs ({args.event})"] + lines)
    print("jobs: FAIL" if failed else f"jobs: ok ({len(needs)} jobs)")
    return 1 if failed else 0


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    sub = parser.add_subparsers(dest="command", required=True)
    tests = sub.add_parser("tests", help="every listed test ran exactly once")
    tests.add_argument("--list", required=True)
    tests.add_argument("--ignored", required=True)
    tests.add_argument("--label")
    tests.add_argument("junit", nargs="+")
    jobs = sub.add_parser("jobs", help="every job succeeded and is gated")
    jobs.add_argument("--event", required=True)
    jobs.add_argument("--needs", required=True)
    jobs.add_argument("--workflow", required=True)
    jobs.add_argument("--gate", default="ci-ok")
    jobs.add_argument("--prose", choices=("true", "false"), default="false")
    args = parser.parse_args(argv)
    return check_tests(args) if args.command == "tests" else check_jobs(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
