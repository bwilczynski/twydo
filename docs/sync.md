# Sync

Twydo does not talk to the network. It runs [vdirsyncer](https://vdirsyncer.pimutils.org/) against
the vdir, records what it changed in the operation journal ([journal.md](journal.md)), and keeps
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
   # vdirsyncer_config = "~/twydo-phase0/xdg/vdirsyncer/config"   # test setup, see below
   ```

   `local_storage` must be the name of the pair's vdir-side storage in the vdirsyncer config. The
   hook tells the local file from the remote one by that name.

   In the test setup, `XDG_CONFIG_HOME` is set for `tdo` only. That hides vdirsyncer's own config,
   so set `vdirsyncer_config`: Twydo passes it to vdirsyncer as `VDIRSYNCER_CONFIG`.

3. **Check by hand.** `tdo sync` should print `synced`; `tdo status` shows the result.

4. **Background sync.** Pick one:

   - Linux, systemd user units:

     ```
     cp contrib/systemd/twydo-sync.service contrib/systemd/twydo-sync.timer ~/.config/systemd/user/
     systemctl --user daemon-reload
     systemctl --user enable --now twydo-sync.timer
     ```

     Adjust `ExecStart` to where `tdo` lives. For the test setup, uncomment the `Environment=`
     line. A run that skips (backoff, or another runner busy) exits 0.

   - macOS, launchd agent: copy `contrib/launchd/app.twydo.sync.plist` to
     `~/Library/LaunchAgents/`, replace `/usr/local/bin/tdo` and `/Users/USER` with real paths
     (launchd does not expand `~`), then
     `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/app.twydo.sync.plist`.

   Both run `tdo sync --background` every minute. Independently of the timer, `tdo op` writes and
   `tdo conflicts resolve` start a detached `tdo sync --background` when `[sync] after_edit` is on
   (the default).

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
3. **Recovery.** Unfinished journal entries are rolled forward before vdirsyncer runs. For an entry
   that cannot be finished, every write whose file holds neither its `before` nor its `after`
   becomes a `journal` conflict and the entry is abandoned (see Known limits).
4. **vdirsyncer** `sync <pair>` runs under the shared vdir lock, so readers (the TUI, `tdo op`
   reads) continue and writers wait. It is killed after `timeout_secs`. Then `vdirsyncer metasync
   <pair>` runs if `metasync` is on. A failed metasync is reported (`metasync failed: ...`) but is
   not a sync failure.
5. **Reconcile**, under the exclusive vdir lock (never held while waiting on the network):
   - Journal entries whose result the server now holds are marked synced. An entry is kept until
     then.
   - vdirsyncer settles delete-versus-edit silently, in both directions. Twydo notices from the
     journal and records it as a conflict (below), so a deletion is not lost without a trace.
   - Held conflicts that you decided, or that both sides now agree on, are marked resolved.
   - Journal entries and conflicts older than `retention_days` (default 30) are pruned.
6. **Status.** Each list is classified and `status.json` is written.
7. **Rerun** if a request arrived meanwhile (at most twice).

A path that changed in the vdir with no conflict to explain it stays pending, with
`tdo: warning: <path> changed before its edit synced; the edit stays pending`.

## Conflicts

`tdo status` and `tdo conflicts` list unresolved ones. `tdo conflicts show <id>` prints both
versions and the paths of their `.ics` files. `tdo conflicts resolve <id> --keep local|remote`
keeps one version. In the TUI, `:conflicts` opens the list (`j`/`k` move, `l` keep local, `r` keep
remote, `Esc` closes); a conflicted task is marked `!`.

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
  `written; conflict resolved`; the next sync propagates it like any edit.

Until you choose, nothing is discarded: both versions stay in the conflict store, other tasks keep
syncing.

## Emptied lists

vdirsyncer refuses to sync a list that became empty on either side until it is told to
(`--force-delete`). Twydo handles the two cases differently:

- **Emptied here by Twydo.** If a pending journal entry removed the last task (moving or deleting
  it) and the list has no unresolved conflict, the runner reruns that list with `--force-delete`
  itself. A foreground run prints `applied the emptied list "<name>" on the server`.
- **Anything else** (emptied on the server, or the local files vanished some other way) is never
  forced. `tdo sync` exits 1 with
  `list "<name>" was emptied here; to apply that on the server run: tdo sync --force-delete "<name>"`
  (or `... emptied on the server; to apply that here ...`). After checking that this is what you
  want, run the command. `--force-delete` takes a list's directory name or its unique display
  name, and can be repeated. In the TUI use `:sync! <list>`.

## Backoff and timeout

vdirsyncer is killed (with its child processes) after `timeout_secs` (default 300); the run counts
as a failure. A run fails transiently when the command cannot be started, times out, prints
errors that belong to no conflict or emptied-list report, leaves a list in `error`, or exits
non-zero for no stated reason. Conflicts and emptied lists are not failures.

After `n` failures in a row a background run waits `60 s * 2^(n-1)` before trying again, at most
`max_backoff_secs` (default 1800). A manual `tdo sync` ignores the wait. A success resets the
count. `tdo status` shows `failures: n (next attempt ...)` and the last error. A failed sync never
blocks local use or notifications: the runner holds no exclusive lock while it waits on the
network.

## State files

All under the per-vdir state directory, `<state_dir>/vdirs/<id>/` (see [journal.md](journal.md);
`state_dir` defaults to `$XDG_STATE_HOME/twydo`). The JSON files are pretty-printed and meant to be
read; do not edit them while a sync runs.

| Path | Content |
|---|---|
| `sync/status.json` | `last_attempt`, `last_success`, `failures`, `next_attempt`, `last_error`, `metasync_error`, and per list `state`: `ok`, `conflict`, `emptied-local`, `emptied-remote` or `error` (with `message`) |
| `sync/run.lock` | the run lock (one runner per vdir) |
| `sync/requested` | exists while another runner asked for one more pass |
| `sync/hook.log` | one JSON line per hook call of the current run; the runner sets `TWYDO_SYNC_HOOKLOG` for vdirsyncer |
| `conflicts/<id>/conflict.json` | the record: `uid`, `collection`, `path`, `kind`, `held`, `vdir_holds`, `detected`, `state`, `choice`, `resolved` |
| `conflicts/<id>/local.ics`, `remote.ics` | the two versions as plain `.ics`; a missing file means that side deleted the task |
| `conflicts/.lock` | the conflict store lock |
| `journal/` | the operation journal |

Lock order is always run lock, vdir lock, conflicts lock.

## Known limits

- **A `journal` conflict leaves already-applied writes in place.** When recovery abandons an
  entry, writes of that entry that were already applied to the vdir stay; only the writes that
  collide with a newer version are parked as conflicts.
- **Deletes without an etag.** vdirsyncer sends deletes to Synology without an etag
  (`Deleting an item with no etag`), and the iPhone may show a one-off "Remote server returned an
  invalid task reference during incremental sync" on its next refresh; a retry succeeds and the data
  is right (docs/phase0/plan4-baseline.md). Twydo cannot prevent this.
- **vdirsyncer run by hand records but confirms nothing.** If you run `vdirsyncer sync` yourself,
  the hook still records conflicts (it runs inside vdirsyncer), but nothing confirms journal
  entries, detects delete-versus-edit, or closes conflicts until the next `tdo sync`. Pending
  entries are kept until then, so nothing is lost.
- **Chosen versions are written back with LF line endings.** When the hook applies a choice,
  vdirsyncer reads the hook's temporary files in text mode and writes the chosen task back with LF
  instead of CRLF. Whether 2Do on the iPhone accepts this is to be verified on the iPhone.
- **Ctrl-C does not stop vdirsyncer.** Pressing Ctrl-C during an interactive `tdo sync` does not
  stop vdirsyncer: it runs in its own process group so that a timeout can kill it together with
  its child processes, so it finishes on its own.
- **Phone behaviour is verified later.** What 2Do on the iPhone does with these flows is checked in
  a later session (docs/phase0/plan3-sync.md, not yet written).
- **Repeating tasks completed on two devices.** If two devices complete the same repeating task
  before a sync, there are two history copies. Twydo records the conflict on the live task but does
  not merge the history (observation pending, Plan 3 Task 13).
