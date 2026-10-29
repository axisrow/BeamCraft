#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Sync the fork with upstream: merge upstream/master into the local fork master,
# gate the result on the local fmt/clippy/test triple, then push origin/master.
# Conflicting files keep their entire fork version (including fork deletions);
# cleanly merged files take the normal Git merge result. After a
# green sync the fork-only commits upstream does not have yet are listed with
# the exact upstream-PR commands; the script never opens a PR itself. See
# CONTRIBUTING.md, "Keep the fork aligned".
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
           Conflicting files keep their entire fork version; other files merge
           normally. Skipped upstream changes require manual integration.
           An unresolved conflict stops the run with exit 2.
  status   read-only: divergence counts, pending upstream commits, tree
           cleanliness, best-effort conflict prediction.

Options:
  --dry-run  guards, fetch and conflict prediction only; touches no branch.
  --yes      skip the confirmation prompt (for unattended runs).

Exit codes: 0 ok or nothing to sync, 1 environment, 2 a conflict could not
be cleared, 3 gate failure, 4 push or other git failure.

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

# The fork-only paths are owned by tools/upstream-mirror.sh: this script reads
# the list from there instead of keeping a second copy in sync.
read_fork_only_list() {
    git cat-file -e HEAD:tools/upstream-mirror.sh 2>/dev/null ||
        die "tools/upstream-mirror.sh is missing from HEAD; no FORK_ONLY list to read" $EXIT_ENV
    local list
    list="$(git show HEAD:tools/upstream-mirror.sh | sed -n 's/^FORK_ONLY="\([^"]*\)"$/\1/p')"
    if [ -z "$list" ]; then
        die "tools/upstream-mirror.sh has no FORK_ONLY= line" $EXIT_ENV
    fi
    printf '%s\n' "$list"
}

is_fork_only() {
    local known
    # shellcheck disable=SC2086  # the FORK_ONLY list is space-separated paths
    for known in $FORK_ONLY; do
        [ "$1" = "$known" ] && return 0
    done
    return 1
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

# Skip upstream changes to the entire conflicted path, including clean hunks.
# HEAD is the pre-merge fork tree; absence there preserves a fork deletion.
resolve_one() {
    local path=$1
    if git cat-file -e "HEAD:$path" 2>/dev/null; then
        git checkout HEAD -- "$path" || return 1
    else
        git rm -f --quiet -- "$path" || return 1
    fi
    echo "fork-sync: skipped upstream changes to conflicted file (kept fork): $path"
}

resolve_conflicts() {
    local path left=""
    while IFS= read -r -d '' path; do
        resolve_one "$path" || { left="$left  $path"$'\n'; continue; }
    done < <(git diff --name-only --diff-filter=U -z)
    if [ -z "$left" ]; then
        # Belt for a resolution that claimed success but left a stage behind.
        left="$(git diff --name-only --diff-filter=U)"
        [ -z "$left" ] || left="$(printf '  %s\n' "$left")"
    fi
    if [ -n "$left" ]; then
        echo "fork-sync: could not preserve conflicted files; integrate manually in a separate branch:" >&2
        printf '%s' "$left" >&2
        git merge --abort || die "cannot abort failed merge" $EXIT_PUSH
        exit $EXIT_CONFLICT
    fi
}

run_triple() {
    echo "fork-sync: gate 1/3: cargo fmt --check"
    cargo fmt --check || return $?
    echo "fork-sync: gate 2/3: cargo clippy --workspace --all-targets -- -D warnings"
    cargo clippy --workspace --all-targets -- -D warnings || return $?
    echo "fork-sync: gate 3/3: cargo nextest run --workspace"
    cargo nextest run --workspace || return $?
    # nextest never runs doctests; keep them in the gate, exactly like CI.
    echo "fork-sync: doctests: cargo test --workspace --doc"
    cargo test --workspace --doc
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
    FORK_ONLY="$(read_fork_only_list)"

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
            echo "fork-sync: a sync skips upstream changes to these entire files and keeps the fork versions"
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
        if [ -z "$(git diff --name-only --diff-filter=U)" ]; then
            if git rev-parse -q --verify MERGE_HEAD >/dev/null; then
                git merge --abort || die "cannot abort failed merge" $EXIT_PUSH
            fi
            die "merge failed without file conflicts" $EXIT_PUSH
        fi
        echo "fork-sync: skipping conflicted files; keeping their fork versions"
        resolve_conflicts
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

    # Fork-only content is never dropped (issue #141): name the commits the
    # fork carries that upstream does not have yet and hand over the exact
    # commands for their upstream PR.
    report_unmirrored

    cleanup_branches
}

# A commit whose every path is fork-only has no upstream form; anything else
# the fork carries that upstream lacks is named with the commands for its
# upstream PR, which this script never opens itself. Equivalence is patch-id
# based, computed by hand: after a sync merge upstream/master is an ancestor
# of master, so the symmetric difference has an empty upstream side and git
# cherry cannot see equivalence at all. The scan is O(upstream history),
# which is fine for a fork at this size; batch it if that ever stops holding.
report_unmirrored() {
    local sha path mirrorable id candidates="" upstream_ids
    upstream_ids="$(git log -p --no-merges upstream/master |
        git patch-id --stable | cut -d' ' -f1)"
    while read -r sha; do
        [ -n "$sha" ] || continue
        mirrorable=false
        while read -r path; do
            if ! is_fork_only "$path"; then
                mirrorable=true
                break
            fi
        done < <(git diff-tree --no-commit-id --name-only -r --root "$sha")
        if [ "$mirrorable" = false ]; then
            continue
        fi
        id="$(git show "$sha" --format= | git patch-id --stable | cut -d' ' -f1)"
        if [ -n "$id" ] && printf '%s\n' "$upstream_ids" | grep -qxF "$id"; then
            continue
        fi
        candidates="$candidates $sha"
    done < <(git rev-list --no-merges master ^upstream/master)
    if [ -z "$candidates" ]; then
        return
    fi
    echo "fork-sync: fork-only commits not yet mirrored upstream:"
    # shellcheck disable=SC2086  # whitespace-separated SHAs, list only
    git show -s --format='  %h %s' $candidates
    echo "fork-sync: open the upstream PR by hand (this script never opens one):"
    echo "  git push origin master:refs/heads/upstream-sync/from-master"
    echo "  gh pr create --repo Apibim-com/BeamCraft --base master --head axisrow:upstream-sync/from-master"
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
