#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Gate a branch before it becomes an upstream pull request. A cumulative
# branch can silently revert other people's upstream commits; the gate
# refuses a stale base and upstream file deletions before the PR is opened.
# Read-only: it checks the named branch against upstream/master and never
# pushes, merges or rewrites anything. See CONTRIBUTING.md, step 8.
#
# Usage:
#   bash tools/upstream-pr-gate.sh <head-branch>
#   bash tools/upstream-pr-gate.sh <head-branch> --allow-delete <path> [<path> ...]
#
# Exit codes: 0 checks passed, 1 a check failed, 2 usage or environment.
set -euo pipefail

EXIT_OK=0
EXIT_FAIL=1
EXIT_USAGE=2

usage() {
    cat <<'EOF'
Usage: bash tools/upstream-pr-gate.sh <head-branch> [--allow-delete <path> ...]

Runs three read-only checks of <head-branch> against the current
upstream/master (the 'upstream' remote must exist):
  1. fresh base: the branch merge-base equals upstream/master;
  2. the diff deletes no upstream files unless every deleted path is
     listed after --allow-delete;
  3. prints a summary: changed files, added and deleted lines.
EOF
    exit $EXIT_USAGE
}

die() {
    echo "upstream-pr-gate: $1" >&2
    exit $EXIT_FAIL
}

[ $# -ge 1 ] || usage
case $1 in
-h|--help) usage ;;
esac
branch=$1
shift

allow_delete=0
if [ $# -gt 0 ]; then
    [ "$1" = "--allow-delete" ] || usage
    allow_delete=1
    shift
fi

git rev-parse --verify --quiet "$branch^{commit}" >/dev/null \
    || die "branch not found: $branch (run from the fork clone)"
git rev-parse --verify --quiet refs/remotes/upstream/master >/dev/null \
    || die "no upstream/master: add the Apibim-com remote as 'upstream' and fetch it"

base=$(git merge-base "$branch" upstream/master)
tip=$(git rev-parse refs/remotes/upstream/master)

# 1. Fresh base: the branch must contain the current upstream/master tip.
if [ "$base" != "$tip" ]; then
    missed=$(git rev-list --count "$base..$tip")
    die "stale base: upstream/master has $missed commit(s) this branch does not contain; rebase or merge upstream/master into $branch and rerun"
fi

# 2. Upstream files must not be deleted: with a fresh base every path shown
# as deleted exists in upstream/master.
deleted=$(git diff --diff-filter=D --name-only "$base" "$branch")
if [ -n "$deleted" ]; then
    if [ "$allow_delete" -eq 0 ]; then
        echo "upstream-pr-gate: the branch deletes files that exist in upstream/master:" >&2
        printf '%s\n' "$deleted" | sed 's/^/  D /' >&2
        die "refusing: rerun with --allow-delete <path> ... listing every path meant to be deleted"
    fi
    unexpected=$(printf '%s\n' "$deleted" | while IFS= read -r path; do
        covered=0
        if [ $# -gt 0 ]; then
            for allowed in "$@"; do
                case $path in
                "$allowed"|"$allowed"/*) covered=1; break ;;
                esac
            done
        fi
        if [ "$covered" -eq 0 ]; then
            printf '%s\n' "$path"
        fi
    done)
    if [ -n "$unexpected" ]; then
        echo "upstream-pr-gate: deletions not covered by --allow-delete:" >&2
        printf '%s\n' "$unexpected" | sed 's/^/  D /' >&2
        die "refusing: add each path meant to be deleted to --allow-delete"
    fi
    echo "upstream-pr-gate: allowed deletions:"
    printf '%s\n' "$deleted" | sed 's/^/  D /'
fi

# 3. Summary.
files=$(git diff --name-only "$base" "$branch" | awk 'END { print NR }')
counts=$(git diff --numstat "$base" "$branch" \
    | awk '{ add += $1; del += $2 } END { printf "%d %d", add + 0, del + 0 }')

echo "upstream-pr-gate: OK"
echo "  base: $(git rev-parse --short "$tip") (upstream/master)"
echo "  head: $(git rev-parse --short "$branch") ($branch)"
echo "  diff: $files file(s) changed, +$(printf '%s' "$counts" | cut -d ' ' -f 1) -$(printf '%s' "$counts" | cut -d ' ' -f 2)"

exit $EXIT_OK
