#!/usr/bin/env bash
# Run the daemon and the gateway in one container. If either exits, stop and exit with its
# status so the restart policy restarts both. Other arguments run as a command instead.
set -u

if [ "$#" -gt 0 ]; then
    exec "$@"
fi

daemon_pid=
gateway_pid=
trap 'kill $daemon_pid $gateway_pid 2>/dev/null' TERM INT

# A socket left by a previous run (/run/twydo survives a restart) would pass -S yet refuse
# connections, and the ping below would start a second daemon. Nothing else serves it here.
rm -f "${TWYDO_SOCKET:-}"

tdo daemon &
daemon_pid=$!

# Wait for the socket before pinging: a ping starts a daemon if none answers, which would
# race a second one into existence.
ready=0
for _ in $(seq 1 30); do
    if [ -S "${TWYDO_SOCKET:-}" ]; then
        ready=1
        break
    fi
    if ! kill -0 "$daemon_pid" 2>/dev/null; then
        wait "$daemon_pid"
        exit $?
    fi
    sleep 1
done
if [ "$ready" -ne 1 ] || ! tdo daemon ping >/dev/null 2>&1; then
    echo "entrypoint: the daemon did not answer within 30 s" >&2
    kill "$daemon_pid" 2>/dev/null
    exit 1
fi

tdo gateway serve &
gateway_pid=$!

wait -n
status=$?
kill "$daemon_pid" "$gateway_pid" 2>/dev/null
wait 2>/dev/null
exit "$status"
