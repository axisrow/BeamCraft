#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Sync the fork with upstream: merge upstream/master into the local fork master,
# gate the result on the local fmt/clippy/test triple, then push origin/master.
# The fork inherits upstream 1:1: a merge conflict is an error to investigate,
# never something to auto-resolve. See CONTRIBUTING.md, "Keep the fork aligned".
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: bash tools/fork-sync.sh [sync] [--dry-run] [--yes]
       bash tools/fork-sync.sh status

Modes:
  sync     (default) fetch upstream, merge upstream/master into the local fork
           master, run the local triple (fmt --check, clippy -D warnings,
           test --workspace) BEFORE committing, push origin/master, then delete
           merged sync/* branches. No PR: master is the integration branch.
  status   read-only: divergence counts, pending upstream commits, tree
           cleanliness, best-effort conflict prediction.

Options:
  --dry-run  guards, fetch and conflict prediction only; touches no branch.
  --yes      skip the confirmation prompt (for unattended runs).

Exit codes: 0 ok or nothing to sync, 1 environment, 2 merge conflict,
3 gate failure, 4 push or other git failure.

The gate is the local triple only. Fork CI does not gate a sync: it has no
macOS lane and its Windows runners queue for hours.
EOF
}

EXIT_OK=0
EXIT_ENV=1
EXIT_CONFLICT=2
EXIT_GATE=3
EXIT_PUSH=4

die() {
    echo "fork-sync: $1" >&2
    exit "${2:-$EXIT_ENV}"
}

need_master_worktree() {
    local git_dir common_dir branch
    git_dir="$(git rev-parse --git-dir)" || die "not a git repository" $EXIT_ENV
    common_dir="$(git rev-parse --git-common-dir)" || die "not a git repository" $EXIT_ENV
    if [ "$git_dir" != "$common_dir" ]; then
        die "this is a linked worktree; run the sync from the main checkout" $EXIT_ENV
    fi
    branch="$(git branch --show-current)" || die "cannot read the current branch" $EXIT_ENV
    if [ "$branch" != "master" ]; then
        die "sync runs on master only; current branch: $branch" $EXIT_ENV
    fi
    if [ -n "$(git status --porcelain)" ]; then
        die "working tree is not clean; commit or stash first" $EXIT_ENV
    fi
    git remote get-url origin >/dev/null 2>&1 || die "no origin remote" $EXIT_ENV
    git remote get-url upstream >/dev/null 2>&1 || die "no upstream remote" $EXIT_ENV
    # A master tracking a foreign remote turns every habitual 'git pull' into a
    # fast-forward to the wrong history; refuse and hand over the repair.
    local tracking
    tracking="$(git config branch.master.remote || true)"
    if [ -n "$tracking" ] && [ "$tracking" != "origin" ]; then
        die "master tracks '$tracking', not origin. Fix: git branch --set-upstream-to=origin/master master" $EXIT_ENV
    fi
}

resolve_cargo() {
    if ! command -v cargo >/dev/null 2>&1; then
        # On the owner's Mac cargo lives in ~/.cargo/bin and is not in PATH.
        PATH="$HOME/.cargo/bin:$PATH"
        export PATH
    fi
    command -v cargo >/dev/null 2>&1 || die "cargo not found in PATH or ~/.cargo/bin" $EXIT_ENV
}

# Best-effort conflict prediction; prints the conflicting paths, one per line.
# merge-tree prints the merged tree OID on line 1, conflicted paths after it.
predict_conflicts() {
    local out
    out="$(git merge-tree --write-tree "$1" upstream/master 2>/dev/null || true)"
    printf '%s\n' "$out" | tail -n +2
}

# The branch a sync would merge from: after the fast-forward a stale local
# master becomes origin/master. Read-only, so status and --dry-run use it too.
sync_base() {
    if [ "$(git rev-list --count origin/master..master)" -eq 0 ] &&
        [ "$(git rev-list --count master..origin/master)" -gt 0 ]; then
        git rev-parse origin/master
    else
        git rev-parse master
    fi
}

# A stale local master is fast-forwarded to origin/master. A strictly ahead
# master is the unpushed merge commit of a previous rejected push: keep it.
# Anything else is real divergence and needs a human.
align_master() {
    local local_head origin_head
    local_head="$(git rev-parse master)"
    origin_head="$(git rev-parse origin/master)"
    if [ "$local_head" != "$origin_head" ]; then
        if [ "$(git rev-list --count origin/master..master)" -eq 0 ]; then
            echo "fork-sync: fast-forwarding local master to origin/master"
            git merge --ff-only origin/master
        elif [ "$(git rev-list --count master..origin/master)" -eq 0 ]; then
            echo "fork-sync: local master is ahead of origin/master (unpushed merge commit); proceeding"
        else
            die "local master has diverged from origin/master; reconcile by hand first" $EXIT_ENV
        fi
    fi
}

conflict_report() {
    local path
    echo "Merge conflict: a fork/upstream divergence the 1:1 contract does not expect." >&2
    echo "Conflicting paths:" >&2
    git diff --name-only --diff-filter=U | while read -r path; do
        echo "  $path" >&2
    done
    echo "" >&2
    echo "Investigate before re-running (never auto-resolve):" >&2
    echo "  - was a fork change not mirrored upstream yet?" >&2
    echo "  - or did upstream rework a fork-only artifact (icon pipeline, docs)?" >&2
    echo "  - git log master..upstream/master -- <path> and git log upstream/master..master -- <path>" >&2
}

run_triple() {
    echo "fork-sync: gate 1/3: cargo fmt --check"
    cargo fmt --check || return $?
    echo "fork-sync: gate 2/3: cargo clippy --workspace --all-targets -- -D warnings"
    cargo clippy --workspace --all-targets -- -D warnings || return $?
    echo "fork-sync: gate 3/3: cargo test --workspace"
    cargo test --workspace
}

# Merged sync branches accumulate one per sync; -d never loses unmerged work.
cleanup_branches() {
    local branch
    while read -r branch; do
        [ -n "$branch" ] || continue
        git branch -d "$branch" >/dev/null && echo "fork-sync: deleted merged branch $branch"
    done < <(git branch --merged master --format '%(refname:short)' | grep -E '^(sync/|chore/sync-upstream-)' || true)
}

merge_message() {
    local topics="" subject
    while read -r subject; do
        [ -n "$subject" ] || continue
        if [ -n "$topics" ]; then
            topics="$topics, $subject"
        else
            topics="$subject"
        fi
    done < <(git log --no-decorate --max-count=4 master..upstream/master --format=%s | cut -c1-48)
    echo "Merge upstream master into the fork: $topics"
}

do_sync() {
    local dry_run=$1
    need_master_worktree

    echo "fork-sync: fetching upstream and origin"
    git fetch upstream master
    git fetch origin master

    # Read-only planning runs before any mutation, so --dry-run stays true to
    # "touches no branch": the base is where a real sync would merge from.
    local base
    base="$(sync_base)"
    local pending
    pending="$(git rev-list --count "$base..upstream/master")"
    if [ "$pending" -eq 0 ]; then
        echo "fork-sync: upstream has no new commits, nothing to sync"
        exit $EXIT_OK
    fi
    echo "fork-sync: $pending upstream commit(s) to merge:"
    git log --oneline --no-decorate "$base..upstream/master"

    if [ "$dry_run" = true ]; then
        local conflicts
        conflicts="$(predict_conflicts "$base")"
        if [ -n "$conflicts" ]; then
            echo "fork-sync: dry-run predicts conflicts in:"
            echo "  ${conflicts//$'\n'/$'\n'  }"
        else
            echo "fork-sync: dry-run predicts a clean merge"
        fi
        echo "fork-sync: dry-run stops here; nothing was merged"
        exit $EXIT_OK
    fi

    align_master

    if [ "$assume_yes" = false ]; then
        printf 'Proceed with merge, local triple and push to origin/master? [y/N] '
        # EOF (closed stdin) means empty answer: abort, never guess.
        read -r answer || answer=""
        case "$answer" in
            y | Y | yes | Yes) ;;
            *) die "aborted by the user" $EXIT_OK ;;
        esac
    fi

    resolve_cargo

    if ! git merge --no-commit --no-ff upstream/master; then
        conflict_report
        git merge --abort
        exit $EXIT_CONFLICT
    fi

    if ! run_triple; then
        echo "fork-sync: the local triple failed; rolling the merge back" >&2
        git merge --abort
        exit $EXIT_GATE
    fi

    git commit -m "$(merge_message)"

    if ! git push origin master; then
        # The local merge commit is correct; the next run pushes it after a re-fetch.
        die "push to origin/master was rejected; the merge commit stays local" $EXIT_PUSH
    fi
    echo "fork-sync: origin/master updated"

    cleanup_branches
}

do_status() {
    local base behind ahead conflicts
    base="$(sync_base)"
    behind="$(git rev-list --count "$base..upstream/master")"
    ahead="$(git rev-list --count upstream/master..master)"
    echo "upstream commits not in master: $behind"
    echo "fork commits not in upstream (pending upstreaming): $ahead"
    local local_head origin_head
    local_head="$(git rev-parse master)"
    origin_head="$(git rev-parse origin/master)"
    if [ "$local_head" != "$origin_head" ]; then
        if [ "$(git rev-list --count origin/master..master)" -eq 0 ]; then
            echo "master is behind origin/master by $(git rev-list --count master..origin/master) (a sync fast-forwards it)"
        else
            echo "master has diverged from origin/master; reconcile by hand"
        fi
    else
        echo "master is in sync with origin/master"
    fi
    if [ "$behind" -gt 0 ]; then
        local pending_log
        pending_log="$(git log --oneline --no-decorate "$base..upstream/master")"
        echo "pending upstream commits:"
        echo "  ${pending_log//$'\n'/$'\n'  }"
        conflicts="$(predict_conflicts "$base")"
        if [ -n "$conflicts" ]; then
            echo "conflict prediction:"
            echo "  ${conflicts//$'\n'/$'\n'  }"
        else
            echo "conflict prediction: clean merge"
        fi
    fi
    if [ -n "$(git status --porcelain)" ]; then
        echo "working tree: dirty"
    else
        echo "working tree: clean"
    fi
}

mode=sync
dry_run=false
assume_yes=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        sync | status)
            mode="$1"
            ;;
        --dry-run)
            dry_run=true
            ;;
        --yes)
            assume_yes=true
            ;;
        --help | -h)
            usage
            exit $EXIT_OK
            ;;
        *)
            usage >&2
            die "unknown argument: $1" $EXIT_ENV
            ;;
    esac
    shift
done

if [ "$mode" = status ]; then
    if [ "$dry_run" = true ] || [ "$assume_yes" = true ]; then
        die "status takes no flags" $EXIT_ENV
    fi
    git rev-parse --git-dir >/dev/null 2>&1 || die "not a git repository" $EXIT_ENV
    do_status
    exit $EXIT_OK
fi

do_sync "$dry_run"
