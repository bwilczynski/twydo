# Twydo configuration

File: `$XDG_CONFIG_HOME/twydo/config.toml`, else `~/.config/twydo/config.toml` (Linux and macOS
alike). An empty `XDG_CONFIG_HOME` counts as unset. Unknown keys are an error and the message
names the key. Twydo only reads this file.

## Top-level keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `vdir` | path | required | Root of the vdir: one subdirectory per list, one `.ics` per task. A leading `~` or `~/` expands to `$HOME`. |
| `timezone` | IANA name | system zone, else UTC | Zone used for "today" and floating times. |
| `week_start` | `"mon"`..`"sun"` (or full names, any case) | `"mon"` | First day of the week. |
| `morning_time` | `"HH:MM"` | `"09:00"` | Time used for date-only tasks that need one (e.g. reminders). |
| `default_list` | list id | none | List new tasks go to. |
| `state_dir` | path | `$XDG_STATE_HOME/twydo`, else `~/.local/state/twydo` | Where Twydo keeps its write journal and other state. A leading `~` expands to `$HOME`. |

## `[writes]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `mode` | `"verified"` or `"test"` | `"verified"` | `verified` runs only operations the compatibility matrix marks as passing. `test` also runs unverified operations. |

`test` must only be used with a config that points at the isolated test account, never at
production data: unverified operations may write something 2Do on the iPhone cannot read. Every
`tdo op` run in `test` mode prints `tdo: test write mode: unverified operations are enabled` to
stderr.

`tdo op` (add, edit, complete, reopen, delete, move, undo, check, journal, recover, abandon) is
the scriptable write interface. It prints the operation id and exits 0; exit 1 means the
operation was blocked, stale or stuck (reason on stderr); exit 2 is a usage, config or IO error.
`tui`, `export-json`, `check` and `op` first roll forward unfinished journal entries in
`state_dir`.

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

Hidden and archived lists do not notify (spec section 7; notifications arrive in Plan 2).

These preferences are a local fallback. They are not stored in CalDAV and do not sync between
devices (spec section 5).

## `[[groups]]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | required | Group heading. |
| `lists` | list of ids | `[]` | Lists shown under the group. |

## Example

```toml
vdir = "~/.local/share/vdirsyncer/tasks"
timezone = "America/New_York"
week_start = "mon"
morning_time = "09:00"
default_list = "inbox"

[lists.bills]
show_in_today = false
sort = { field = "title", descending = false }

[[groups]]
name = "Home"
lists = ["inbox", "personal"]
```
