#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
# SPDX-FileCopyrightText: 2026 Apibim
"""Decide whether a pull request changes only prose, so CI may skip its Rust builds and tests.

    ci-changes.py CHANGED.txt [--root DIR]

CHANGED.txt lists the changed paths, one per line, relative to the repository root (renamed
files appear under both names). Prints `prose=true` only when every path is a Markdown file
that no program reads; otherwise `prose=false`, with the first reason on stderr. A file is
read when a string literal in a `.rs` or `.toml` file under apps/, crates/ or tools/ names it
or a directory above it, resolved the way the code resolves it: relative to the crate that
holds the literal when it starts with `/`, `../` or `{}/` (CARGO_MANIFEST_DIR), else relative
to the repository root. Anything unexpected counts as code, so a mistake runs the full CI.

Standard library only.
"""

import os
import posixpath
import re
import sys

SOURCE_ROOTS = ("apps", "crates", "tools")
LITERAL = re.compile(r'"((?:[^"\\\n]|\\.)*)"')
MAX_FILES = 3000  # the pull request files API stops listing here


def package_dir(path):
    """The crate directory of a source path such as crates/kernel/tests/x.rs."""
    parts = path.split("/")
    return "/".join(parts[:2]) if len(parts) > 2 and parts[0] in SOURCE_ROOTS else ""


def read_literals(root):
    """Every path-like string literal of the Rust and TOML sources, resolved to a repo path."""
    resolved = set()
    for top in SOURCE_ROOTS:
        for directory, dirs, files in os.walk(os.path.join(root, top)):
            dirs[:] = [d for d in dirs if d not in ("target", ".git")]
            for name in files:
                if not name.endswith((".rs", ".toml")):
                    continue
                full = os.path.join(directory, name)
                relative = os.path.relpath(full, root).replace(os.sep, "/")
                try:
                    with open(full, encoding="utf-8", errors="replace") as handle:
                        text = handle.read()
                except OSError:
                    continue
                for literal in LITERAL.findall(text):
                    path = resolve(literal, package_dir(relative))
                    if path:
                        resolved.add(path)
    return resolved


def resolve(literal, crate):
    """A repository path for a literal that looks like a file or directory path, or None."""
    if "/" not in literal or "://" in literal or " " in literal:
        return None
    value = literal
    while value.startswith("{}"):
        value = value[2:]
    # Stop at the first placeholder: `parameters/{name}.tsv` names the `parameters` directory.
    if "{" in value:
        value = value[: value.index("{")].rsplit("/", 1)[0]
    relative_to_crate = value.startswith(("/", "../", "./"))
    value = value.lstrip("/")
    base = crate if relative_to_crate and crate else ""
    path = posixpath.normpath(posixpath.join(base, value)) if value else ""
    if not path or path in (".", "/") or path.startswith("../"):
        return None
    return path


def read_changed(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return [line.strip() for line in handle if line.strip()]
    except OSError as error:
        print(f"cannot read {path}: {error}", file=sys.stderr)
        return None


def classify(changed, literals):
    """(prose, reason)"""
    if changed is None:
        return False, "the changed-file list could not be read"
    if not changed:
        return False, "the pull request lists no changed files"
    if len(changed) >= MAX_FILES:
        return False, f"{len(changed)} changed files: the list may be truncated"
    for path in changed:
        if not path.endswith(".md"):
            return False, f"{path} is not Markdown"
        for literal in literals:
            if path == literal or path.startswith(literal + "/"):
                return False, f"{path} is read by the code (literal {literal!r})"
    return True, f"{len(changed)} Markdown files, none read by the code"


def main(argv):
    root = "."
    args = list(argv)
    if "--root" in args:
        at = args.index("--root")
        root = args[at + 1]
        del args[at : at + 2]
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    prose, reason = classify(read_changed(args[0]), read_literals(root))
    print(f"prose={'true' if prose else 'false'}")
    print(f"{'prose only' if prose else 'full CI'}: {reason}", file=sys.stderr)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as out:
            out.write(f"- Changed files: {'prose only' if prose else 'full CI'} — {reason}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
