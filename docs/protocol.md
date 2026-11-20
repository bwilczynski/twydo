# The daemon protocol

`tdo daemon` serves the TUI and the CLI over a Unix socket. You can write your own client
against the same protocol. This page is the short version; the design has the detail
([§6](superpowers/specs/2026-11-09-daemon-design.md#6-the-protocol)), and the schema files below
are the reference. The protocol is generation 3 and not yet promised stable for outside
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

- `views.get` takes `{view, list?}` and returns a finished view for thin clients
  (a phone, a display) that hold no view logic: dated sections with labels (a `completed`
  section also carries its `date`, since a view can have several), rows with relative due text,
  overdue and pending flags, and the `capture` target for a new task. Dates and
  groups follow the daemon's clock in the configured timezone (`now`, `today`). A client that
  writes from a view sends `based_on` built from the result's `epoch` and `revision`. The result
  also carries `sync` (absent without `[sync]`): `attention`, a `headline`, an optional `detail`
  (at most 300 characters) and `last_success`. `attention` is true when sync needs a person: a
  failing sync, failed list discovery, a list emptied and not applied (with the
  `tdo sync --force-delete` command), conflicts or automatic sync off. It informs only.
  `view` is `inbox`, `today`, `all`, `scheduled`, `done`, `next_3_days` or `list`; the set is
  open, so a client treats an unknown value it receives as unknown. `list` names one list by
  its collection id in `list`, which is required for `list` and refused (error kind `invalid`)
  for every other view. A list id that does not exist, an unknown `view` and a missing `list`
  are refused the same way. `inbox` and `today` return exactly what they always did. The `title`
  of any other view is its sidebar name, and `capture` is the list that view's new tasks go to
  (the named list for `list`, the default list for the others; `today` also sets its due date).
- `views.list` takes no parameters and returns the sidebar for a view picker: `epoch`,
  `revision`, `now`, `today` and `entries`, in display order. An entry is a heading
  (`{heading}`, such as a configured group or "Lists") or a view `{view, name, color,
  open_count, due_count}`. `view` is the `views.get` params that open it, so a client passes it
  back unchanged. The focus lists come first (the Inbox list as `{view: "inbox"}`, then
  `today`, `all`, `scheduled`, `done`), then the other visible lists as `{view: "list", list}`
  under their headings, then the "Smart lists" heading with `next_3_days`. Hidden and archived
  lists are left out. `color` is `#rrggbb` for a list (the Inbox entry carries its list's
  colour) and null for Today, All, Scheduled, Done and Next 3 Days.
  `open_count` and `due_count` are the counts the TUI sidebar shows: the open tasks, and those
  due today or overdue.
- `tasks.get` takes `{uid}` and returns one task worded for a thin client's task screen, with
  `epoch`, `revision`, `now` and `today` as `views.get` has. `task` carries the title, the shown
  `kind` (a plain task with items is a `project`), its `list`, relative `due` and `start` (the
  start whenever set), `overdue`, `due_today`, the `repeat` rule in words, one line per alert in
  `alerts`, load `warnings`, the `blocked` reason edits are off, `conflict`, `pending`,
  `completed`, the `note` (none when blank) and the `parent` (`{uid, title, kind}`). `children`
  lists every child: the open ones in sibling order, then the completed ones, newest first;
  each has `uid`, `title`, `kind`, `due`, `overdue`, `due_today`, `completed`, `pending`,
  `has_note` and `child_count`. The wording is the TUI details pane's. `form` is the edit form's
  starting state, as the TUI card would show it: `fields` in display order (a checklist item has
  only `title` and `notes`), `values` (the typed texts, `kind` and `list`), `previews` (the
  `{field, text}` lines the card shows, such as `→ Wed 18 Nov` or `… (kept)` for a repeat rule
  Twydo cannot write) and `alert_editable`, false when the task has alerts Twydo cannot write.
  An unknown `uid` is
  refused with error kind `invalid`. A client refetches on `data.changed` and `status.changed`.
- `tasks.form` checks a phone's edit or new-task form with the TUI's own form code. It takes
  `uid` (an edit) or `new` (`{collection, parent?, due?}`; only `due` equal to today starts the
  form with Due `today`), exactly one of them, else `invalid` (`give uid or new`; an unknown
  `uid` or `new.parent` is `invalid` `no such task`). `values` are the typed texts (none asks
  for the starting values) and, for an edit, `initial` is the values the form was opened with.
  It returns `epoch` and `revision` (the basis to submit `op` with), `form` (the same
  `{fields, values, previews, alert_editable}` as `tasks.get`), `op` (the write, when every
  field reads and something would change) or `error` (`no changes`, `title must not be empty`,
  `due: ...`), `changed_since` and `rebased`. When `initial` differs from the task's current
  values the typed fields are rebased onto the saved version, as the TUI does: `changed_since`
  is true, `form` is the rebased form, and `rebased` gives the new `initial` to send from now
  on, `overwritten` (typed fields the saved version changed too, whose typing wins) and
  `dropped` (typed fields the saved version's form cannot take). It reads one published
  snapshot with queued writes laid over it, never writes and keeps no state.
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
- `notify.test` (no parameters) sends one fixed notification through the sink the daemon uses
  (the configured `[notifications] command`, else the platform command), even when reminders are
  disabled, and records nothing. It returns `{}`, or an `invalid` refusal whose message is the
  reason (the daemon has no sink, the command failed or timed out). `status.get` carries
  `notifier_error` while a reminder cannot be delivered or its record cannot be saved: a text
  to show as is, never naming a task, and absent when all is well.
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
