#!/usr/bin/env bash
# Open one issue when a BeamCraft Actions job waits in the runner queue too long.
#
# Environment: GITHUB_TOKEN (or GH_TOKEN), GITHUB_REPOSITORY, GITHUB_API_URL,
# QUEUE_THRESHOLD_MINUTES (default 15). All API access goes through `gh`, JSON
# through `jq`; both are preinstalled on the GitHub runners.

set -eu

fail() {
	printf '%s\n' "$1" >&2
	exit 1
}

# The repository path used in API URLs, from GITHUB_REPOSITORY.
repository=${GITHUB_REPOSITORY:-}
case $repository in
*/*) ;;
*) fail 'GITHUB_TOKEN, GITHUB_REPOSITORY and a positive threshold are required' ;;
esac
slashes=${repository//[!\/]/}
[ "${#slashes}" -eq 1 ] || fail 'GITHUB_TOKEN, GITHUB_REPOSITORY and a positive threshold are required'
[ -n "${GITHUB_TOKEN:-${GH_TOKEN:-}}" ] || fail 'GITHUB_TOKEN, GITHUB_REPOSITORY and a positive threshold are required'
threshold=${QUEUE_THRESHOLD_MINUTES:-15}
case $threshold in
'' | *[!0-9]*) fail 'QUEUE_THRESHOLD_MINUTES must be an integer' ;;
esac
[ "$threshold" -ge 1 ] || fail 'GITHUB_TOKEN, GITHUB_REPOSITORY and a positive threshold are required'
repo="/repos/$repository"

# A GET of one page; the response body on stdout.
api_get() {
	gh api "$1" -H 'Accept: application/vnd.github+json' -H 'X-GitHub-Api-Version: 2022-11-28'
}

# A POST with a JSON body read from stdin.
api_post() {
	path=$1
	gh api -X POST "$path" --input -
}

# Streams every item of a paginated collection as NDJSON.
# $1 = API path, $2 = jq filter naming the collection, $3 = collection name for errors.
collect() {
	path=$1
	filter=$2
	name=$3
	sep='?'
	case $path in
	*\?*) sep='&' ;;
	esac
	for page in $(seq 1 100); do
		body=$(api_get "${path}${sep}per_page=100&page=${page}") || fail "gh api failed for ${path} (page ${page})"
		# Shape-aware extraction: Actions endpoints wrap their collection in an
		# object; issues and labels are top-level arrays. Anything else, or a
		# collection field that is not an array, is rejected here.
		printf '%s' "$body" | jq "$filter" > /dev/null 2>&1 || fail "GitHub API returned no recognizable ${name} collection for ${path} (page ${page})"
		printf '%s' "$body" | jq -c "$filter | .[]"
		count=$(printf '%s' "$body" | jq "$filter | length")
		if [ "$count" -lt 100 ]; then
			return 0
		fi
	done
	fail "GitHub API pagination exceeded the 100-page safety limit for ${path}"
}

RUNS_FILTER='if type == "array" then error("expected the workflow_runs envelope") elif has("workflow_runs") and (.workflow_runs | type == "array") then .workflow_runs else error("expected the workflow_runs envelope") end'
JOBS_FILTER='if type == "array" then error("expected the jobs envelope") elif has("jobs") and (.jobs | type == "array") then .jobs else error("expected the jobs envelope") end'
ARRAY_FILTER='if type == "array" then . else error("expected a top-level array collection") end'

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
stale="$work/stale.ndjson"
: > "$stale"

scan_runs() {
	status=$1
	runs="$work/runs-$status.ndjson"
	collect "$repo/actions/runs?status=$status" "$RUNS_FILTER" 'workflow runs' > "$runs"
	while IFS= read -r run; do
		run_id=$(printf '%s' "$run" | jq -r '.id // empty')
		case $run_id in
		'' | *[!0-9]*) fail 'GitHub API returned a workflow run without an integer id' ;;
		esac
		ref=$(printf '%s' "$run" | jq -r '.head_branch // "unknown ref"')
		run_url=$(printf '%s' "$run" | jq -r '.html_url // ""')
		jobs="$work/jobs-$run_id.ndjson"
		collect "$repo/actions/runs/$run_id/jobs" "$JOBS_FILTER" 'jobs' > "$jobs"
		while IFS= read -r job; do
			printf '%s' "$job" | jq -c \
				--arg ref "$ref" --arg run_url "$run_url" --argjson t "$threshold" '
				if .status != "queued" then empty
				elif (.id | type) != "number" then error("queued job without an integer id")
				elif (.created_at | type) != "string" then error("queued job without created_at")
				else (.created_at | fromdateiso8601) as $queued
				| if $queued <= (now - $t * 60) then
					{
						job: ((.name // "unnamed job")[0:200]),
						run_id: .id,
						ref: $ref[0:200],
						queued_at: .created_at,
						run_url: $run_url[0:500]
					}
				else empty
				end
				end' >> "$stale" || fail 'GitHub API returned a queued job with an invalid id or created_at'
		done < "$jobs"
	done < "$runs"
}

scan_runs queued
scan_runs in_progress

if [ ! -s "$stale" ]; then
	echo 'No queued job exceeded the threshold.'
	exit 0
fi

issues="$work/issues.ndjson"
collect "$repo/issues?state=open&labels=ci-queue" "$ARRAY_FILTER" 'issues' > "$issues"
open_alerts=$(jq -s '[.[] | select(has("pull_request") | not)] | length' < "$issues")
if [ "$open_alerts" -gt 0 ]; then
	echo 'An open ci-queue issue already exists; skipping duplicate alert.'
	exit 0
fi

labels="$work/labels.ndjson"
collect "$repo/labels" "$ARRAY_FILTER" 'labels' > "$labels"
if [ "$(jq -s '[.[] | select(.name == "ci-queue")] | length' < "$labels")" -eq 0 ]; then
	jq -n '{name: "ci-queue", color: "d93f0b", description: "CI jobs waiting for hosted runners"}' |
		api_post "$repo/labels" > /dev/null
fi

refs=$(jq -r '.ref' "$stale" | sort -u)
ref_count=$(printf '%s\n' "$refs" | wc -l | tr -d ' ')
first_ref=$(printf '%s\n' "$refs" | head -n 1)
if [ "$ref_count" -eq 1 ]; then
	title_ref=$first_ref
else
	title_ref="$first_ref (+$((ref_count - 1)) refs)"
fi
title="CI runner queue: $title_ref"
[ "${#title}" -le 256 ] || title=${title:0:256}

{
	echo "The following CI jobs have remained queued for at least $threshold minutes."
	echo ''
	echo '| Ref | Job | Run | Queued since (UTC) |'
	echo '|---|---|---:|---|'
	while IFS= read -r item; do
		printf '%s\n' "$item" | jq -r '"| `\(.ref)` | `\(.job)` | [#\(.run_id)](\(.run_url)) | `\(.queued_at)` |"'
	done < "$stale"
	echo ''
	echo 'Close this issue after the queue drains. The watcher will then alert on a later incident.'
} > "$work/body.md"

jq -n --arg title "$title" --rawfile body "$work/body.md" \
	'{title: $title, body: $body, labels: ["ci-queue"]}' |
	api_post "$repo/issues" > /dev/null

job_count=$(wc -l < "$stale" | tr -d ' ')
echo "Opened one ci-queue issue for $job_count queued job(s)."
