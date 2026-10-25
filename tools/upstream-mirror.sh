#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Mirror a merged fork PR to upstream: replay its reviewed commits on a fresh
# upstream/master and open a mirror PR there. The counterpart of fork-sync.sh:
# upstream -> fork master is a merge (1:1 inheritance), but a fork PR goes to
# upstream as a cherry-pick chain, because upstream never takes the fork's
# master history. Commits already upstream drop out as empty cherry-picks.
# A conflict is an error to resolve by hand, never something to auto-merge.
# See CONTRIBUTING.md, "Mirror to upstream".
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: bash tools/upstream-mirror.sh mirror <pr-number> [--dry-run]
       bash tools/upstream-mirror.sh status

Modes:
  mirror    replay the merged fork PR's commits on upstream/master, push the
            replay branch and open a mirror PR in Apibim-com/BeamCraft.
  status    read-only: recent merged fork PRs and the upstream-sync branches.

Options:
  --dry-run  print the delta and its classification; touch no branch and open
             nothing.

Exit codes: 0 ok or a clean skip, 1 environment, 2 conflict or unexpected PR
shape, 3 publication failure.

The mirror job needs gh credentials that can read the fork and open a PR in
upstream (the UPSTREAM_SYNC_TOKEN secret in the workflow; the owner's gh
auth locally). Cargo is not involved: upstream CI gates the mirror PR.
EOF
}

EXIT_OK=0
EXIT_ENV=1
EXIT_CONFLICT=2
EXIT_PUBLISH=3

UPSTREAM_REPO="Apibim-com/BeamCraft"
FORK_REPO="axisrow/BeamCraft"

# Paths the fork keeps to itself: upstream has its own decision for each, and
# a conflict touching only these resolves to the upstream side (checkout --ours).
FORK_ONLY="tools/fork-sync.sh tools/upstream-mirror.sh .github/workflows/fork-sync.yml .github/workflows/upstream-mirror.yml .github/workflows/macos-icon-acceptance.yml"

die() {
    echo "upstream-mirror: $1" >&2
    exit "${2:-$EXIT_ENV}"
}

is_fork_only() {
    for known in $FORK_ONLY; do
        [ "$1" = "$known" ] && return 0
    done
    return 1
}

need_repo() {
    git rev-parse --git-dir >/dev/null 2>&1 || die "not a git repository" $EXIT_ENV
    [ -z "$(git status --porcelain)" ] || die "working tree is not clean; commit or stash first" $EXIT_ENV
    git remote get-url origin >/dev/null 2>&1 || die "no origin remote" $EXIT_ENV
    git remote get-url upstream >/dev/null 2>&1 || die "no upstream remote (git remote add upstream https://github.com/$UPSTREAM_REPO.git)" $EXIT_ENV
}

read_pr() {
    gh pr view "$1" --repo "$FORK_REPO" \
        --json number,title,state,baseRefName,mergeCommit 2>/dev/null ||
        die "cannot read fork PR #$1 (gh credentials?)" $EXIT_ENV
}

slug_of() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]' |
        sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//' | cut -c1-40
}

mirror_marker() {
    printf 'Mirror of %s#%s' "$FORK_REPO" "$1"
}

# A mirror PR for this fork PR must not exist yet, open or merged.
already_mirrored() {
    local count
    count="$(gh pr list --repo "$UPSTREAM_REPO" --state all --limit 5 \
        --search "\"$(mirror_marker "$1")\" in:body" --json number --jq 'length' 2>/dev/null || echo 0)"
    [ "$count" -gt 0 ]
}

# Resolve the conflicts of the in-progress cherry-pick to the upstream side if
# and only if every conflicted path is fork-only; anything else is a stop.
resolve_or_stop() {
    local commit=$1 conflicted path fork_only=true
    conflicted="$(git diff --name-only --diff-filter=U)"
    for path in $conflicted; do
        if ! is_fork_only "$path"; then
            fork_only=false
            break
        fi
    done
    if [ "$fork_only" = false ]; then
        echo "upstream-mirror: conflict replaying $(git log -1 --format='%h %s' "$commit"):" >&2
        # shellcheck disable=SC2086  # newline-separated paths, one per line
        printf '  %s\n' $conflicted >&2
        echo "Not a fork-only path; resolve by hand (never auto-merge):" >&2
        echo "  git cherry-pick --abort   # then replay manually" >&2
        exit $EXIT_CONFLICT
    fi
    # shellcheck disable=SC2086  # newline-separated paths as separate args
    git checkout --ours -- $conflicted
    # shellcheck disable=SC2086
    git add -A -- $conflicted
    git -c core.editor=true cherry-pick --continue >/dev/null
}

do_mirror() {
    local number=$1 dry_run=$2
    need_repo

    local pr merge_oid title
    pr="$(read_pr "$number")"
    [ "$(printf '%s' "$pr" | jq -r .state)" = "MERGED" ] ||
        die "PR #$number is not merged; nothing to mirror" $EXIT_ENV
    [ "$(printf '%s' "$pr" | jq -r .baseRefName)" = "master" ] ||
        die "PR #$number does not target master" $EXIT_ENV
    merge_oid="$(printf '%s' "$pr" | jq -r .mergeCommit.oid)"
    title="$(printf '%s' "$pr" | jq -r .title)"

    echo "upstream-mirror: fetching origin and upstream"
    git fetch origin master >/dev/null 2>&1
    git fetch upstream master >/dev/null 2>&1

    if already_mirrored "$number"; then
        echo "upstream-mirror: a mirror PR for #$number already exists upstream, skipping"
        exit $EXIT_OK
    fi

    # The PR's reviewed commits: the second parent of the merge commit is the
    # branch tip as it was reviewed; first parent is the master it landed on.
    if ! git rev-parse --verify --quiet "$merge_oid^2" >/dev/null; then
        die "PR #$number has no merge commit with a second parent (squash or rebase merge?); mirror it by hand" $EXIT_CONFLICT
    fi
    local commits
    commits="$(git rev-list --reverse --no-merges "$merge_oid^2" "^$merge_oid^1")"
    if [ -z "$commits" ]; then
        echo "upstream-mirror: PR #$number carries no non-merge commits, skipping"
        exit $EXIT_OK
    fi

    local files fork_only_delta=true file
    files="$(git diff --name-only "$merge_oid^1" "$merge_oid")"
    for file in $files; do
        if ! is_fork_only "$file"; then
            fork_only_delta=false
            break
        fi
    done
    if [ "$fork_only_delta" = true ]; then
        echo "upstream-mirror: PR #$number touches only fork-only paths, skipping"
        exit $EXIT_OK
    fi

    # shellcheck disable=SC2086  # newline-separated revisions, count only
    echo "upstream-mirror: PR #$number \"$title\": $(printf '%s\n' $commits | wc -l | tr -d ' ') commit(s):"
    # shellcheck disable=SC2086  # multiple revisions, no ancestry walk
    git show -s --format='  %h %s' $commits
    if [ "$dry_run" = true ]; then
        echo "upstream-mirror: dry run stops here; no branch, no PR"
        exit $EXIT_OK
    fi

    local branch
    branch="upstream-sync/pr-$number-$(slug_of "$title")"
    git checkout -B "$branch" upstream/master >/dev/null 2>&1

    local commit
    for commit in $commits; do
        if ! git cherry-pick "$commit" >/dev/null 2>&1; then
            if [ -n "$(git diff --name-only --diff-filter=U)" ]; then
                resolve_or_stop "$commit"
            else
                echo "upstream-mirror: already in upstream, dropped: $(git log -1 --format=%s "$commit")"
                git cherry-pick --skip >/dev/null 2>&1
            fi
        fi
    done

    if [ "$(git rev-list --count upstream/master..HEAD)" -eq 0 ]; then
        echo "upstream-mirror: every commit of #$number is already in upstream, skipping"
        git checkout master >/dev/null 2>&1
        git branch -D "$branch" >/dev/null
        exit $EXIT_OK
    fi

    # The upstream-sync namespace is automation-owned and single-writer (the
    # workflow's concurrency group serialises runs): --force lets a re-run
    # after a failure replace a stale branch instead of wedging forever on a
    # non-fast-forward rejection.
    if ! git push --force origin "HEAD:refs/heads/$branch" >/dev/null 2>&1; then
        die "could not push $branch to origin" $EXIT_PUBLISH
    fi

    local body
    body="$(mktemp)"
    {
        echo "Mirror of $FORK_REPO#$number — $title."
        echo
        echo "Carries the fork PR's reviewed commits verbatim, replayed on a fresh"
        echo "upstream/master by the fork's upstreaming automation (commits already"
        echo "upstream dropped out as empty):"
        echo
        git log --no-decorate --format='- %h %s' "upstream/master..HEAD"
        echo
        echo "## How you verified it"
        echo
        echo "- [x] The fork PR was reviewed and merged with its gates green; upstream CI"
        echo "      runs the full triple (fmt, clippy -D warnings, test) on this PR."
        echo
        echo "## Never-crash"
        echo
        echo "- [x] Replay of an already-reviewed fork PR: no new \`unwrap\`/\`expect\`/\`panic!\`,"
        echo "      no \`unsafe\`."
        echo
        echo "## Provenance"
        echo
        echo "- [x] Nothing taken from Tekla Structures or any Trimble product; no GPL/AGPL"
        echo "      code copied in."
        echo
        echo "🤖 Generated with [Claude Code](https://claude.com/claude-code)"
    } >"$body"

    if ! gh pr create --repo "$UPSTREAM_REPO" --base master --head "axisrow:$branch" \
        --title "$title" --body-file "$body"; then
        rm -f "$body"
        die "could not open the mirror PR in $UPSTREAM_REPO (UPSTREAM_SYNC_TOKEN?)" $EXIT_PUBLISH
    fi
    rm -f "$body"
    echo "upstream-mirror: mirror PR for #$number opened as $branch"
}

do_status() {
    need_repo
    echo "recently merged fork PRs:"
    gh pr list --repo "$FORK_REPO" --state merged --limit 10 \
        --json number,title,mergedAt --jq '.[] | "  #\(.number) \(.title)"' || true
    echo "upstream-sync branches on origin:"
    git branch -r --format='%(refname:short)' | grep '^origin/upstream-sync/' | sed 's/^/  /' || true
}

mode=mirror
number=""
dry_run=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        mirror | status)
            mode="$1"
            ;;
        --dry-run)
            dry_run=true
            ;;
        --help | -h)
            usage
            exit $EXIT_OK
            ;;
        -*)
            usage >&2
            die "unknown argument: $1" $EXIT_ENV
            ;;
        *)
            number="$1"
            ;;
    esac
    shift
done

if [ "$mode" = "status" ]; then
    [ -z "$number" ] && [ "$dry_run" = false ] || die "status takes no PR number and no flags" $EXIT_ENV
    do_status
    exit $EXIT_OK
fi

[ -n "$number" ] || { usage >&2; die "mirror needs a PR number" $EXIT_ENV; }
do_mirror "$number" "$dry_run"
