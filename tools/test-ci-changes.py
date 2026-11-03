#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
# SPDX-FileCopyrightText: 2026 Apibim
"""Tests for tools/ci-changes.py, each against a small synthetic repository and run as CI runs
it: a separate process that prints `prose=true|false`."""

import os
import subprocess
import sys
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ci-changes.py")
ROOT = os.path.join(os.path.dirname(SCRIPT), "..")

SOURCES = {
    # The same shapes the repository uses to read documents.
    "apps/boundaries/src/table.rs": 'pub const DOCUMENT_PATH: &str = "docs/contracts/f08.md";\n',
    "apps/beamcraft-cli/tests/registry.rs": 'let path = concat!(env!("CARGO_MANIFEST_DIR"), "/COMMANDS.md");\n',
    "crates/kernel/tests/g03.rs": 'fn root() { join("../../docs/fixtures/g03") }\n'
    'fn param() { format!("{}/../../docs/tables/{name}.tsv", dir) }\n',
    "crates/kernel/Cargo.toml": '[dependencies]\nbeamcraft-geom = { path = "../geom" }\n',
    # A comment that names a document is not a read.
    "crates/engine/src/lib.rs": "//! See docs/design/engine.md for the design.\n",
}


class ChangesCase(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        for path, text in SOURCES.items():
            full = os.path.join(self.dir.name, path)
            os.makedirs(os.path.dirname(full), exist_ok=True)
            with open(full, "w", encoding="utf-8") as handle:
                handle.write(text)

    def classify(self, *changed, raw=None):
        listing = os.path.join(self.dir.name, "changed.txt")
        with open(listing, "w", encoding="utf-8") as handle:
            handle.write(raw if raw is not None else "".join(f"{path}\n" for path in changed))
        env = dict(os.environ)
        env.pop("GITHUB_STEP_SUMMARY", None)
        result = subprocess.run(
            [sys.executable, SCRIPT, listing, "--root", self.dir.name],
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip(), result.stderr

    def test_prose_nobody_reads_is_prose(self):
        out, why = self.classify("docs/design/engine.md", "README.md", "AGENTS.md")
        self.assertEqual(out, "prose=true", why)

    def test_a_file_that_is_not_markdown_runs_everything(self):
        out, why = self.classify("docs/design/engine.md", ".github/workflows/ci.yml")
        self.assertEqual(out, "prose=false")
        self.assertIn("ci.yml is not Markdown", why)

    def test_a_document_named_by_a_literal_runs_everything(self):
        out, why = self.classify("docs/contracts/f08.md")
        self.assertEqual(out, "prose=false")
        self.assertIn("read by the code", why)

    def test_a_crate_relative_literal_resolves_inside_its_crate(self):
        self.assertEqual(self.classify("apps/beamcraft-cli/COMMANDS.md")[0], "prose=false")
        self.assertEqual(self.classify("COMMANDS.md")[0], "prose=true")

    def test_a_directory_literal_covers_every_file_below_it(self):
        self.assertEqual(self.classify("docs/fixtures/g03/README.md")[0], "prose=false")
        self.assertEqual(self.classify("docs/fixtures/README.md")[0], "prose=true")

    def test_a_placeholder_names_the_directory_before_it(self):
        self.assertEqual(self.classify("docs/tables/notes.md")[0], "prose=false")

    def test_a_crate_directory_named_by_a_path_dependency_counts_as_read(self):
        self.assertEqual(self.classify("crates/geom/README.md")[0], "prose=false")

    def test_an_empty_or_huge_list_runs_everything(self):
        self.assertEqual(self.classify(raw="")[0], "prose=false")
        self.assertEqual(self.classify(*[f"docs/n{i}.md" for i in range(3000)])[0], "prose=false")

    def test_the_real_repository_reads_its_known_documents(self):
        listing = os.path.join(self.dir.name, "real.txt")
        cases = {
            "docs/contracts/f08-module-boundaries-v0.1.md": "prose=false",
            "docs/fixtures/expectations.md": "prose=false",
            "apps/beamcraft-cli/COMMANDS.md": "prose=false",
            "docs/fixtures/g03/README.md": "prose=false",
            "ROADMAP.md": "prose=true",
        }
        for path, expected in cases.items():
            with open(listing, "w", encoding="utf-8") as handle:
                handle.write(path + "\n")
            result = subprocess.run(
                [sys.executable, SCRIPT, listing, "--root", ROOT],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.stdout.strip(), expected, f"{path}: {result.stderr}")


if __name__ == "__main__":
    unittest.main()
