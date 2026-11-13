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

# The demo runs in New York time; generate its dates for today there too.
export TZ=America/New_York
demo_dir=$(mktemp -d)
# The demo daemon listens on its own socket, never your own daemon's; stop it before cleaning up.
export TWYDO_SOCKET="$demo_dir/tdo.sock"
cleanup() {
  # The daemon answers only a client with its own configuration.
  XDG_CONFIG_HOME="$demo_dir/config" XDG_STATE_HOME="$demo_dir/state" \
    target/release/tdo daemon stop >/dev/null 2>&1 || true
  rm -rf "$demo_dir"
}
trap cleanup EXIT
python3 tools/demo_vdir.py "$demo_dir/vdir" --today "$(date +%F)"
mkdir -p "$demo_dir/config/twydo"
# A stand-in for vdirsyncer: there is no server, so a sync waits briefly and succeeds. The
# demo shows the TUI's sync states, not a real exchange.
cat > "$demo_dir/vdirsyncer" <<SH
#!/bin/sh
[ "\$1" = __probe ] && exit 0
sleep 1.5
# Report each list as synced, as vdirsyncer does, so the demo's edits count as sent.
[ "\$1" = sync ] && for d in "$demo_dir"/vdir/*/; do echo "Syncing twydo/\$(basename "\$d")" >&2; done
exit 0
SH
chmod +x "$demo_dir/vdirsyncer"
# Automatic sync is pushed far out, so only the tape's :sync shows a sync (the daemon's
# startup sync runs in the tape's hidden prologue).
cat > "$demo_dir/config/twydo/config.toml" <<TOML
vdir = "$demo_dir/vdir"
timezone = "America/New_York"

[daemon]
unit_warning = false

[sync]
url = "https://dav.example.test/demo/"
username = "demo"
password_command = ["echo", "demo"]
command = "$demo_dir/vdirsyncer"
metasync = false
# No list helper: discovery cannot start, so it never reaches for the placeholder server.
list_python = "$demo_dir/no-list-helper"
batch_size = 1000
max_wait_secs = 3600
TOML

# The tape points XDG_CONFIG_HOME and XDG_STATE_HOME at the demo directory inside the recorded
# shell only: changing them here would also hide mise's own state from the vhs and ttyd shims.
export TWYDO_DEMO_DIR="$demo_dir" PATH="$PWD/target/release:$PATH"

if [[ "${1:-}" == "--shell" ]]; then
  # Debugging aid: a shell with the demo environment, to try the tape's keys by hand.
  XDG_CONFIG_HOME="$demo_dir/config" XDG_STATE_HOME="$demo_dir/state" VISUAL='nvim --clean' \
    "${SHELL:-bash}"
else
  vhs "${DEMO_TAPE:-docs/demo/tui.tape}"
fi
