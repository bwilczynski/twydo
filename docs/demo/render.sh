#!/usr/bin/env bash
# Re-renders the README demo from docs/demo/tui.tape against a generated demo vdir.
# Run from anywhere: `mise run demo`. Needs vhs, ttyd and ffmpeg (mise installs vhs and ttyd,
# except on macOS where ttyd has no release: `brew install ttyd`).
#
# The demo data is the synthetic sample vdir with its dates moved to today
# (tools/demo_vdir.py). Config and state live in a temporary directory, so the render never
# reads or writes your own Twydo config, vdir or journal.
set -euo pipefail
cd "$(dirname "$0")/../.."

command -v ffmpeg >/dev/null || { echo "render.sh: vhs needs ffmpeg on PATH" >&2; exit 1; }
cargo build --release --quiet

demo_dir=$(mktemp -d)
trap 'rm -rf "$demo_dir"' EXIT
python3 tools/demo_vdir.py "$demo_dir/vdir"
mkdir -p "$demo_dir/config/twydo"
cat > "$demo_dir/config/twydo/config.toml" <<TOML
vdir = "$demo_dir/vdir"
timezone = "America/New_York"
TOML

# The tape points XDG_CONFIG_HOME and XDG_STATE_HOME at the demo directory inside the recorded
# shell only: changing them here would also hide mise's own state from the vhs and ttyd shims.
export TWYDO_DEMO_DIR="$demo_dir" TZ=America/New_York PATH="$PWD/target/release:$PATH"

if [[ "${1:-}" == "--shell" ]]; then
  # Debugging aid: a shell with the demo environment, to try the tape's keys by hand.
  XDG_CONFIG_HOME="$demo_dir/config" XDG_STATE_HOME="$demo_dir/state" "${SHELL:-bash}"
else
  vhs "${DEMO_TAPE:-docs/demo/tui.tape}"
fi
