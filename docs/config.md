# Twydo configuration

File: `$XDG_CONFIG_HOME/twydo/config.toml`, else `~/.config/twydo/config.toml` (Linux and macOS
alike). An empty `XDG_CONFIG_HOME` counts as unset. Unknown keys are an error and the message
names the key. Twydo only reads this file.

## Top-level keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `vdir` | path | required | Root of the vdir: one subdirectory per list, one `.ics` per task. A leading `~` or `~/` expands to `$HOME`. |
| `timezone` | IANA name | system zone, else UTC | Zone used for "today" and floating times. |
| `week_start` | `"mon"`..`"sun"` (or full names, any case) | `"mon"` | First day of the week. Reserved: accepted, not used yet. |
| `work_days` | list of `"mon"`..`"sun"` | `["mon", "tue", "wed", "thu", "fri"]` | The days the repeat field's `workdays` stands for (2Do's "Work days" preset). Must not be empty. |
| `morning_time` | `"HH:MM"` | `"09:00"` | Time for date-only tasks that need one, such as reminders. Reserved: accepted, not used yet (notifications are not available yet). |
| `default_list` | list id | none | List new tasks go to when added in the TUI outside a list (Today, All, Scheduled, …). Unset or unknown: the list named Inbox, else the first list. `tdo op add` takes `--list` instead. |
| `state_dir` | path | `$XDG_STATE_HOME/twydo`, else `~/.local/state/twydo` | Where Twydo keeps its write journal and other state, per vdir under `vdirs/<id>/` (see [architecture.md](architecture.md#the-operation-journal)). A leading `~` expands to `$HOME`. |

## `[writes]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `mode` | `"verified"` or `"test"` | `"verified"` | `verified` runs only operations the compatibility matrix marks as passing. `test` also runs unverified operations. |

`test` must only be used with a config that points at a separate test account, never at your
real data: unverified operations may write something 2Do on the iPhone cannot read. Every
`tdo op` run in `test` mode prints `tdo: test write mode: unverified operations are enabled` to
stderr.

`tdo op` (add, edit, complete, reopen, delete, move, undo, check, journal, recover, abandon) is
the scriptable write interface. It prints the operation id and exits 0; exit 1 means the
operation was blocked, stale or stuck (reason on stderr); exit 2 is a usage, config or IO error.
`tui`, `export-json`, `check` and `op` first roll forward unfinished journal entries of
their vdir, kept under `state_dir`.

`tdo op --matrix <path>` is a hidden test-only hook: it replaces the compatibility matrix built
into `tdo` with the file at `<path>`. Never use it on real data.

## `[sync]`

Optional. A missing `[sync]` table turns sync off; an empty `[sync]` table turns it on with the
defaults below. Twydo runs the configured vdirsyncer command and has no network code of its own. Setup, behaviour
and conflict handling are in [sync.md](sync.md).

| Key | Type | Default | Meaning |
|---|---|---|---|
| `pair` | string | `"twydo"` | vdirsyncer pair to sync. |
| `local_storage` | string | `"twydo_local"` | Name of the pair's vdir-side storage in the vdirsyncer config. |
| `command` | string | `"vdirsyncer"` | vdirsyncer executable. |
| `vdirsyncer_config` | path | none | Passed to vdirsyncer as `VDIRSYNCER_CONFIG`. Needed when `XDG_CONFIG_HOME` is set for `tdo` only, because that hides vdirsyncer's own config. A leading `~` expands to `$HOME`. |
| `metasync` | bool | `true` | Run `vdirsyncer metasync` after each sync. |
| `batch_size` | integer | `5` | A sync is due once this many local edits are unsynced. Must be at least 1. |
| `max_wait_secs` | integer | `60` | A sync is due this many seconds after the oldest unsynced edit. Must be at least 1. |
| `timeout_secs` | integer | `300` | vdirsyncer is killed after this many seconds. |
| `max_backoff_secs` | integer | `1800` | Upper bound for the wait between retries after failures. |
| `retention_days` | integer | `30` | How long resolved (or superseded) conflicts and synced journal entries are kept; each sync prunes older ones. Undo of a synced entry works until it is pruned. |

`after_edit` is no longer used: it is accepted and ignored, and `tdo sync` and `tdo status` and the TUI
warn about it. Edits sync in batches (`batch_size`, `max_wait_secs`); `tdo sync off` keeps syncs manual.

## `[lists.<id>]`

`<id>` is the collection directory name. A list without an entry gets the defaults below.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `archived` | bool | `false` | Treat the list as archived: like `hidden`, except its completed tasks stay in Done (subject to `show_in_done`). |
| `hidden` | bool | `false` | Hide the list from the list pane, and its tasks from Today, All, Scheduled, Next 3 Days and Done. `:list <name>` still opens it. |
| `show_in_all` | bool | `true` | Include its tasks in the All view. |
| `show_in_today` | bool | `true` | Include its tasks in the Today view. |
| `show_in_done` | bool | `true` | Include its tasks in the Done view. |
| `sort` | `{ field, descending }` | `{ field = "due", descending = false }` | `field` is one of `due`, `start`, `creation`, `title`. |

Hidden and archived lists will not notify once notifications exist (not available yet).

These preferences are a local fallback. They are not stored in CalDAV and do not sync between
devices.

## `[[groups]]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | required | Group heading. |
| `lists` | list of ids | `[]` | Lists shown under the group. |

## Example

```toml
vdir = "~/.local/share/vdirsyncer/tasks"
timezone = "America/New_York"

[lists.bills]
show_in_today = false
sort = { field = "title", descending = false }

[[groups]]
name = "Home"
lists = ["inbox", "personal"]
```
