#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# See assets/icons/README.md for renderer versions and native acceptance checks.
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$root/packaging/generate-icon.py"
