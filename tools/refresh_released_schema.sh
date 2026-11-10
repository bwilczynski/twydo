#!/usr/bin/env bash
# Copy the current protocol schema over the released one. Run on the release PR branch so the
# breaking-change check compares the next release against this release (daemon spec §6.3).
set -euo pipefail
cd "$(dirname "$0")/.."
cp schema/tdo.openrpc.json schema/released.openrpc.json
