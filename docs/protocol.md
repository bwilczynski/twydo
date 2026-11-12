# The daemon protocol

`tdo daemon` serves the TUI and the CLI over a Unix socket. You can write your own client
against the same protocol. This page is the short version; the design has the detail
([§6](superpowers/specs/2026-11-09-daemon-design.md#6-the-protocol)), and the schema files below
are the reference. The protocol is generation 1 and not yet promised stable for outside
clients.

## The socket

`--socket`, else the `TWYDO_SOCKET` environment variable, else `$XDG_RUNTIME_DIR/twydo/tdo.sock`
(`$TMPDIR/twydo/tdo.sock` on macOS). The socket is `0600` in a `0700` directory. The daemon
refuses connections from other users, and a client should check the daemon's user the same way.
A client may start the daemon with `tdo daemon --detach`, or through the service unit
`tdo daemon install` writes ([install.md](install.md#start-it-at-login)).

Messages are JSON-RPC 2.0, one per line (UTF-8, `\n`-terminated), in both directions. Requests
may be pipelined; responses carry the request's `id`. Messages without an `id` from the daemon
are notifications.

To see the traffic, use [`tools/tdo-sniff.sh`](../tools/tdo-sniff.sh) (needs `socat` and
`jq`). It logs each message as one JSON line, `{"t", "conn", "from": "tdo"|"daemon", "msg"}`.
Run without arguments, it is a standalone proxy that writes the traffic to stdout until Ctrl-C:
start `tdo` (the TUI too) in another terminal with the `TWYDO_SOCKET` it prints, and filter with
jq, for example `tools/tdo-sniff.sh | jq -c '{from, m: .msg.method, id: .msg.id}'`.
`tools/tdo-sniff.sh status` runs one command and writes its traffic to stderr, or to a file with
`TDO_SNIFF_LOG=<file>`. To ask a running daemon directly, `daemon.info` answers before any
`hello`:

```sh
echo '{"jsonrpc":"2.0","id":1,"method":"daemon.info","params":{}}' \
  | socat - UNIX-CONNECT:"$XDG_RUNTIME_DIR/twydo/tdo.sock"
```

## `hello`

The first call. It names the client and the instance it means:

```json
{"jsonrpc":"2.0","id":1,"method":"hello","params":{"client":"my-client","version":"0.1.0",
 "protocol":1,"session":null,
 "instance":{"config":"/home/user/.config/twydo/config.toml","vdir":null}}}
```

The result has `daemon` (its version), `protocol`, `epoch`, `session` and `vdir`.

- `instance` is the absolute config file path (from `XDG_CONFIG_HOME` or `HOME`) and the
  `--vdir` override, if any. A daemon serving another configuration answers with error kind
  `wrong_instance`, so a client with the wrong socket setting never reads or writes the wrong
  data. Do not stop or restart a daemon that refused you this way.
- A `protocol` that does not match is refused with error kind `protocol`.
- `epoch` identifies the daemon process. If it changes after a reconnect, every revision number
  has restarted and every session is gone.
- `session` is the undo session. Pass the previous one when you reconnect to keep your undo
  stack.
- Before a successful `hello` only `hello`, `rpc.discover`, `daemon.info` and `daemon.shutdown`
  work.

## Data, writes and events

- `snapshot.get` returns the whole loaded vdir with a `revision`; `status.get` returns sync,
  conflict, journal and refusal state with a `status_rev`.
- `events.subscribe` starts notifications: `event` (with a rising `seq`: `data.changed`,
  `status.changed`, `op.finished`, `sync.started`, `sync.finished`, `job.finished`), `missed`
  when events were dropped (refetch from the revisions it names) and `heartbeat`. An event says
  that something changed; fetch the snapshot instead of trusting the event's hints.
- `op.submit` carries your `session`, a `request_id` and `based_on` (`epoch` and `revision` of
  the snapshot you planned against). The answer is `applied`, `queued` or `satisfied`; a queued
  write ends with an `op.finished` event. Resending the same `request_id` returns the original
  outcome and never writes twice, so a client may retry after a lost answer. `op.get` looks an
  outcome up by `op_id` or `request_id`. A refusal names a kind and a message to show as is.
- Read-only reports: `check.report` (`verbose`, optional `matrix` path, optional `uid`) returns
  `{lines, warnings}`, the exact stdout and stderr lines of `tdo check` or `tdo op check`.
  `export` returns `{skipped, document}`, the schema-v1 JSON of `tdo export-json`.
  `conflicts.list` and `conflicts.show {id}` return conflicts and the detail of one (fields and
  file paths of the local, remote and current copies). `status.get` also carries the journal
  entries still unfinished.
- Syncs and list operations are jobs: they answer at once with a `job_id` and end with a
  `job.finished` event; `job.get` returns the outcome later. There is no `job.progress` yet.
  `sync.stop` asks the running sync to stop; its result then reports the outcome `stopped`.

## List deletion result

`lists.preview_delete` finishes with a `delete_preview` result (a private backup id and its
counts). `lists.delete` finishes with `list_deleted` (`operation`, `collection`, `name`,
`backup`), as does `lists.reconcile` of an interrupted deletion. The deletion is
best-effort and the backup is kept (exportable with `lists.export`, not yet restorable).

## Schema files and open enums

- `schema/tdo.openrpc.json` in the repository describes every method, result, error and event,
  and is generated from the Rust types. `tdo daemon schema` prints the same
  document without a daemon, and a running daemon returns it for `rpc.discover`.
- `schema/released.openrpc.json` is the schema of the last release; a test rejects changes that
  would break a client built against it.
- Enums a client receives (outcomes, operation states, error kinds, the event `type`, causes) are
  open, marked `"x-open": true`. Read a value you do not know as `Unknown` and act
  conservatively: an unknown outcome or state is not known to be applied, an unknown error kind
  is a refusal to show as is, an unknown event is a reason to refetch. A newer daemon may add
  such values without a new protocol generation.
