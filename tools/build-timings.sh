#!/usr/bin/env bash
# Measure CLI + UI compilation without reusing target artifacts or compiler wrappers.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: bash tools/build-timings.sh [--profile NAME] [--target TRIPLE] [--output DIR]

Defaults: release profile, host target, target/build-timings/<UTC timestamp>-<pid>.
DIR must not exist. The fresh target, Cargo HTML, build log and metadata are retained.
The Cargo registry is reused; this measures compilation, not an empty download cache.
EOF
}

profile=release
target=
output=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --profile | --target | --output)
            if [ "$#" -lt 2 ] || [ -z "$2" ]; then
                echo "$1 requires a value" >&2
                exit 2
            fi
            case "$1" in
                --profile) profile="$2" ;;
                --target) target="$2" ;;
                --output) output="$2" ;;
            esac
            shift 2
            ;;
        -h | --help) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
done

root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
output="${output:-target/build-timings/$(date -u +%Y%m%dT%H%M%SZ)-$$}"
mkdir -p "$(dirname "$output")"
# Refuse existing output: no previous build or ordinary target directory is removed.
mkdir "$output"
output="$(cd "$output" && pwd -P)"
mkdir "$output/target"

args=(build --locked --timings -p beamcraft-cli -p beamcraft-ui)
if [ "$profile" = release ]; then
    args+=(--release)
else
    args+=(--profile "$profile")
fi
if [ -n "$target" ]; then
    args+=(--target "$target")
fi

{
    printf 'started_utc: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'revision: %s\n' "$(git rev-parse HEAD)"
    printf 'working_tree:\n'
    git status --short
    uname -a
    if [ "$(uname -s)" = Darwin ]; then
        sysctl -n machdep.cpu.brand_string hw.ncpu
    elif [ -r /proc/cpuinfo ]; then
        awk -F ': ' '/^model name/ {print "cpu: " $2; exit}' /proc/cpuinfo
        if command -v nproc >/dev/null 2>&1; then
            printf 'available_cpus: %s\n' "$(nproc)"
        fi
        if [ -r /sys/fs/cgroup/cpu.max ]; then
            printf 'cgroup_cpu_quota: %s\n' "$(cat /sys/fs/cgroup/cpu.max)"
        fi
    fi
    printf 'build_jobs: %s\n' "${CARGO_BUILD_JOBS:-automatic}"
    rustc -vV
    cargo -V
    printf 'target_dir: %s\n' "$output/target"
    printf 'compiler_wrappers: disabled\n'
    printf 'registry_cache: inherited; helper performs no prefetch\n'
    printf 'cargo_net_offline: %s\n' "${CARGO_NET_OFFLINE:-false}"
    if [ -n "${RUSTFLAGS:-}${CARGO_ENCODED_RUSTFLAGS:-}" ]; then
        printf 'rustflags: inherited (set; clear for a default-profile comparison)\n'
    else
        printf 'rustflags: unset\n'
    fi
    printf 'command: cargo '
    printf '%q ' "${args[@]}"
    printf '\n'
} > "$output/metadata.txt"

started="$(date +%s)"
if CARGO_TARGET_DIR="$output/target" RUSTC_WRAPPER= RUSTC_WORKSPACE_WRAPPER= \
    cargo "${args[@]}" 2>&1 | tee "$output/build.log"; then
    status=0
else
    status=$?
fi
{
    printf 'elapsed_seconds: %s\n' "$(( $(date +%s) - started ))"
    printf 'exit_status: %s\n' "$status"
} >> "$output/metadata.txt"

if [ -f "$output/target/cargo-timings/cargo-timing.html" ]; then
    cp "$output/target/cargo-timings/cargo-timing.html" "$output/cargo-timing.html"
fi
printf 'Measurement retained in %s\n' "$output"
exit "$status"
