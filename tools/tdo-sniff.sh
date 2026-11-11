#!/usr/bin/env bash
# Watch the protocol traffic between tdo and its daemon through a logging proxy
# (docs/protocol.md). Each message is logged as one JSON line, so the log pipes into jq:
#
#   {"t":"14:02:11.503","conn":4711,"from":"tdo","msg":{"jsonrpc":"2.0","id":1,"method":"hello",...}}
#
# `from` is "tdo" or "daemon"; `conn` tells connections apart (one per tdo process).
#
# Two ways to run it:
#
#   tools/tdo-sniff.sh [| jq ...]
#       A standalone proxy: it writes the traffic to stdout until Ctrl-C. In another terminal,
#       run tdo (the TUI or any command) with the TWYDO_SOCKET it prints on stderr. Start the
#       proxy first and quit those tdo processes before stopping it: a tdo that finds no
#       daemon on the proxy socket would try to start one there (it exits again, since the
#       real daemon holds the vdir). TDO_SNIFF_SOCKET sets the proxy's socket (default:
#       $XDG_RUNTIME_DIR/twydo-sniff/tdo.sock).
#
#   tools/tdo-sniff.sh <tdo arguments>      e.g. tools/tdo-sniff.sh status
#       Runs that one command through a private proxy and writes its traffic to stderr (stdout
#       stays the command's). The command's own stderr mixes in, so for jq log to a file:
#       TDO_SNIFF_LOG=traffic.jsonl tools/tdo-sniff.sh status; jq . traffic.jsonl
#
# TDO_SNIFF_LOG=<file> appends the traffic to a file instead. The real socket is the one tdo
# would use (TWYDO_SOCKET, else $XDG_RUNTIME_DIR/twydo/tdo.sock), so export TWYDO_SOCKET and
# XDG_CONFIG_HOME first to watch a test setup. TDO names the tdo binary (default: tdo on PATH).
# Needs socat and jq.
set -euo pipefail

# One connection, run by socat for each: copies the bytes both ways unchanged and logs every
# line (one protocol message) as a JSON object on fd 3. A line that is not JSON is logged as a
# string, so the logger never dies and never breaks the connection.
if [ "${1:-}" = __relay ]; then
    log() {
        jq -R -c --unbuffered --arg from "$1" --argjson conn "$$" '
            now as $n
            | {t: (($n | strflocaltime("%H:%M:%S")) + "."
                   + ("00" + (($n * 1000 | floor) % 1000 | tostring))[-3:]),
               conn: $conn, from: $from, msg: (try fromjson catch .)}' >&3
    }
    tee >(log tdo) | socat - UNIX-CONNECT:"$TDO_SNIFF_REAL" | tee >(log daemon)
    exit
fi

tdo=${TDO:-tdo}
runtime=${XDG_RUNTIME_DIR:-}
export TDO_SNIFF_REAL=${TWYDO_SOCKET:-${runtime:?set XDG_RUNTIME_DIR or TWYDO_SOCKET}/twydo/tdo.sock}
TDO_SNIFF_SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
export TDO_SNIFF_SELF

for cmd in socat jq; do
    command -v "$cmd" >/dev/null || { echo "tdo-sniff: $cmd is not installed" >&2; exit 2; }
done

# The daemon must run on the real socket: the proxy accepts connections, so tdo would never
# start a daemon behind it.
"$tdo" daemon ping >/dev/null

# socat's arguments to listen on $1 (in a 0700 directory: tdo uses no other) and relay each
# connection.
listen() {
    args=(UNIX-LISTEN:"$1",fork,mode=600,unlink-early SYSTEM:'exec "$TDO_SNIFF_SELF" __relay')
}

if [ $# -eq 0 ]; then
    if [ -n "${TDO_SNIFF_LOG:-}" ]; then exec 3>>"$TDO_SNIFF_LOG"; else exec 3>&1; fi
    sock=${TDO_SNIFF_SOCKET:-${runtime:?set XDG_RUNTIME_DIR or TDO_SNIFF_SOCKET}/twydo-sniff/tdo.sock}
    mkdir -p "$(dirname "$sock")"
    chmod 700 "$(dirname "$sock")"
    trap 'rm -f "$sock"' EXIT
    echo "tdo-sniff: proxying $TDO_SNIFF_REAL; run tdo elsewhere with:" >&2
    echo "  TWYDO_SOCKET=$sock tdo" >&2
    listen "$sock"
    socat "${args[@]}"
    exit
fi

if [ -n "${TDO_SNIFF_LOG:-}" ]; then exec 3>>"$TDO_SNIFF_LOG"; else exec 3>&2; fi
dir=$(mktemp -d)
chmod 700 "$dir"
sock=$dir/tdo.sock
listen "$sock"
# socat itself in the background (no subshell around it), so the trap ends it and with it
# every reader of the log.
socat "${args[@]}" &
socat_pid=$!
# SIGKILL: socat reports a SIGTERM on stderr, which would land in the log.
trap 'kill -KILL "$socat_pid" 2>/dev/null; rm -rf "$dir"' EXIT

for _ in $(seq 50); do
    [ -S "$sock" ] && break
    sleep 0.05
done
[ -S "$sock" ] || { echo "tdo-sniff: the proxy socket did not appear" >&2; exit 2; }

# fd 3 is the log; the command does not need it.
TWYDO_SOCKET=$sock "$tdo" "$@" 3>&-
