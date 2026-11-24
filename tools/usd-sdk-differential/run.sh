#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# SPDX-FileCopyrightText: 2026 Apibim
#
# N03 differential harness: runs the committed synthetic layers through the independent
# Rust oracle and, where an official OpenUSD SDK is installed, through its tools too.
#
# The synthetic fixtures under fixtures/ are contributor-original; nothing here is derived
# from a real project export or a third-party model.
#
# Usage:
#   tools/usd-sdk-differential/run.sh [--no-sdk] [--digest]
#
# The official-SDK legs run only when `usdcat`/`usdchecker` (or USD_USDCAT/USD_USDCHECKER
# overrides) are found. When they are absent or --no-sdk is given, the harness prints an
# explicit SKIP banner: such a run verifies the oracle only and makes NO SDK claim.
# Exit status is 0 when every run check passed; skipped legs never count as passes.
# --digest replaces the check matrix with the LEW-05 digest export: one fixed-identity
# stack written by the real writer and hashed exactly as written, for a cross-platform
# SHA-256 comparison (one manual run per platform; digests never normalize bytes).

set -u

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
FIXTURES="$SCRIPT_DIR/fixtures"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ORACLE="$REPO_ROOT/target/debug/beamcraft-usd-oracle"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/usd-sdk-differential.XXXXXX")"

NO_SDK=0
DIGEST=0
for arg in "$@"; do
    case "$arg" in
        --no-sdk) NO_SDK=1 ;;
        --digest) DIGEST=1 ;;
        *) echo "usage: $0 [--no-sdk] [--digest]" >&2; exit 2 ;;
    esac
done

PASS=0
FAIL=0

record() { # record <PASS|FAIL> <check name>
    if [ "$1" = PASS ]; then
        PASS=$((PASS + 1)); echo "PASS $2"
    else
        FAIL=$((FAIL + 1)); echo "FAIL $2"
    fi
}

expect_exit() { # expect_exit <name> <want: 0|nonzero> <actual exit>
    local name="$1" want="$2" got="$3"
    if [ "$want" = 0 ]; then
        if [ "$got" -eq 0 ]; then record PASS "$name"; else record FAIL "$name (exit $got, expected 0)"; fi
    else
        if [ "$got" -ne 0 ]; then record PASS "$name"; else record FAIL "$name (exit 0, expected a nonzero refusal)"; fi
    fi
}

# ---- Tool discovery ---------------------------------------------------------------------------

CARGO="${CARGO:-cargo}"
command -v "$CARGO" > /dev/null 2>&1 || CARGO="$HOME/.cargo/bin/cargo"
SDK_LEG=1
USDCAT=""
USDCHECKER=""
if [ "$NO_SDK" -eq 1 ]; then
    SDK_LEG=0
else
    USDCAT="${USD_USDCAT:-$(command -v usdcat || true)}"
    USDCHECKER="${USD_USDCHECKER:-$(command -v usdchecker || true)}"
    if [ -z "$USDCAT" ] || [ -z "$USDCHECKER" ]; then
        SDK_LEG=0
        echo "SKIP official-SDK tools: usdcat/usdchecker not found on this machine"
    fi
fi
if [ "$SDK_LEG" -eq 1 ]; then
    echo "SDK: $("$USDCAT" --version 2>&1) [usdcat=$USDCAT usdchecker=$USDCHECKER]"
else
    echo "BANNER: the official-SDK legs are skipped; this run does NOT claim SDK verification"
fi

# Always rebuild: cargo is a no-op when the binary already matches the committed sources, so
# a leftover binary from an older commit can never be silently reused by a rerun.
echo "building the independent oracle (cargo build --locked -p beamcraft-cli --bin beamcraft-usd-oracle)"
"$CARGO" build --locked -p beamcraft-cli --bin beamcraft-usd-oracle >&2 || {
    echo "FAIL oracle build"; exit 1; }
echo "oracle: $ORACLE (repo $(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown))"
echo "work:   $WORK"
echo

# ---- Digest mode (LEW-05) -----------------------------------------------------------------------
# One fixed-identity stack written by the real writer on this platform, hashed exactly as
# written (the oracle's digest mode normalizes nothing). No reader checks run in this mode:
# two platforms compare digests only when both run these same committed commands. The run
# itself stays manual per the N03 owner decision of 2026-11-22; it is never a CI gate.

if [ "$DIGEST" -eq 1 ]; then
    BEAMCRAFT="$REPO_ROOT/target/debug/beamcraft"
    echo "building the writer CLI (cargo build --locked -p beamcraft-cli --bins)"
    "$CARGO" build --locked -p beamcraft-cli --bins >&2 || { echo "FAIL cli build"; exit 1; }
    STORE="$WORK/store"
    mkdir -p "$STORE"
    "$BEAMCRAFT" create-plate --store "$STORE" \
        --new-id 0199e7a0-0000-7000-8000-00000000d001 \
        --op-id 0199e7a0-0000-7000-8000-00000000d002 \
        --revision-id 0199e7a0-0000-7000-8000-00000000d003 \
        --actor-id 0199e7a0-0000-7000-8000-00000000d004 \
        --project-id 0199e7a0-0000-7000-8000-00000000d005 \
        --workspace-id 0199e7a0-0000-7000-8000-00000000d006 \
        --width 1000 --length 500 --thickness 10 --name "Digest Reference" \
        --translation "2000,-3000,1000" \
        --orientation-xyzw "0,0,0.7071067811865476,0.7071067811865476" \
        --hole-round "250,250,60" > /dev/null 2>"$WORK/create.err" || {
        echo "FAIL the fixed-identity stack was refused:"; head -3 "$WORK/create.err"; exit 1
    }
    echo "platform: $(uname -s -r -m)  cli: $("$BEAMCRAFT" --version 2>&1)"
    echo
    MANIFEST=$(grep -rl '^beamcraft-revision-manifest' "$STORE/blobs" | head -1)
    if [ -z "$MANIFEST" ]; then
        echo "FAIL no revision manifest blob in the store"; exit 1
    fi
    status=0
    while read -r name digest size; do
        blob="$STORE/blobs/${digest%"${digest#??}"}/$digest"
        line=$("$ORACLE" digest "$blob")
        if [ "${line%% *}" = "$digest" ]; then
            printf 'sha256  %s  %s  %s bytes\n' "$digest" "$name" "$size"
        else
            echo "FAIL layer $name ($digest) hashes to ${line%% *} (the store content addressing broke)"
            status=1
        fi
    done < <(grep '^layer ' "$MANIFEST" | sort -k3 | awk '{print $3, $4, $5}')
    rm -rf "$WORK"
    if [ "$status" -ne 0 ]; then
        exit 1
    fi
    echo
    echo "OK: every manifest layer matches its content digest; compare this table across platforms"
    exit 0
fi

# ---- Positive fixtures ------------------------------------------------------------------------

check_flatten() { # check_flatten <name> <flatten output file> <assert list file>
    local name="$1" flat="$2" asserts="$3" line
    while IFS= read -r line; do
        [ -z "$line" ] && continue
        case "$line" in
            '!'*)
                if grep -Fq "${line#!}" "$flat"; then
                    record FAIL "$name: flatten must not contain ${line#!}"
                else
                    record PASS "$name: flatten drops ${line#!}"
                fi
                ;;
            *)
                if grep -Fq "$line" "$flat"; then
                    record PASS "$name: flatten contains $line"
                else
                    record FAIL "$name: flatten lacks $line"
                fi
                ;;
        esac
    done < "$asserts"
}

run_positive() { # run_positive <fixture dir> <oracle mode>
    local dir="$1" mode="$2" name
    name="oracle/$mode/$(basename "$dir")"
    local out
    out="$("$ORACLE" "$mode" "$dir/root.usda" "$dir/expectation.json" 2>&1)"
    expect_exit "$name" 0 "$?"
    [ "$out" != "${out%%FAIL*}" ] && echo "$out" | grep '^FAIL' | while IFS= read -r line; do
        echo "     $line"
    done

    [ "$SDK_LEG" -eq 1 ] || return 0
    local flat="$WORK/$(basename "$dir")-flat.usda"
    "$USDCAT" --flatten "$dir/root.usda" > "$flat" 2>"$WORK/flatten.err"
    expect_exit "sdk/usdcat-flatten/$(basename "$dir")" 0 "$?"
    check_flatten "sdk" "$flat" "$dir/flatten-must.txt"

    "$USDCHECKER" "$dir/root.usda" > "$WORK/checker.out" 2>&1
    expect_exit "sdk/usdchecker/$(basename "$dir")" 0 "$?"

    # The official SDK round-trips the canonical text through its binary USDC form.
    "$USDCAT" "$dir/root.usda" --out "$WORK/roundtrip.usdc" > /dev/null 2>&1
    expect_exit "sdk/usdc-write/$(basename "$dir")" 0 "$?"
    "$USDCAT" "$WORK/roundtrip.usdc" > /dev/null 2>&1
    expect_exit "sdk/usdc-read/$(basename "$dir")" 0 "$?"
}

run_positive "$FIXTURES/positive-revision" revision
run_positive "$FIXTURES/positive-reference" reference

# ---- Negative fixtures: refusal matrix ---------------------------------------------------------
# root.usda of the positive revision is replaced by a committed variant. Each row states what
# both readers must do; a mismatch between the recorded expectation and either reader fails.

refuse() { # refuse <variant name> <sdk: ok|fail>
    local variant="$1" sdk_want="$2" dir="$WORK/neg-$1"
    cp -R "$FIXTURES/positive-revision" "$dir"
    cp "$FIXTURES/negative/$1-root.usda" "$dir/root.usda"
    "$ORACLE" revision "$dir/root.usda" "$FIXTURES/positive-revision/expectation.json" > /dev/null 2>&1
    expect_exit "oracle/refuse/$1" nonzero "$?"

    [ "$SDK_LEG" -eq 1 ] || return 0
    "$USDCAT" "$dir/root.usda" > /dev/null 2>&1
    local got=$?
    if [ "$sdk_want" = ok ]; then
        # The official reader is more permissive here; the delta is a documented
        # structural-strictness choice of our profile, not a defect.
        if [ "$got" -eq 0 ]; then record PASS "sdk/accept/$1 (official reader more permissive, as documented)"; else record FAIL "sdk/accept/$1: official reader refused (exit $got), expected acceptance"; fi
    else
        if [ "$got" -ne 0 ]; then record PASS "sdk/refuse/$1 observed"; else record FAIL "sdk/refuse/$1: official reader accepted, expected refusal"; fi
    fi
}

refuse bad-header refuse
refuse unknown-metadata ok
refuse unexpected-op ok

# ---- Summary -----------------------------------------------------------------------------------

echo
echo "checks: $PASS passed, $FAIL failed"
rm -rf "$WORK"
if [ "$FAIL" -ne 0 ]; then
    exit 1
fi
echo "OK: the independent oracle checks passed$([ "$SDK_LEG" -eq 1 ] && echo ' and the official-SDK legs agreed' || echo '; the official-SDK legs were skipped')"
