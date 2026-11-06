# Sync

Twydo does not talk to the network. It runs [vdirsyncer](https://vdirsyncer.pimutils.org/) against
the vdir, records what it changed in the operation journal ([architecture.md](architecture.md#the-operation-journal)), and keeps
both versions of anything that was changed on two devices until you choose. Configuration keys are
in [config.md](config.md) (`[sync]`).

## Setup

1. **vdirsyncer config.** Start from `contrib/vdirsyncer/config.example`. The line that matters is

   ```
   conflict_resolution = ["command", "tdo", "conflicts", "hook"]
   ```

   It is not `a wins` / `b wins`. The hook keeps both versions in the conflict store and changes
   nothing until you choose. `tdo` must be on the `PATH` of whatever runs vdirsyncer (the timer,
   the agent, your shell).

2. **Twydo config.** Add a `[sync]` table to the config. An empty table turns sync on with the
   defaults:

   ```toml
   [sync]
   # pair = "twydo"
   # local_storage = "twydo_local"
   # vdirsyncer_config = "~/.config/vdirsyncer/config"
   ```

   `local_storage` must be the name of the pair's vdir-side storage in the vdirsyncer config. The
   hook tells the local file from the remote one by that name.

   If `tdo` runs with a different `XDG_CONFIG_HOME` than vdirsyncer expects (for example a separate
   test setup), vdirsyncer cannot find its own config: set `vdirsyncer_config`, which Twydo passes
   to vdirsyncer as `VDIRSYNCER_CONFIG`.

3. **Check by hand.** `tdo sync` should print `synced`; `tdo status` shows the result.

4. **Background sync.** Pick one:

   - Linux, systemd user units:

     ```
     cp contrib/systemd/twydo-sync.service contrib/systemd/twydo-sync.timer ~/.config/systemd/user/
     systemctl --user daemon-reload
     systemctl --user enable --now twydo-sync.timer
     ```

     Adjust `ExecStart` to where `tdo` lives. For a separate test setup, uncomment the
     `Environment=` line. A run that skips (backoff, or another runner busy) exits 0. The unit sets
     `SuccessExitStatus=1`: `tdo sync` exits 1 while a conflict or an emptied list waits for you,
     or after a failure, and the unit would otherwise show failed every minute; check
     `tdo status` instead. Exit 2 (configuration or IO error) still marks the unit failed.

   - macOS, launchd agent: copy `contrib/launchd/app.twydo.sync.plist` to
     `~/Library/LaunchAgents/`, replace `/usr/local/bin/tdo` and `/Users/USER` with real paths
     (launchd does not expand `~`), then
     `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/app.twydo.sync.plist`.

   Both run `tdo sync --background` every minute. The timer also pulls changes made on other
   devices; edits made here sync without it too ([When edits sync](#when-edits-sync)).

## When edits sync

Edits sync in batches. An edit is **unsynced** while its journal entry is applied, not yet
confirmed by a sync, and made after the last sync attempt began; entries an earlier sync saw
and left pending (an open conflict, a list that did not count) wait for you or the next sync
instead. A sync is **due** once `batch_size` edits (default 5) are unsynced and edits have
paused for 3 seconds, or `max_wait_secs` (default 60) after the oldest of them
([config.md](config.md)), whichever comes first, or later while the runner backs off. The
pause keeps a run of quick edits (closing checklist items one by one) from being interrupted by
its own sync. A save made while a sync is running still waits for it
([tui.md](tui.md#when-a-write-is-refused)); the write queue will remove that later.

After every write (the TUI, `tdo op`, a `tdo conflicts resolve` that writes the vdir) the
writer checks that rule:

- **Due now** (edits have paused, or the oldest is overdue): it starts a detached
  `tdo sync --background` at once.
- **Otherwise:** it starts the **batch waiter**, a detached `tdo sync --background --when-due`.
  The waiter takes `sync/waiter.lock` without waiting; if another waiter holds it, it exits at
  once, so there is one per vdir. While waiting, it rechecks once per second: more edits can
  fill the batch and bring the deadline forward, or push the quiet period out; the timer or
  `:sync` may have sent everything meanwhile. Deadlines use wall clock, so an overdue sync
  starts after the laptop wakes. It syncs when due and loops while edits keep arriving,
  exiting when nothing is unsynced or automatic sync is off.

Background syncs and the batch waiter run in their own process group, so closing the terminal
or pressing Ctrl-C does not stop them. A waiter that fails (it cannot read the journal, say)
exits silently: the timer or the next edit catches up. Quitting the TUI starts no sync: a waiter
started by the last edit outlives it and sends that edit within `max_wait_secs`.

A conflict choice recorded for a held conflict is not an edit and only applies on a sync, so
`resolve` starts one at once. `:sync` and `tdo sync` sync now, whatever the count.

**Turning automatic sync off.** `tdo sync off` (or `:sync off` in the TUI) stores
`sync/off`, so it lasts across restarts and every process sees it. While off, every automatic
sync skips quietly and records no failure: the timer, the TUI's startup sync, the batch waiter
and the batch-full sync. Edits stay local and pending; the TUI's status line shows `sync off`
with the pending count, and `tdo status` prints `automatic sync: off`. `:sync` and `tdo sync`
still run once. `tdo sync on` (`:sync on`) removes the file and, with unsynced edits, starts a
sync at once.

**The "sync ended" event.** Every run that is not skipped ends, clean, failed or stopped, by
atomically writing `sync/ended` (a run id and the time). The TUI checks it and reloads when it
changes, so it shows what any sync brought in (startup, waiter, timer, systemd or `tdo sync`
in another terminal), even when no file change was reported.

## What `tdo sync` does

`tdo sync` exits 0 when every list is fine, 1 when the run had conflicts, emptied lists or a
failure, and 2 on a configuration or IO error. Problems are printed to stderr as `tdo: ...`.
`--background` prints nothing on success, does not wait for another runner, and honours backoff.

One run, in order:

1. **Run lock.** At most one runner per vdir (`run.lock` in the state directory). A foreground run
   waits for the lock. A background run that finds it taken leaves a request (`requested`) and
   exits; the running runner then does one more pass, at most twice.
2. **Backoff.** A background run does nothing while the status says `next_attempt` is in the
   future.
3. **Recovery.** Under the exclusive vdir lock (taken on every run, so the run first waits for a
   vdirsyncer left over from a killed run), unfinished journal entries are rolled forward before
   vdirsyncer runs. For an entry that cannot be finished, every write whose file does not hold its
   `after` (the file holds its `before`, or a third version) becomes a `journal` conflict and the
   entry is abandoned (see Known limits). When nothing is stuck, edits waiting in the write
   queue are applied here, under the same lock, oldest first.
4. **vdirsyncer** `sync <pair>` runs under the shared vdir lock, so readers (the TUI, `tdo op`
   reads) continue and writers wait. vdirsyncer holds a shared lock of its own too (its stdin is
   the lock file), which lasts until it exits even if `tdo` does not. The runner sets
   `TWYDO_SYNC_VDIR` to the vdir, so the hook works on the same vdir as a
   `tdo --vdir <dir> sync`. While it runs, the runner records itself in `sync/running.json`. It
   is killed after `timeout_secs`, or earlier when a save in the TUI asks to stop it (see
   [Stopping a sync](#stopping-a-sync)). Then `vdirsyncer metasync
   <pair>` runs if `metasync` is on and the sync did not fail (a failure such as a missing
   password would only repeat). A failed metasync is reported (`metasync failed: ...`) but is
   not a sync failure; a stopped one is (see [Stopping a sync](#stopping-a-sync)).
5. **Reconcile**, under the exclusive vdir lock (never held while waiting on the network):
   - Journal entries whose result the server now holds are marked synced. An entry is kept until
     then.
   - vdirsyncer settles delete-versus-edit silently, in both directions. Twydo notices from the
     journal and records it as a conflict (below), so a deletion is not lost without a trace. A
     re-upload of Twydo's edit to a server that deleted the item is recorded even when the run
     failed (the list errored, the run timed out, or printed errors of its own); the entry is
     confirmed only by a run that counts. If the upload itself failed, the conflict is spurious:
     keep the local version.
   - Held conflicts that you decided, or that both sides now agree on, are marked resolved.
   - Journal entries, conflicts and refused queued edits older than `retention_days` (default
     30) are pruned.
   - Edits queued while vdirsyncer ran are applied now, and when any was applied the runner does
     another pass (unless it was the last) so they are sent. An edit that cannot be applied
     because its file changed on another device is not saved; the next foreground `tdo sync`
     prints `tdo: Not saved: "<title>" changed on another device` once.
6. **Status.** Each list is classified and `status.json` is written.
7. **Rerun** if a request arrived meanwhile (at most twice).

A path whose vdir file differs from Twydo's edit after a clean sync of its list, with no
conflict to explain it, means the server changed the item after the edit reached it in a run
Twydo did not see (vdirsyncer by hand, or an edit made before sync was turned on). The entry is
confirmed with the server's version and the run warns once:
`tdo: warning: <path> was changed on the server after its edit synced; keeping the server's version`.
If the file still holds what the edit replaced, an unresolved conflict exists for it, or its list
did not sync cleanly, the entry stays pending instead, and every run warns
`tdo: warning: <path> changed before its edit synced; the edit stays pending`.

## Conflicts

`tdo status` and `tdo conflicts` list unresolved ones. `tdo conflicts show <id>` prints both
versions and the paths of their `.ics` files, then the task as the vdir holds it now (`current:`,
or `(absent)`). `tdo conflicts resolve <id> --keep local|remote|current` keeps one version. In the
TUI, `:conflicts` opens the list (`j`/`k` move, `l` keep local, `r` keep remote, `c` keep current,
`Esc` closes); a conflicted task is marked `!`.

Conflict states: `open`, `chosen` (shown as `chosen: keep local` / `chosen: keep remote`),
`resolved`, `superseded` (a newer conflict for the same task replaced it). Open and chosen ones
are unresolved.

| Kind (`conflict.json`) | In words | Local / remote |
|---|---|---|
| `both-changed` | changed here and on another device | both versions exist |
| `local-delete-remote-edit` | deleted here, edited on another device | local side is "(deleted)" |
| `remote-delete-local-edit` | edited here, deleted on another device | remote side is "(deleted)" |
| `journal` | an interrupted change here meets another device's version | local is the interrupted write, remote is the file as found; the vdir keeps the file as found |

What a choice does:

- **`both-changed`** is held: vdirsyncer keeps the two sides apart and the item stays out of sync
  (the vdir keeps the local version meanwhile). `resolve` only records the choice and prints
  `recorded; the next sync applies it`. On the next sync vdirsyncer calls the hook again, which
  copies the chosen version over the other, and vdirsyncer writes it to both sides. The runner
  then closes the conflict.
- **`local-delete-remote-edit`**, **`remote-delete-local-edit`** and **`journal`** are not held:
  vdirsyncer already settled the item (it restored the edit, or re-uploaded it), so the vdir
  holds one version. Keeping that one prints `already in place; conflict resolved`. Keeping the
  other writes it into the vdir through the journal (a deletion removes the file) and prints
  `written; conflict resolved`; the next sync propagates it like any edit. Either way the file
  is checked first: if it no longer holds the version it held when the conflict was recorded (a
  later sync or edit changed or deleted the task), `resolve` refuses, writes nothing and leaves
  the conflict open, pointing to `tdo conflicts show <id>` and `--keep current`.
- **`--keep current`** (any conflict that is not held) closes the conflict and leaves the vdir
  file exactly as it is now, whichever version that is (`kept as it is now; conflict resolved`).
  Both stored versions stay in the conflict's files. A held conflict refuses it: vdirsyncer
  still keeps both sides apart, so choose local or remote.

Until you choose, nothing is discarded: both versions stay in the conflict store, other tasks keep
syncing.

## Emptied lists

vdirsyncer refuses to sync a list that became empty on either side until it is told to
(`--force-delete`). Twydo handles the two cases differently:

- **Emptied here by Twydo.** The runner remembers which task files each list held at the end of
  the last sync in which it counted (`sync/manifest.json`). If pending journal entries removed
  every one of those files (moving or deleting the tasks) and the list has no unresolved conflict,
  the runner reruns that list with `--force-delete` itself. A foreground run prints
  `applied the emptied list "<name>" on the server`.
- **Anything else** is never forced: emptied on the server, local files that vanished some other
  way (even if Twydo deleted some of the others), or a list Twydo has not synced yet. `tdo sync`
  exits 1 with
  `list "<name>" was emptied here; to apply that on the server run: tdo sync --force-delete "<name>"`
  (or `... emptied on the server; to apply that here ...`). After checking that this is what you
  want, run the command. `--force-delete` takes a list's directory name or its unique display
  name, and can be repeated. In the TUI use `:sync! <list>`. The TUI reloads tasks when
  a manual sync finishes, including after a failed run that may have applied some changes.
  This refresh also works when live filesystem notifications are unavailable.

The record of a list is only as good as the last sync that wrote it:

- A sync that fails for a list (an error in it, or a run that timed out or printed errors of its
  own) drops the list's record, since it may have downloaded tasks the record lacks. The list is
  not forced automatically until the next clean sync of it records it again. A list reported as
  emptied keeps its record: vdirsyncer transfers nothing in it.
- An unreadable `sync/manifest.json` counts as no record: the run goes on, warns
  `tdo: warning: sync/manifest.json is unreadable (...); emptied lists will not be applied
  automatically until the next sync rewrites it`, and rewrites the file at its end.
- Each sync reads the record and removes the file before vdirsyncer runs, and writes it again
  only at the end of its update. A sync that is killed, fails, or cannot write the file (it warns)
  leaves no record, so the next one forces nothing.

## Deleted lists and recovery

Deleting a collection in 2Do can leave it in vdirsyncer's cached list set, so later syncs
report Not Found. Twydo names the affected list and collection id, and gives the exact
`:forget-list <id>` command. The TUI's `:errors` report retains these details, including
when several lists fail; j/k scrolls the report. A missing task can also produce Not Found,
so Twydo does not remove a list automatically.

If you deleted the list on another device, run `:forget-list <id>` (or its unique display
name) and confirm with `y`. Twydo preserves its complete local folder, list metadata,
unsynced journal evidence, and both versions of recorded conflicts outside the synced vdir
before removing the local folder. Cancelling or a changed confirmation snapshot writes
nothing. Other lists' pending work remains active; archived work is not marked synced.

The next sync first runs the ordinary `vdirsyncer discover <pair>`, with the configured
executable/configuration, timeout, and lock protection. It never automatically answers yes
to creation prompts. This refreshes the cached collection set before sync and metasync.
A discovery failure keeps the backup and discovery request, reports the failure, and blocks
ordinary sync until discovery succeeds. If several lists were removed remotely, forgetting
one can leave discovery blocked on another. The last Not Found reports for remaining local
lists stay available in `:errors` and the `:forget-list` chooser; remote missing-collection
diagnostics from discovery add new reports. Retired lists leave that report, and a later
ordinary sync replaces it with current results. Even without a recognized per-list report,
the discovery failure points to `:forget-list` for other lists you know you deleted; the
underlying vdirsyncer diagnostic is available in `:errors` and the CLI report. Confirm each
remote deletion separately; Twydo never retires another list automatically. With explicit collection names in the
vdirsyncer configuration, remove the deleted entry there before retrying. The usual
`collections = ["from a", "from b"]` setup needs no configuration change.
Only confirm deletion of a list you actually removed remotely: an existing remote list
can be discovered and downloaded again.

Use `:recover` to browse preserved lists, see their backup ids, and preview tasks/notes.
Recover to an existing list with `:recover <backup-id> <destination-list>`; the destination
can be a collection id or unique display name, including spaces. Review the local snapshot
and confirm with `y`. Recovery uses the existing gated move/copy implementation and writes
the complete accepted task set as one journalled operation with fresh identities and
preserved family/history links. Unsupported shapes or unreadable resources refuse the whole
recovery; the original backup stays available. Historical conflict alternatives stay in the
backup files for inspection; recovery does not merge or restore those alternatives.

Backups live under the per-vdir state directory at `retired/<backup-id>/`:
`backup.json` records the original collection, capture time, file inventory, journal
evidence, conflict ids, and any recovery operation/destination. Original files are under
`data/<original-collection>/`; conflict records and both versions are under `conflicts/`.
Directories are private to the user. Backups have no automatic expiry or TUI delete command.
Back them up with the rest of Twydo's state.

A completed recovery retains its backup and can be undone/redone with the ordinary session
keys. A retry recognizes the recorded operation rather than making duplicates. After undo,
use Ctrl-r; a repeated recovery refuses to resurrect removed tasks or overwrite later edits.
A backup cannot be recovered to a second destination through these commands.

## Backoff and timeout

vdirsyncer is killed (with its child processes) after `timeout_secs` (default 300); the run counts
as a failure, and every list it reported (conflicted ones too) is marked `error`, so a timed-out
run confirms no edit. A stopped run (below) is handled the same way, with the error
`sync stopped`. A run fails transiently when the command cannot be started, times out, is stopped, prints
errors that belong to no conflict or emptied-list report, leaves a list in `error`, or exits
non-zero for no stated reason. Conflicts and emptied lists are not failures.

After `n` failures in a row a background run waits `60 s * 2^(n-1)` before trying again, at most
`max_backoff_secs` (default 1800). A manual `tdo sync` ignores the wait. A success resets the
count. `tdo status` shows `failures: n (next attempt ...)` and the last error. A failed sync never
blocks local use or notifications: the runner holds no exclusive lock while it waits on the
network.

## Stopping a sync

A sync holds the vdir for as long as vdirsyncer runs, and a slow one (for example vdirsyncer
waiting for a password prompt) blocks every write meanwhile. A save in the TUI can stop it
([tui.md](tui.md#when-a-write-is-refused)):

- While vdirsyncer runs, the runner keeps `sync/running.json`: its own process id, vdirsyncer's
  process group, when each of those processes started, and when the sync started. A runner
  ended with Ctrl-C leaves the file behind until the next sync replaces it.
- A record counts only while its runner or vdirsyncer is still the process it names: same id
  and same start time (`ps -o lstart=`). A record whose ids now belong to other processes is
  stale: `tdo` removes it and signals nothing.
- To stop it, `tdo` creates `sync/stop`. The runner checks for that file while it waits for
  vdirsyncer (or metasync), kills vdirsyncer and its child processes, and records the run as
  failed with `sync stopped`: it backs off, its lists show `error`, and no edit is confirmed. The edits stay
  pending and go to the server with the next sync that works. A stopped run is not rerun even if
  another runner asked for one.
- If the runner itself is gone (for example `tdo sync` was ended with Ctrl-C and left vdirsyncer
  running, see Known limits), `tdo` kills the recorded process group itself and records the
  stopped sync, with its backoff, so the sync started after the save does not get stuck again
  at once.
- The request stays until the save that asked for it is done, and `tdo` then removes it (it is
  written even when no sync is recorded at that moment, for a pass about to start). Meanwhile
  any sync that starts (one that was waiting behind the stopped one, or the timer) stops before it
  runs vdirsyncer, so it cannot take the vdir first. A request older than 30 seconds is left over
  from a `tdo` that is gone: it is ignored and removed.
- Each stop also records when it was asked for (`sync/last_stop`, never removed). A runner
  that began before that time stops before starting vdirsyncer, even if it was delayed taking
  the run lock, reading status, or waiting for the vdir. Every pass in that run honors the stop
  and records it with backoff. Runs begun later are not affected.

## State files

All under the per-vdir state directory, `<state_dir>/vdirs/<id>/` (see [architecture.md](architecture.md#the-operation-journal);
`state_dir` defaults to `$XDG_STATE_HOME/twydo`). The JSON files are pretty-printed and meant to be
read; do not edit them while a sync runs.

| Path | Content |
|---|---|
| `sync/off` | exists while automatic sync is off (`tdo sync off`); see [When edits sync](#when-edits-sync) |
| `sync/waiter.lock` | the batch waiter's lock (one waiter per vdir) |
| `sync/ended` | the last run that ended: `run_id` and `at`; the TUI reloads when it changes |
| `sync/rediscover` | a confirmed list retirement requires ordinary discovery before another sync; removed only when discovery succeeds |
| `retired/<id>/` | permanent local-list backup, journal/conflict evidence, and recovery operation; see [deleted lists and recovery](#deleted-lists-and-recovery) |
| `sync/status.json` | `last_attempt`, `last_success`, `failures`, `next_attempt`, `last_error`, `metasync_error`, and per list `state`: `ok`, `conflict`, `emptied-local`, `emptied-remote` or `error` (with `message`) |
| `sync/run.lock` | the run lock (one runner per vdir) |
| `sync/requested` | exists while another runner asked for one more pass |
| `sync/running.json` | exists while vdirsyncer runs (and after a runner ended with Ctrl-C, until the next sync): `runner_pid`, `runner_start`, `child_pgid` (vdirsyncer's process group, `null` between calls), `child_start`, `started` |
| `sync/last_stop` | when the last stop was asked for (RFC 3339); a run begun before it does not start vdirsyncer |
| `sync/stop` | a request to stop the running sync, and to start none, until the save that asked is done (at most 30 s) |
| `sync/manifest.json` | per list, the task files (`<list>/<file>.ics`) the vdir held after the last sync in which the list counted; the evidence for forcing a list Twydo emptied |
| `sync/hook.log` | one JSON line per hook call of the current run; the runner sets `TWYDO_SYNC_HOOKLOG` (and `TWYDO_SYNC_VDIR`) for vdirsyncer |
| `conflicts/<id>/conflict.json` | the record: `uid`, `collection`, `path`, `kind`, `held`, `vdir_holds`, `detected`, `state`, `choice`, `resolved` |
| `conflicts/<id>/local.ics`, `remote.ics` | the two versions as plain `.ics`; a missing file means that side deleted the task |
| `conflicts/.lock` | the conflict store lock |
| `journal/` | the operation journal |

Lock order is always run lock, vdir lock, conflicts lock.

## Known limits

- **A `journal` conflict leaves already-applied writes in place.** When recovery abandons an
  entry, writes of that entry that were already applied to the vdir stay. Every other write of it
  becomes a `journal` conflict: one that collides with a newer version, and one that was never
  applied (its remote side is then the unchanged file), so keeping local finishes it.
- **Deletes without an etag.** vdirsyncer sends deletes to Synology without an etag
  (`Deleting an item with no etag`), and the iPhone may show a one-off "Remote server returned an
  invalid task reference during incremental sync" on its next refresh; a retry succeeds and the data
  is right ([plan4-baseline.md](reference/phase0/plan4-baseline.md)). Twydo cannot prevent this.
- **Do not run vdirsyncer by hand while Twydo has pending edits.** The hook still records
  both-changed conflicts (it runs inside vdirsyncer), but nothing confirms journal entries or
  closes conflicts until the next `tdo sync`, and a delete-versus-edit that vdirsyncer settles
  silently in that run (it re-uploads Twydo's edit, or restores the other device's edit) leaves
  no trace for Twydo: a deletion made on the phone can be undone without a conflict. `tdo status`
  shows `pending: 0` when it is safe.
- **Chosen versions are written back with LF line endings.** When the hook applies a choice,
  vdirsyncer reads the hook's temporary files in text mode and writes the chosen task back with LF
  instead of CRLF. 2Do on the iPhone and Synology read such a task correctly, and 2Do can edit it
  again.
- **Ctrl-C does not stop vdirsyncer.** Pressing Ctrl-C (or closing the terminal) during an
  interactive `tdo sync` stops `tdo` but not vdirsyncer: it runs in its own process group so that
  a timeout can kill it together with its child processes. It finishes on its own, still holding
  its shared vdir lock, so writers (`tdo op`, the TUI) and the next `tdo sync` wait until it
  exits, unless you stop it from the TUI. Its result is not reconciled by the killed run: the next `tdo sync` checks the journal
  again, but a delete-versus-edit it settled leaves no trace (as for vdirsyncer run by hand), and
  the `timeout_secs` limit no longer applies to it.
- **A create edited before it was confirmed.** A task created in Twydo whose create reached the
  server in a run that did not count (it failed or timed out), then edited in Twydo and deleted
  on the phone before the next clean sync, is re-uploaded without a conflict: Twydo cannot tell
  that upload from the create's first one.
- **Repeating tasks completed on two devices.** If the phone and Twydo both complete the same
  occurrence of a repeating task before a sync, both advance the live task the same way, so
  vdirsyncer sees no conflict; but each adds its own completed copy, so the occurrence shows as
  done twice. Nothing is lost; delete one copy if it bothers you.
- **A locked password store stalls sync.** If vdirsyncer fetches the password with `pass` and the
  gpg agent needs your passphrase, vdirsyncer waits for pinentry (a few minutes) while holding the
  vdir lock. A save in the TUI then asks whether to stop the sync ([Stopping a sync](#stopping-a-sync)).
  To avoid it, unlock gpg, or let the agent cache the passphrase longer.

All of these flows were checked against 2Do on an iPhone; the session log is
[plan3-sync.md](reference/phase0/plan3-sync.md).
