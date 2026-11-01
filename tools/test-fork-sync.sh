#!/usr/bin/env bash
# Tests for tools/fork-sync.sh: synthetic repositories with bare origin and
# upstream remotes plus a fake `cargo` let the real script run end to end
# without any external writes. Guards the conflict policy directly: fork-only
# conflicts keep the fork version and the sync proceeds, while any shared-path
# conflict must abort the merge and push nothing — the old auto-keep-all form
# silently deleted upstream-added paths (the 2026-11 steel crate) instead.

set -u

ROOT=$(cd "$(dirname "$0")" && pwd)
SYNC=$ROOT/fork-sync.sh
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
FAILED=0

note() {
	echo "$@"
}

assert_rc() {
	# assert_rc <want> <got> <label>
	if [ "$1" != "$2" ]; then
		echo "FAIL $3: exit $2, want $1"
		FAILED=1
	fi
}

assert_contains() {
	case $1 in
	*"$2"*) ;;
	*) echo "FAIL $3: output is missing '$2'"; FAILED=1 ;;
	esac
}

assert_lacks() {
	case $1 in
	*"$2"*)
		echo "FAIL $3: output unexpectedly contains '$2'"
		FAILED=1
		;;
	esac
}

assert_file_equals() {
	# assert_file_equals <path> <want> <label>
	local got
	got=$(cat "$1" 2>/dev/null || echo "<missing>")
	if [ "$got" != "$2" ]; then
		echo "FAIL $3: got '$got', want '$2'"
		FAILED=1
	fi
}

# The fake cargo: the gate calls exit 0, sync-trailer prints a receipt line,
# verify-sync accepts it. The script under test only checks exit codes and
# captures the trailer, so the shapes stay minimal.
make_fake_cargo() {
	mkdir -p "$TMP/bin"
	cat >"$TMP/bin/cargo" <<'SHIM'
#!/usr/bin/env bash
case " $* " in
*" sync-trailer "*) echo "Tested-sync-tree: stub-receipt-for-harness"; exit 0 ;;
esac
exit 0
SHIM
	chmod +x "$TMP/bin/cargo"
}

# make_repo <scenario-root> <with-shared>: builds <root>/repo on top of a base
# commit, an upstream side with a conflicting fork-only edit, a divergent
# clean path and an edit to doomed.txt (deleted fork-side), and a fork side.
# with-shared=yes additionally conflicts shared.txt on both sides.
make_repo() {
	local root=$1 with_shared=$2 dir
	dir=$root/repo
	mkdir -p "$dir/tools" "$dir/docs"
	git -C "$dir" init -q -b master
	git -C "$dir" config user.email fork-sync-test@example.invalid
	git -C "$dir" config user.name "fork sync test"
	cat >"$dir/tools/upstream-mirror.sh" <<'LIST'
# synthetic mirror stub: the fork-only list the sync reads from HEAD
FORK_ONLY="docs/fork-only.md tools/upstream-mirror.sh doomed.txt"
LIST
	printf 'fork-only base\n' >"$dir/docs/fork-only.md"
	printf 'shared base\n' >"$dir/shared.txt"
	printf 'clean base\n' >"$dir/clean.txt"
	printf 'doomed base\n' >"$dir/doomed.txt"
	git -C "$dir" add -A
	git -C "$dir" commit -qm base

	git -C "$dir" clone -q --bare . "$root/origin.git"
	git -C "$dir" clone -q --bare . "$root/upstream.git"
	git -C "$dir" remote add origin "$root/origin.git"
	git -C "$dir" remote add upstream "$root/upstream.git"
	git -C "$dir" push -q origin master

	git -C "$dir" checkout -qb upstream-side
	printf 'fork-only upstream\n' >"$dir/docs/fork-only.md"
	printf 'clean base\nclean upstream line\n' >"$dir/clean.txt"
	printf 'doomed upstream edit\n' >"$dir/doomed.txt"
	if [ "$with_shared" = yes ]; then
		printf 'shared upstream\n' >"$dir/shared.txt"
	fi
	git -C "$dir" add -A
	git -C "$dir" commit -qm upstream-work
	git -C "$dir" push -q upstream upstream-side:master
	git -C "$dir" checkout -q master

	printf 'fork-only fork\n' >"$dir/docs/fork-only.md"
	if [ "$with_shared" = yes ]; then
		printf 'shared fork\n' >"$dir/shared.txt"
	fi
	git -C "$dir" rm -q doomed.txt
	git -C "$dir" add -A
	git -C "$dir" commit -qm fork-work
	git -C "$dir" push -q origin master
}

run_sync() {
	# run_sync <repo-dir> <log-path>; writes the exit code to <log-path>.rc
	local dir=$1 log=$2
	(
		cd "$dir" || exit 1
		PATH="$TMP/bin:$PATH" bash "$SYNC" sync --yes
	) >"$log" 2>&1
	echo $? >"$log.rc"
}

make_fake_cargo

note "== scenario A: fork-only conflicts resolve, clean paths merge, sync pushes =="
make_repo "$TMP/A" no
run_sync "$TMP/A/repo" "$TMP/A.log"
assert_rc 0 "$(cat "$TMP/A.log.rc")" "A: sync exits 0"
assert_contains "$(cat "$TMP/A.log")" \
	"kept the fork version of the fork-only conflicted file: docs/fork-only.md" \
	"A: fork-only conflict keeps the fork version"
assert_file_equals "$TMP/A/repo/docs/fork-only.md" "fork-only fork" \
	"A: fork-only file content is the fork version"
assert_file_equals "$TMP/A/repo/clean.txt" "clean base
clean upstream line" \
	"A: clean path takes the normal merge result"
if [ -e "$TMP/A/repo/doomed.txt" ]; then
	echo "FAIL A: fork deletion of the fork-only doomed.txt was not preserved"
	FAILED=1
fi
if [ "$(git -C "$TMP/A/repo" rev-parse master^2)" != "$(git -C "$TMP/A/repo" rev-parse upstream/master)" ]; then
	echo "FAIL A: merge commit lacks upstream as its second parent"
	FAILED=1
fi
if [ "$(git -C "$TMP/A/repo" rev-parse master)" != "$(git -C "$TMP/A/repo" rev-parse origin/master)" ]; then
	echo "FAIL A: origin/master was not updated to the merge commit"
	FAILED=1
fi
if [ -n "$(git -C "$TMP/A/repo" status --porcelain)" ]; then
	echo "FAIL A: working tree is dirty after the sync"
	FAILED=1
fi

note "== scenario B: a shared-path conflict aborts the merge and pushes nothing =="
make_repo "$TMP/B" yes
master_before=$(git -C "$TMP/B/repo" rev-parse master)
run_sync "$TMP/B/repo" "$TMP/B.log"
assert_rc 2 "$(cat "$TMP/B.log.rc")" "B: sync exits 2"
assert_contains "$(cat "$TMP/B.log")" "needs decision: shared.txt" \
	"B: shared path is listed as needing a decision"
assert_contains "$(cat "$TMP/B.log")" \
	"kept the fork version of the fork-only conflicted file: docs/fork-only.md" \
	"B: fork-only path is still resolved before the stop"
assert_contains "$(cat "$TMP/B.log")" "nothing was merged or pushed" \
	"B: the stop explains that nothing was merged or pushed"
if git -C "$TMP/B/repo" rev-parse -q --verify MERGE_HEAD >/dev/null 2>&1; then
	echo "FAIL B: the merge was not aborted (MERGE_HEAD survives)"
	FAILED=1
fi
if [ -n "$(git -C "$TMP/B/repo" status --porcelain)" ]; then
	echo "FAIL B: working tree is dirty after the abort"
	FAILED=1
fi
if [ "$(git -C "$TMP/B/repo" rev-parse master)" != "$master_before" ]; then
	echo "FAIL B: local master moved"
	FAILED=1
fi
if [ "$(git -C "$TMP/B/repo" rev-parse origin/master)" != "$master_before" ]; then
	echo "FAIL B: something was pushed to origin"
	FAILED=1
fi

note "== scenario C: the auto-keep-all regression stays gone =="
# The old behaviour logged 'skipped upstream changes to conflicted file' for
# shared paths and deleted upstream-added content; neither may come back.
assert_lacks "$(cat "$TMP/B.log")" \
	"skipped upstream changes to conflicted file (kept fork): shared.txt" \
	"C: shared conflicts are not auto-kept anymore"

if [ "$FAILED" -eq 0 ]; then
	note "all fork-sync tests passed"
	exit 0
fi
note "fork-sync tests FAILED"
exit 1
