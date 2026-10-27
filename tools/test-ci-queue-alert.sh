#!/usr/bin/env bash
# Tests for tools/ci-queue-alert.sh: a fake `gh` serves realistic GitHub API
# envelopes, so the script's real decoding, pagination and issue-creation path
# runs end to end without any external writes.

set -u

ROOT=$(cd "$(dirname "$0")" && pwd)
WATCHER=$ROOT/ci-queue-alert.sh
FAILED=0

assert_eq() {
	[ "$1" = "$2" ] || {
		echo "FAIL $3: got '$1', want '$2'"
		FAILED=1
	}
}

assert_contains() {
	case $1 in
	*"$2"*) ;;
	*) echo "FAIL $3: output is missing '$2'"; FAILED=1 ;;
	esac
}

# Writes the fake gh into $1/bin; fixtures in $1/fixtures, log in $1/log.
# The generator lines are intentionally single-quoted: the stub must receive
# them unexpanded.
# shellcheck disable=SC2016,SC2028
setup_fake() {
	dir=$1
	mkdir -p "$dir/bin" "$dir/fixtures" "$dir/log"
	BIN=$dir/bin
	fixtures=$dir/fixtures
	log=$dir/log
	{
		echo '#!/usr/bin/env bash'
		echo 'set -u'
		echo 'method=GET; path='
		echo 'while [ $# -gt 0 ]; do'
		echo '  case $1 in'
		echo '    api) shift ;;'
		echo '    -X) method=$2; shift 2 ;;'
		echo '    -H) shift 2 ;;'
		echo '    --input|-) shift ;;'
		echo '    -*) shift ;;'
		echo '    *) [ -z "$path" ] && path=$1; shift ;;'
		echo '  esac'
		echo 'done'
		echo 'echo "$method $path" >> "$LOG/get.log"'
		echo 'if [ "$method" = POST ] && [ "${path##*/}" = issues ]; then'
		echo '  cat >> "$LOG/issues-post.jsonl"'
		echo '  echo "{\"number\": 1, \"html_url\": \"https://github.com/o/r/issues/1\"}"'
		echo '  exit 0'
		echo 'fi'
		echo 'if [ "$method" = POST ] && [ "${path##*/}" = labels ]; then'
		echo '  cat >> "$LOG/labels-post.jsonl"'
		echo '  echo "{}"'
		echo '  exit 0'
		echo 'fi'
		echo 'page=1; case $path in *page=2*) page=2 ;; esac'
		echo 'empty_runs() { printf "{\"total_count\": 0, \"workflow_runs\": []}"; }'
		echo 'empty_jobs() { printf "{\"total_count\": 0, \"jobs\": []}"; }'
		echo 'run_id=$(printf "%s" "$path" | sed -E "s#.*/runs/([0-9]+)/jobs.*#\\1#")'
		echo 'case $path in'
		echo '  *"/actions/runs?status=queued"*)'
		echo '    [ -f "$FIX/runs-queued-$page.json" ] && cat "$FIX/runs-queued-$page.json" || empty_runs ;;'
		echo '  *"/actions/runs?status=in_progress"*)'
		echo '    [ -f "$FIX/runs-inprogress-$page.json" ] && cat "$FIX/runs-inprogress-$page.json" || empty_runs ;;'
		echo '  *"/jobs"*)'
		echo '    if [ -f "$FIX/jobs-$run_id.json" ]; then cat "$FIX/jobs-$run_id.json"; else empty_jobs; fi ;;'
		echo '  *"issues?state=open"*)'
		echo '    [ -f "$FIX/issues.json" ] && cat "$FIX/issues.json" || printf "[]" ;;'
		echo '  *"/labels"*)'
		echo '    [ -f "$FIX/labels.json" ] && cat "$FIX/labels.json" || printf "[]" ;;'
		echo '  *) printf "{\"message\": \"unexpected path\"}"; exit 1 ;;'
		echo 'esac'
	} > "$dir/bin/gh"
	chmod +x "$dir/bin/gh"
}

# Runs the watcher with the fake gh; stdout+stderr in $OUT, exit code in $CODE.
run_watcher() {
	OUT=$(PATH="$BIN:$PATH" LOG="$log" FIX="$fixtures" GITHUB_TOKEN=test-token \
		GITHUB_REPOSITORY=o/r bash "$WATCHER" 2>&1)
	CODE=$?
	if [ -n "${DEBUG:-}" ]; then
		printf '%s\n' "$OUT"
		echo '== get.log =='
		cat "$log/get.log" 2> /dev/null
	fi
}

post_count() {
	if [ -f "$log/issues-post.jsonl" ]; then
		jq -s 'length' < "$log/issues-post.jsonl"
	else
		echo 0
	fi
}

# Envelopes shaped like the real Actions API responses.
run_json() {
	jq -n --argjson id "$1" --arg branch "$2" \
		'{total_count: 1, workflow_runs: [{id: $id, head_branch: $branch, html_url: ("https://github.com/o/r/actions/runs/" + ($id | tostring))}]}'
}
job_json() {
	jq -n --argjson id "$1" --arg status "$2" --arg created "$3" --arg name "$4" \
		'{id: $id, name: $name, status: $status, created_at: $created}'
}

# Wraps job documents from stdin into one jobs envelope.
jobs_envelope() {
	jq -s '{total_count: length, jobs: .}'
}

## 1. An empty Actions response alerts nothing and writes nothing.
d=$(mktemp -d)
setup_fake "$d"
run_watcher
assert_eq "$CODE" "0" "empty response exit code"
assert_contains "$OUT" "No queued job exceeded the threshold." "empty response message"
assert_eq "$(post_count)" "0" "empty response issue posts"
rm -rf "$d"

## 2. A stale queued job inside an already-running workflow opens exactly one
##    issue; fresh and non-queued jobs never count; the label is created.
d=$(mktemp -d)
setup_fake "$d"
run_json 101 "queued-ref" > "$d/fixtures/runs-queued-1.json"
{
	job_json 1 queued "2099-01-01T00:00:00Z" "recent"
	job_json 2 in_progress "2020-01-01T00:00:00Z" "running, never queued"
} | jobs_envelope > "$d/fixtures/jobs-101.json"
run_json 202 "active-ref" > "$d/fixtures/runs-inprogress-1.json"
job_json 4 queued "2020-01-01T00:00:00Z" "queued behind dependency" | jobs_envelope > "$d/fixtures/jobs-202.json"
printf '[{"name": "bug"}]' > "$d/fixtures/labels.json"
run_watcher
assert_eq "$CODE" "0" "stale job exit code"
assert_contains "$OUT" "Opened one ci-queue issue for 1 queued job(s)." "stale job message"
assert_eq "$(post_count)" "1" "stale job opens one issue"
assert_eq "$(jq -s 'length' < "$d/log/labels-post.jsonl")" "1" "stale job creates the label"
grep -q "active-ref" "$d/log/issues-post.jsonl" || {
	echo "FAIL stale job: the issue body does not name the ref"
	FAILED=1
}
grep -q "status=queued" "$d/log/get.log" || {
	echo "FAIL stale job: the queued status was not scanned"
	FAILED=1
}
grep -q "status=in_progress" "$d/log/get.log" || {
	echo "FAIL stale job: the in_progress status was not scanned"
	FAILED=1
}
rm -rf "$d"

## 3. An existing open ci-queue issue suppresses the alert.
d=$(mktemp -d)
setup_fake "$d"
run_json 202 "active-ref" > "$d/fixtures/runs-inprogress-1.json"
printf '[{"number": 7, "title": "CI runner queue: main"}]' > "$d/fixtures/issues.json"
job_json 4 queued "2020-01-01T00:00:00Z" "queued behind dependency" | jobs_envelope > "$d/fixtures/jobs-202.json"
run_watcher
assert_eq "$CODE" "0" "suppression exit code"
assert_contains "$OUT" "An open ci-queue issue already exists" "suppression message"
assert_eq "$(post_count)" "0" "suppression opens no issue"
rm -rf "$d"

## 4. A pull request carrying the label is not an alert, so it does not suppress.
d=$(mktemp -d)
setup_fake "$d"
run_json 202 "active-ref" > "$d/fixtures/runs-inprogress-1.json"
job_json 4 queued "2020-01-01T00:00:00Z" "queued behind dependency" > "$d/fixtures/jobs-202.json"
printf '[{"number": 9, "pull_request": {"url": "https://github.com/o/r/pulls/9"}}]' > "$d/fixtures/issues.json"
job_json 4 queued "2020-01-01T00:00:00Z" "queued behind dependency" | jobs_envelope > "$d/fixtures/jobs-202.json"
run_watcher
assert_eq "$CODE" "0" "pr suppression exit code"
assert_contains "$OUT" "Opened one ci-queue issue" "pr suppression opens an issue"
assert_eq "$(post_count)" "1" "pr suppression issue posts"
rm -rf "$d"

## 5. Multi-page run collections are walked to the stale page.
d=$(mktemp -d)
setup_fake "$d"
jq -n '[range(1000; 1100) | {id: ., head_branch: "bulk", html_url: ("https://github.com/o/r/actions/runs/" + (. | tostring))}]' |
	jq '{total_count: 100, workflow_runs: .}' > "$d/fixtures/runs-inprogress-1.json"
run_json 202 "active-ref" > "$d/fixtures/runs-inprogress-2.json"
job_json 4 queued "2020-01-01T00:00:00Z" "queued behind dependency" | jobs_envelope > "$d/fixtures/jobs-202.json"
run_watcher
assert_eq "$CODE" "0" "multi-page exit code"
assert_contains "$OUT" "Opened one ci-queue issue for 1 queued job(s)." "multi-page message"
assert_eq "$(post_count)" "1" "multi-page issue posts"
rm -rf "$d"

## 6. A malformed collection shape fails loudly.
d=$(mktemp -d)
setup_fake "$d"
printf '[{"id": 1}]' > "$d/fixtures/runs-queued-1.json"
run_watcher
[ "$CODE" -ne 0 ] || {
	echo "FAIL malformed envelope: the watcher exited 0"
	FAILED=1
}
assert_contains "$OUT" "no recognizable workflow runs collection" "malformed envelope message"
rm -rf "$d"

if [ "$FAILED" -ne 0 ]; then
	echo 'ci-queue-alert tests failed.'
	exit 1
fi
echo 'ci-queue-alert tests passed.'
