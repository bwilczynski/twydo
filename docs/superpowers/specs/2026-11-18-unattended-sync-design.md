# Unattended sync: server-emptied lists and sync logging

Issues: #142 (unattended mode), #141 (daemon does not log sync failures), plus sync state on
the phone. Size: a small change in three parts.

## Why

The daemon in the gateway container runs with nobody at a terminal. Two things break that:

- A list emptied on another device (Inbox cleared on the iPhone) stops syncing until someone
  runs `tdo sync --force-delete "<list>"` inside the container. 2Do on the iPhone applies the
  same change without asking; Twydo should too.
- A failing sync is recorded only in `sync/status.json`. `docker compose logs`, log collectors
  and `daemon-<socket-id>.log` show nothing after `serving ...`.

## Goals

- A list emptied on the server is applied locally by any sync (background or manual, desktop
  or container) when doing so cannot lose anything that exists only here.
- The daemon prints one stderr line per sync state change, in the `tdo daemon: ...` style.
- The phone shows when sync needs a person, and what to run.

## Non-goals

- Backup and restore. Issue #23 is the real safety net for wrongly applied deletions; it is
  separate work.
- Changing how lists emptied **here** are handled (the existing auto-force stays as it is).
- Logging every retry, or logging from the foreground `tdo sync` (it prints its own messages).

## Anti-goals

- Deleting a local edit or a task that was never uploaded.
- Wiping every list because the server answered "empty" for all of them.
- Task titles in logs.

## Approaches (#142)

- **A. Guarded automatic apply (chosen).** Force an emptied-on-server list only when the local
  side provably holds nothing the server lacks. Mirrors the existing rule for lists Twydo
  emptied itself. Cost: a list emptied on the server while it holds an unsynced edit still
  needs the manual command, the one case where forcing could lose that edit.
- **B. Always force.** Simplest, closest to 2Do, but relies on unverified vdirsyncer behaviour
  for a task edited here and deleted there, and a server wrongly answering "empty" would wipe
  every list with no backup yet.

The user chose A, enabled everywhere (no setting).

## Design (#142)

In `crates/twydo-core/src/sync/run.rs`, `auto_force` also selects collections in
`CollState::EmptiedRemote` when all hold:

1. **No local-only files.** Every `.ics` file now in the local collection directory is listed
   for it in the manifest read at the start of the run (the last sync in which it counted).
   A missing manifest entry means no evidence: not forced.
2. **No queued edit for it.** No pending journal entry writes a path in that collection.
3. **No unresolved conflict** in that collection.
4. **Not a server-wide emptying.** If at least two collections took part in the run and every
   one of them came back `EmptiedRemote`, none is forced. (My call: a single-list setup has no
   such signal, so the guard needs two or more lists.)

Forced collections rerun with `--force-delete` in the same second pass as the local case,
under the same lock, and land in `report.auto_forced`. A list failing any check keeps today's
message and `tdo sync --force-delete` / `:sync! <list>` behaviour.

Reporting: an auto-forced server emptying prints
`applied the list "<name>" emptied on the server` in a foreground run (next to the existing
`applied the emptied list "<name>" on the server`); the report needs to know which side each
auto-forced list was emptied on.

## Design (#141)

The daemon logs after each sync in `crates/twydo-daemon/src/server/syncjob.rs`, comparing the
run's result and `status.json` with what it logged last (kept in memory; a restart logs the
first result again). Lines, printed only on change:

- `tdo daemon: sync ok (N lists)`: the first success after start, and recovery as
  `tdo daemon: sync ok again after N failures`.
- `tdo daemon: sync failed (attempt N, next in Xs): <last_error>`: the first failure and
  whenever the error text changes; identical retries stay quiet.
- `tdo daemon: list discovery failed: <discovery_error>`: when it appears or changes.
- `tdo daemon: applied list <id> emptied on the server` (and `... emptied here ... on the
  server` for the local case): every auto-forced list, every time.
- `tdo daemon: list <id> emptied on the server; not applied (<reason>)`: when a refused
  emptying first appears, so an unattended setup can see why it is stuck.

Lists are named by collection id, never by display name or task title (my call: display names
are list names, which the gateway rule also keeps out of logs).

## Design: sync state on the phone

The phone shows nothing about sync today, so a stuck container goes unnoticed. Inform only:
the phone says what is wrong and what to run; it gets no new actions. Actions ("Sync now",
"Apply emptying") would open `sync.run` with `force_delete` to a paired device and need their
own design; with #142 the forced case should be rare.

- **Where the text is made.** The daemon adds an optional `sync` summary to the `views.get`
  result (`crates/twydo-daemon/src/protocol/views.rs`), built from the same status the TUI
  status line reads. The phone stays a thin client and `status.get` (journal entries, conflict
  file paths) stays closed to it. No new event or gateway rule: the phone already refetches
  `views.get` on `status.changed`, which the gateway forwards.
- **Shape (my call).** `sync: { attention: bool, headline: string, detail: string?,
  last_success: datetime? }`, absent when sync is not configured. `attention` is true when
  something needs a person:
  - the last sync failed (`last_error`), with the retry time: `Sync failing since 14:02`,
    detail `Cannot reach the server ... Next try 14:10.`;
  - list discovery failed;
  - a list emptied on one side and not applied (a refused #142 case), naming the list and the
    command: `Run: tdo sync --force-delete "Inbox"`;
  - unresolved conflicts: `2 conflicts; resolve them in tdo`;
  - automatic sync switched off.
  Otherwise `attention` is false and `headline` is `Synced 5m ago` or `Not synced yet`.
  Wording follows the TUI status line minus its `:command` hints. List names are shown: the
  phone already shows the user's own task titles.
- **Web app.** A persistent warning strip under the view title while `attention` is true,
  headline plus detail, not dismissable (it clears when the state does). When all is well,
  nothing is shown (my call: a constant "Synced 5m ago" adds noise to a capture screen).
- Display-role devices receive the same field.

## Docs

Update `docs/sync.md` (Emptied lists), `docs/gateway.md` (logs, sync warning on the phone) and
`docs/protocol.md` (`views.get` `sync` field), and regenerate the protocol schema files.

## Left to the builder

Exact reason wording for refused emptyings, where the "last logged" state lives in `Shared`,
and test layout (core tests for each guard in `auto_force`; a daemon test for log lines on
change only).
