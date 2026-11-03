#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
# SPDX-FileCopyrightText: 2026 Apibim
"""Tests for tools/ci-gate.py. The fixtures follow the shapes cargo-nextest 0.9.149 writes
(`cargo nextest list --message-format json` and its JUnit report); every case runs the script
as CI does, as a separate process."""

import json
import os
import subprocess
import sys
import tempfile
import unittest

GATE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ci-gate.py")


def nextest_list(suites):
    """suites: {binary_id: {test: "run" | "ignored" | <filter reason>}}"""
    rust_suites = {}
    for binary_id, cases in suites.items():
        testcases = {}
        for name, state in cases.items():
            if state == "run":
                testcases[name] = {"ignored": False, "filter-match": {"status": "matches"}}
            elif state == "ignored":
                testcases[name] = {
                    "ignored": True,
                    "filter-match": {"status": "mismatch", "reason": "ignored"},
                }
            else:
                testcases[name] = {
                    "ignored": False,
                    "filter-match": {"status": "mismatch", "reason": state},
                }
        rust_suites[binary_id] = {"binary-id": binary_id, "testcases": testcases}
    return {"rust-build-meta": {}, "test-count": 0, "rust-suites": rust_suites}


def junit(cases, retried=()):
    """cases: [(binary_id, test)] in run order."""
    suites = {}
    for binary_id, name in cases:
        suites.setdefault(binary_id, []).append(name)
    parts = ['<?xml version="1.0" encoding="UTF-8"?>', '<testsuites name="nextest-run">']
    for binary_id, names in suites.items():
        parts.append(f'<testsuite name="{binary_id}" tests="{len(names)}">')
        for name in names:
            extra = "<flakyFailure/>" if (binary_id, name) in retried else ""
            parts.append(f'<testcase name="{name}" classname="{binary_id}" time="0.1">{extra}</testcase>')
        parts.append("</testsuite>")
    parts.append("</testsuites>")
    return "\n".join(parts)


WORKFLOW = """name: CI
on:
  pull_request:
jobs:
  quality:
    runs-on: ubuntu-latest
  test-windows:
    runs-on: windows-latest
  test-archive-windows:
    runs-on: windows-latest
  ci-ok:
    needs: [quality, test-windows, test-archive-windows]
"""


class GateCase(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)

    def path(self, name, content):
        full = os.path.join(self.dir.name, name)
        with open(full, "w", encoding="utf-8") as handle:
            handle.write(content if isinstance(content, str) else json.dumps(content))
        return full

    def run_gate(self, *args):
        env = dict(os.environ)
        env.pop("GITHUB_STEP_SUMMARY", None)
        return subprocess.run(
            [sys.executable, GATE, *args], capture_output=True, text=True, env=env, check=False
        )

    def run_tests(self, suites, shards, ignored="", retried=()):
        listing = self.path("list.json", nextest_list(suites))
        ignored_file = self.path("ignored.txt", ignored)
        reports = [
            self.path(f"junit-{i}.xml", junit(cases, retried)) for i, cases in enumerate(shards)
        ]
        return self.run_gate("tests", "--list", listing, "--ignored", ignored_file, *reports)


SUITES = {
    "beamcraft-cli::cli": {"a::one": "run", "a::two": "run", "b::three": "run"},
    "beamcraft-host::recovery": {"torn": "run"},
    "beamcraft-ui::edit_tools": {"blocked": "ignored"},
}
IGNORED = "# blocked on #763\nbeamcraft-ui::edit_tools blocked\n"
ALL = [
    ("beamcraft-cli::cli", "a::one"),
    ("beamcraft-cli::cli", "a::two"),
    ("beamcraft-cli::cli", "b::three"),
    ("beamcraft-host::recovery", "torn"),
]


class TestsCheck(GateCase):
    def test_every_test_once_across_shards_passes(self):
        result = self.run_tests(SUITES, [ALL[:2], ALL[2:]], IGNORED)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("4 listed, 4 run in 2 reports, 1 ignored", result.stdout)

    def test_a_missing_test_fails(self):
        result = self.run_tests(SUITES, [ALL[:2], ALL[3:]], IGNORED)
        self.assertEqual(result.returncode, 1)
        self.assertIn("tests listed but not run by any shard: 1", result.stdout)
        self.assertIn("beamcraft-cli::cli b::three", result.stdout)

    def test_a_lost_shard_report_fails(self):
        result = self.run_tests(SUITES, [ALL[:2]], IGNORED)
        self.assertEqual(result.returncode, 1)
        self.assertIn("not run by any shard: 2", result.stdout)

    def test_a_test_run_twice_fails(self):
        result = self.run_tests(SUITES, [ALL, ALL[:1]], IGNORED)
        self.assertEqual(result.returncode, 1)
        self.assertIn("run more than once", result.stdout)

    def test_same_name_in_two_binaries_is_two_tests(self):
        suites = {"x::one": {"t": "run"}, "x::two": {"t": "run"}}
        result = self.run_tests(suites, [[("x::one", "t")]])
        self.assertEqual(result.returncode, 1)
        self.assertIn("x::two t", result.stdout)

    def test_a_test_outside_the_list_fails(self):
        result = self.run_tests(SUITES, [ALL + [("beamcraft-cli::cli", "c::new")]], IGNORED)
        self.assertEqual(result.returncode, 1)
        self.assertIn("run but absent from the list", result.stdout)

    def test_a_filtered_test_fails(self):
        suites = {**SUITES, "beamcraft-kernel::g09": {"fuzz": "default-filter"}}
        result = self.run_tests(suites, [ALL], IGNORED)
        self.assertEqual(result.returncode, 1)
        self.assertIn("left out by a filter", result.stdout)
        self.assertIn("default-filter", result.stdout)

    def test_a_newly_ignored_test_fails(self):
        suites = {**SUITES, "beamcraft-host::recovery": {"torn": "ignored"}}
        result = self.run_tests(suites, [ALL[:3]], IGNORED)
        self.assertEqual(result.returncode, 1)
        self.assertIn("newly #[ignore]d", result.stdout)

    def test_a_stale_ignored_entry_fails(self):
        ignored = IGNORED + "beamcraft-cli::cli a::one\n"
        result = self.run_tests(SUITES, [ALL], ignored)
        self.assertEqual(result.returncode, 1)
        self.assertIn("not ignored tests any more", result.stdout)

    def test_a_retried_test_fails(self):
        result = self.run_tests(SUITES, [ALL], IGNORED, retried={ALL[0]})
        self.assertEqual(result.returncode, 1)
        self.assertIn("retried", result.stdout)

    def test_skipped_testcases_do_not_count_as_run(self):
        listing = self.path("list.json", nextest_list(SUITES))
        ignored_file = self.path("ignored.txt", IGNORED)
        report = junit(ALL).replace(
            '<testcase name="torn" classname="beamcraft-host::recovery" time="0.1">',
            '<testcase name="torn" classname="beamcraft-host::recovery" time="0.1"><skipped/>',
        )
        result = self.run_gate(
            "tests", "--list", listing, "--ignored", ignored_file, self.path("j.xml", report)
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("beamcraft-host::recovery torn", result.stdout)

    def test_a_broken_report_is_an_error(self):
        listing = self.path("list.json", nextest_list(SUITES))
        ignored_file = self.path("ignored.txt", IGNORED)
        result = self.run_gate(
            "tests", "--list", listing, "--ignored", ignored_file, self.path("j.xml", "<testsuites")
        )
        self.assertEqual(result.returncode, 2)


class JobsCheck(GateCase):
    def jobs(self, event, results):
        needs = {job: {"result": result, "outputs": {}} for job, result in results.items()}
        return self.run_gate(
            "jobs",
            "--event",
            event,
            "--needs",
            self.path("needs.json", needs),
            "--workflow",
            self.path("ci.yml", WORKFLOW),
        )

    def test_pull_request_may_skip_windows(self):
        result = self.jobs(
            "pull_request",
            {"quality": "success", "test-windows": "skipped", "test-archive-windows": "skipped"},
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_push_may_not_skip_windows(self):
        result = self.jobs(
            "push",
            {"quality": "success", "test-windows": "skipped", "test-archive-windows": "success"},
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("test-windows: skipped is not allowed on a push run", result.stdout)

    def test_a_skipped_job_outside_the_list_fails(self):
        result = self.jobs(
            "pull_request",
            {"quality": "skipped", "test-windows": "skipped", "test-archive-windows": "skipped"},
        )
        self.assertEqual(result.returncode, 1)

    def test_failed_and_cancelled_jobs_fail(self):
        for bad in ("failure", "cancelled"):
            result = self.jobs(
                "push",
                {"quality": bad, "test-windows": "success", "test-archive-windows": "success"},
            )
            self.assertEqual(result.returncode, 1, bad)

    def test_a_job_the_gate_does_not_wait_for_fails(self):
        result = self.jobs("push", {"quality": "success", "test-windows": "success"})
        self.assertEqual(result.returncode, 1)
        self.assertIn("does not wait for", result.stdout)
        self.assertIn("test-archive-windows", result.stdout)

    def prose_jobs(self, event, prose, results):
        workflow = "jobs:\n" + "".join(f"  {job}:\n    runs-on: x\n" for job in results) + "  ci-ok:\n"
        needs = {job: {"result": result} for job, result in results.items()}
        return self.run_gate(
            "jobs",
            "--event",
            event,
            "--prose",
            prose,
            "--needs",
            self.path("needs.json", needs),
            "--workflow",
            self.path("ci.yml", workflow),
        )

    HEAVY = ("quality", "test-archive-ubuntu", "test-ubuntu", "test-archive-macos", "test-macos",
             "build-timings", "wasm")

    def results(self, heavy, docs, windows="skipped"):
        out = {job: heavy for job in self.HEAVY}
        out.update({"changes": "success", "deny": "success", "docs-checks": docs,
                    "test-archive-windows": windows, "test-windows": windows})
        return out

    def test_a_prose_pull_request_may_skip_the_builds_and_runs_the_document_checks(self):
        result = self.prose_jobs("pull_request", "true", self.results("skipped", "success"))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_prose_pull_request_without_the_document_checks_fails(self):
        result = self.prose_jobs("pull_request", "true", self.results("skipped", "skipped"))
        self.assertEqual(result.returncode, 1)
        self.assertIn("docs-checks: skipped is not allowed", result.stdout)

    def test_a_code_pull_request_may_not_skip_the_builds(self):
        result = self.prose_jobs("pull_request", "false", self.results("skipped", "skipped"))
        self.assertEqual(result.returncode, 1)
        self.assertIn("quality: skipped is not allowed", result.stdout)

    def test_a_code_pull_request_skips_only_the_document_checks(self):
        result = self.prose_jobs("pull_request", "false", self.results("success", "skipped"))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_prose_is_ignored_on_a_push(self):
        result = self.prose_jobs("push", "true", self.results("skipped", "success", "success"))
        self.assertEqual(result.returncode, 1)
        self.assertIn("quality: skipped is not allowed on a push run", result.stdout)

    def test_the_real_workflow_gates_every_job(self):
        workflow = os.path.join(os.path.dirname(GATE), "..", ".github", "workflows", "ci.yml")
        with open(workflow, encoding="utf-8") as handle:
            text = handle.read()
        ids = set(__import__("re").findall(r"(?m)^  ([A-Za-z0-9_-]+):\s*$", text.split("\njobs:", 1)[1]))
        self.assertIn("ci-ok", ids)
        needs = {job: {"result": "success"} for job in ids - {"ci-ok"}}
        result = self.run_gate(
            "jobs",
            "--event",
            "push",
            "--needs",
            self.path("needs.json", needs),
            "--workflow",
            workflow,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
