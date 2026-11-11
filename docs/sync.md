# Sync

Twydo does not talk to the network. Its daemon (`tdo daemon`, [architecture.md](architecture.md#the-daemon))
runs [vdirsyncer](https://vdirsyncer.pimutils.org/) against the vdir, records what it changed in the operation journal ([architecture.md](architecture.md#the-operation-journal)), and keeps
both versions of anything that was changed on two devices until you choose. Configuration keys are
in [config.md](config.md) (`[sync]`).

## Setup

1. **vdirsyncer config.** Start from `contrib/vdirsyncer/config.example`. The line that matters is

   ```
   conflict_resolution = ["command", "tdo", "conflicts", "hook"]
   ```

   It is not `a wins` / `b wins`. The hook keeps both versions in the conflict store and changes
   nothing until you choose. `tdo` must be on the `PATH` of vdirsyncer, which the daemon starts.
   **Do not run `vdirsyncer sync` by hand while the daemon runs.** The daemon does not know
   about that run: it cannot confirm edits, close conflicts or notice a delete-versus-edit that
   vdirsyncer settled silently (see [Known limits](#known-limits)). Use `tdo sync`.

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

3. **Check by hand.** `tdo sync` should print `synced`; `tdo status` shows the result. The
   daemon starts when needed.

4. **Background sync.** Nothing to set up: while the daemon runs it syncs by itself (at startup,
   every 60 seconds, after edits, after a conflict choice, when automatic sync is switched on
   and after the machine wakes), and the TUI and the commands start it. To keep the daemon (and
   so the sync) running while no `tdo` is open, run `tdo daemon install` once: it installs a
   login service ([install.md](install.md#start-it-at-login)).

   The old timer (`twydo-sync.timer` on Linux, `app.twydo.sync` on macOS) is retired. `tdo
   daemon install` disables it if it finds it and leaves the files in place; `tdo sync
   --background` (hidden) now only starts the daemon, so a timer you keep is harmless.

   Do not run `vdirsyncer sync` by hand while the daemon runs: it would race the daemon's own
   sync. Use `tdo sync`, which asks the daemon.

## When edits sync

Edits sync in batches. An edit is **unsynced** while its journal entry is applied, not yet
confirmed by a sync, and made after the last sync attempt began; entries an earlier sync saw
and left pending (an open conflict, a list that did not count) wait for you or the next sync
instead. A sync is **due** once `batch_size` edits (default 5) are unsynced and edits have
paused for 3 seconds, or `max_wait_secs` (default 60) after the oldest of them
([config.md](config.md)), whichever comes first, or later while the runner backs off. The
pause keeps a run of quick edits (closing checklist items one by one) from being interrupted by
its own sync. A save made while a sync is running is queued and written when the sync ends
([tui.md](tui.md#when-a-write-is-refused)).

The daemon checks that rule after every write (from the TUI, `tdo op` or a `tdo conflicts
resolve` that writes the vdir). When a sync is due it runs one at once; otherwise it waits for
the batch to fill or the quiet period to pass, rechecking as more edits arrive, so a run of quick
edits is sent in one sync. Deadlines use wall clock, so an overdue sync starts after the laptop
wakes. Quitting the TUI does not lose anything: the daemon outlives it and sends the last edits
within `max_wait_secs`.

A conflict choice recorded for a held conflict is not an edit and only applies on a sync, so
`resolve` starts one at once. Choosing again before that sync replaces the choice (the latest
wins). `:sync` and `tdo sync` sync now, whatever the count; if a sync is already running, they wait for it and then run theirs.

**Turning automatic sync off.** `tdo sync off` (or `:sync off` in the TUI) stores
`sync/off`, so it lasts across restarts and every process sees it. While off, every automatic
sync skips quietly and records no failure: the daemon's scheduled syncs (startup, every 60
seconds, wake, batches) and a background run. Edits stay local and pending; the TUI's status line shows `sync off`
with the pending count, and `tdo status` prints `automatic sync: off`. `:sync` and `tdo sync`
still run once. `tdo sync on` (`:sync on`) removes the file and, with unsynced edits, starts a
sync at once.

**What the TUI sees.** The daemon tells the TUI when a sync starts and ends and when files
change, so the TUI shows what any sync brought in, whoever asked for it (`tdo sync` in another
terminal, the daemon itself).

## What `tdo sync` does

`tdo sync` asks the daemon to sync and waits for the result (`tdo sync on` and `off` switch
automatic sync; `tdo sync --background` only starts the daemon). Exit codes: 0 when every list is fine; 1 when the run had problems (conflicts or
emptied lists), failed or was stopped; 2 when the daemon refused the call, could not be
reached, or the sync job itself failed or was interrupted. Problems are printed to stderr as `tdo: ...`.
`--background` prints nothing and does not wait. A run the daemon makes by itself honours
backoff; `tdo sync` and `:sync` run once whatever the backoff.

One run, in order:

1. **Run lock.** At most one runner per vdir (`run.lock` in the state directory). The daemon
   runs syncs one at a time on its vdir lane, so `:sync` and `tdo sync` wait behind a running
   sync. A run that finds the lock taken leaves a request (`requested`); the running runner
   then does one more pass, at most twice.
2. **Backoff.** A scheduled run does nothing while the status says `next_attempt` is in the
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
   is killed after `timeout_secs`, or earlier when it is stopped (see
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
  emptied keeps its record: vdirsyncer transfers nothing in it. The runner captures this
  record before applying post-sync queued edits, so deleting the final task from the queue
  retains the evidence needed to send that deletion automatically on the next pass.
- An unreadable `sync/manifest.json` counts as no record: the run goes on, warns
  `tdo: warning: sync/manifest.json is unreadable (...); emptied lists will not be applied
  automatically until the next sync rewrites it`, and rewrites the file at its end.
- Each sync reads the record and removes the file before vdirsyncer runs, and writes it again
  only at the end of its update. A sync that is killed, fails, or cannot write the file (it warns)
  leaves no record, so the next one forces nothing.

## New lists from other devices

A list created on another device (the phone) is picked up by discovery. Run
`vdirsyncer discover <pair>` by hand only once, at setup; after that Twydo discovers for you.
Discovery runs at these points only:

- once after the daemon starts: it leaves a `sync/discover` marker, and the first sync that
  gets past the skip checks (timer, batch, manual or the startup sync itself) discovers and then
  removes the marker, whether the discovery succeeded or failed, so a broken helper is not
  retried on every sync. A discovery that was stopped learned nothing and keeps the marker; one that timed out
  is retried once more (the marker remembers it), so a hanging discovery does not cost a full
  timeout on every sync. A startup sync skipped by backoff, or while automatic sync is off,
  therefore does not lose it;
- on request: `tdo sync --discover` or `:discover` in the TUI. These discover, then sync,
  and report like `tdo sync`. `--discover` cannot be combined with `on`/`off` or `--background`;
- after `:forget-list`, as part of the recovery below.

Ordinary syncs (timer, after edits, plain `tdo sync` and `:sync`) do not discover and make no
extra server request, so a new phone list appears after the next daemon start or `:discover`.

Discovery only creates local folders. A collection that exists on the server and not locally
is adopted: the same run syncs and metasyncs it, so it arrives with its tasks, name and color,
and the sync note says `found new list "<name>"`. A collection missing on the server is never
created there, because a local folder without a server counterpart usually means the list was
deleted on the phone. Discovery runs through Twydo's bundled helper, not the plain
`vdirsyncer discover` command, which cannot tell the two cases apart.

A local list whose server collection is gone (deleted on the phone, not yet forgotten) stops
discovery, as vdirsyncer does, so no new list is adopted in that run. The report names the
list and its collection id and gives the `:forget-list <id>` command; see
[Deleted lists and recovery](#deleted-lists-and-recovery). Ordinary sync continues. Discovery
reports one deleted list at a time, because each deletion must be confirmed separately; after
you forget one, the next discovery reports the next.

The discovery helper runs under the sync run: it is recorded in `sync/running.json`, holds the
shared vdir lock (inherited from the run, so it outlives even a killed daemon) for as long as
it runs, and is killed with its process group when the sync is stopped (`tdo sync stop`, a write
that stops it, or the daemon shutting down after its grace period) or times out. If discovery
fails, the folders it had already created are removed again (only while empty), by the helper
itself or, when it was killed before it could (stopped, timed out, a signal), by the sync run, so no list is half adopted.

A failed discovery (server unreachable, a pair layout the helper refuses, a stale list,
timeout) keeps the previous cached collection set. With `tdo sync --discover` it counts as a
problem: the ordinary sync still runs, but the exit code is 1. At daemon startup it is only
a warning on the startup sync. A helper that cannot run at all (the `[sync].list_python`
interpreter does not start, or vdirsyncer is not importable from it) is likewise only a
warning, and is not saved, when the discovery was not requested (the startup marker and the
`:forget-list` recovery); `tdo sync --discover` reports and saves it. Any other failure is saved in
`sync/status.json` (`discovery_error`) and shown as `discovery error:` in `tdo status` and in
the TUI's `:errors` report until a later discovery succeeds; runs that do not discover leave
it alone. A list removed with `:forget-list` that still exists on the server is adopted again
by the next discovery.

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

The next sync first runs the local-only discovery described in
[New lists from other devices](#new-lists-from-other-devices). When it succeeds it has refreshed
the cached collection set itself, so the request is cleared and the plain command is skipped.
When it fails or cannot run (a warning), the ordinary `vdirsyncer discover <pair>` runs, with the configured
executable/configuration, timeout, and lock protection. It never automatically answers yes
to creation prompts. This refreshes the cached collection set before sync and metasync.
A discovery failure keeps the backup and discovery request, reports the failure, and blocks
ordinary sync until discovery succeeds. Lists the helper adopted already have local folders,
so a new phone list no longer makes this discovery fail. If several lists were removed remotely, forgetting
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

## The write queue

The queue's layout, claiming and the strict checks are in
[architecture.md](architecture.md#the-write-queue). From the outside: a write that finds the
lock taken is queued and applied at the next drain point, which is the sync runner (step 3 after
recovery, step 5 after reconcile), the daemon when a sync or job ends, any writer that gets the
lock, or `tdo op --wait`. Rebasing is strict for now: an item whose file changed on another
device is refused, never merged. A queued edit counts as unsynced from when it was queued, so a
batch ages from then, not from the drain.

## Stopping a sync

A sync holds the vdir for as long as vdirsyncer runs, and a slow one (for example vdirsyncer
waiting for a password prompt) holds the vdir meanwhile; saves are queued until it ends.
`tdo op` queues too: it prints `queued (a sync is running): <queue id>` and exits 0, or with
`--wait` waits (saying so once on stderr), then prints the journal entry id (exit 0) or the
refusal on stderr (exit 1). A queued write that is refused is reported once on stderr by the
next `tdo op` or `tdo sync`.

When a `tdo op`, `tdo sync` or `tdo conflicts` command gets no answer from the daemon for a
minute, it says `the daemon did not answer; ... (request <request-id>)` and names the check
that tells what happened:

- A task write, `tdo op undo` or `tdo sync` (a sync run): `tdo op status <request-id>`. It
  prints one line: `pending` (queued behind a sync), `applied <journal id>`, `refused: <reason>`,
  `cancelled`, `satisfied <journal id>`, or, for a sync, `queued`, `running`, `finished`,
  `failed: <reason>` or `interrupted`. It exits 0 when the daemon knows the request and 1 when
  it does not (`no record of request <id>: only task writes and syncs are looked up; it may not
  have reached the daemon, or may still be applied`). No record does not prove the change
  never ran: a request still on its way can be applied after the lookup, and old records are
  pruned. Look at the task itself (or `tdo status` for a sync) before you run a write again,
  since a new run can add a second task.
- `tdo sync on|off`: check with `tdo status`; running it again is safe (it sets the same value).
- `tdo conflicts resolve`: check with `tdo conflicts`; choosing again is safe (the same side is
  recorded again, or the daemon says the conflict is already resolved).
- `tdo op abandon`: check with `tdo op journal`; running it again is safe (an entry that is
  already abandoned answers `abandoned: false`).

No `tdo` command or TUI key stops a running sync today. The daemon can: its `sync.stop` method
(see [protocol.md](protocol.md#data-writes-and-events)) asks the running sync to stop, and the daemon does the same when
it shuts down during a sync, after `shutdown_grace`. Ctrl-C on `tdo sync` only stops that client
from waiting: the daemon's sync runs to its end, and its result shows in `tdo status` and the TUI.
A slow sync (a password prompt) therefore holds the vdir until vdirsyncer exits or `timeout_secs`
kills it.

How a stop works:

- While vdirsyncer runs, the daemon's runner keeps `sync/running.json`: its own process id,
  vdirsyncer's process group, when each of those processes started, and when the sync started. The
  record can outlive the runner only when the daemon itself is killed.
- A record counts only while its runner or vdirsyncer is still the process it names: same id
  and same start time (`ps -o lstart=`). A record whose ids now belong to other processes is
  stale: it is removed and nothing is signalled.
- To stop a sync, the daemon creates `sync/stop`. The runner checks for that file while it waits
  for vdirsyncer (or metasync), kills vdirsyncer and its child processes, and records the run as
  failed with `sync stopped`: it backs off, its lists show `error`, and no edit is confirmed. The
  edits stay pending and go to the server with the next sync that works. A stopped run is not
  rerun even if another runner asked for one.
- If the daemon was killed during a sync and left vdirsyncer running, the next daemon start
  finds the record, kills the recorded process group before recovery and records the stopped
  sync, with its backoff (`tdo daemon: stopped a vdirsyncer left by an earlier run` in the
  log). If another process is syncing the vdir, it leaves that alone and writes wait for it.
- The request stays until the sync that was asked to stop has ended, and the daemon then removes
  it. Meanwhile any sync that starts (one that was waiting behind the stopped one, or a scheduled
  one) stops before it runs vdirsyncer, so it cannot take the vdir first. A request older than 30
  seconds is left over from a process that is gone: it is ignored and removed.
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
| `sync/rediscover` | a confirmed list retirement requires ordinary discovery before another sync; removed only when discovery succeeds |
| `retired/<id>/` | permanent local-list backup, journal/conflict evidence, and recovery operation; see [deleted lists and recovery](#deleted-lists-and-recovery) |
| `sync/status.json` | `last_attempt`, `last_success`, `failures`, `next_attempt`, `last_error`, `metasync_error`, `discovery_error`, and per list `state`: `ok`, `conflict`, `emptied-local`, `emptied-remote` or `error` (with `message`) |
| `sync/run.lock` | the run lock (one runner per vdir) |
| `sync/requested` | exists while another runner asked for one more pass |
| `sync/running.json` | exists while vdirsyncer runs (and after a killed daemon, until the next daemon start reaps it): `runner_pid`, `runner_start`, `child_pgid` (vdirsyncer's process group, `null` between calls), `child_start`, `started` |
| `sync/last_stop` | when the last stop was asked for (RFC 3339); a run begun before it does not start vdirsyncer |
| `sync/stop` | the daemon's request to stop the running sync (`sync.stop`, or a shutdown during a sync) and to start none meanwhile; the requester withdraws it once its save is done, and one older than 30 s is stale and ignored |
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
  is right ([plan4-baseline.md](reference/evidence.md)). Twydo cannot prevent this.
- **Do not run vdirsyncer by hand while the daemon runs (unsupported).** The hook still records
  both-changed conflicts (it runs inside vdirsyncer), but nothing confirms journal entries or
  closes conflicts until the next `tdo sync`, and a delete-versus-edit that vdirsyncer settles
  silently in that run (it re-uploads Twydo's edit, or restores the other device's edit) leaves
  no trace for Twydo: a deletion made on the phone can be undone without a conflict. `tdo status`
  shows `pending: 0` when it is safe.
- **Chosen versions are written back with LF line endings.** When the hook applies a choice,
  vdirsyncer reads the hook's temporary files in text mode and writes the chosen task back with LF
  instead of CRLF. 2Do on the iPhone and Synology read such a task correctly, and 2Do can edit it
  again.
- **A killed daemon leaves vdirsyncer running.** vdirsyncer runs in its own process group so that
  a timeout can kill it together with its child processes. If the daemon is killed during a sync,
  vdirsyncer finishes on its own, still holding its shared vdir lock, so writes wait until it
  exits or the next daemon start stops it. Its result is not reconciled: the next sync checks the
  journal again, but a delete-versus-edit it settled leaves no trace (as for vdirsyncer run by
  hand), and the `timeout_secs` limit no longer applies to it.
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
  vdir lock. Saves in the TUI are queued until it ends.
  To avoid the wait, unlock gpg, or let the agent cache the passphrase longer.

All of these flows were checked against 2Do on an iPhone; the session log is
[plan3-sync.md](reference/evidence.md).
