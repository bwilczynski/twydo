# Own the CalDAV settings and guide setup: `tdo config init` and `tdo config check` (design)

Date: 2026-11-12
Status: draft, awaiting review
Issue: [#88](https://github.com/bwilczynski/twydo/issues/88)

## Why

Today a user who wants sync keeps two configs that must agree: Twydo's `[sync]` (`pair`,
`local_storage`, `vdirsyncer_config`) and a hand-written vdirsyncer file holding the pair, the
storages, the conflict hook and `metadata`. Getting that second file wrong is easy and costly:
a wrong hook, `a wins` instead of the hook, a missing `metadata`, a storage name that does not
match `local_storage`. A new user also has to read `sync.md` and an example file before
anything works.

After this change:

- Twydo's `config.toml` holds the CalDAV connection (server URL, username, password command,
  optional CA file). Twydo generates the vdirsyncer config itself, with everything that must not
  vary fixed by Twydo.
- `tdo config init` takes a new user from nothing to a working config by answering prompts, each
  with a default that Enter accepts. Sync is optional.
- `tdo config check` reports, step by step, whether this machine is ready to sync.

This also prepares #86 (replacing the list helper by Rust code that reads Twydo's config), but
does not start it.

## Decisions (made by the user)

| Question | Decision |
|---|---|
| How setup is guided | A `tdo config` subcommand modelled on unifictl's `config init` / `config check` |
| vdir prompt | Shows a default; Enter accepts it |
| Sync | Optional. Declining it writes no `[sync]` table: no automatic sync |
| Existing setups | No migration step. The user runs `tdo config init` on one machine and shares the resulting `config.toml` to the others through dotfiles (stow) |
| vdirsyncer status | Not carried over. Twydo uses fixed names and its own `status_path`; vdirsyncer discovers and builds its status from scratch. The user syncs every machine before upgrading |
| Re-running `init` on an existing config | Reuses the values already in the config as prompt defaults |
| CA file | Optional, only for a server whose certificate is not trusted. `init` does not probe the server and does not ask for it |
| Network checks during setup | None. Neither `init` nor `check` contacts the server |
| `config check` | Checks that vdirsyncer is installed in a supported version |

## Configuration

### New `[sync]` keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `url` | string | required when `[sync]` is present | CalDAV server URL (the collection home the NAS gives the account). |
| `username` | string | required | CalDAV username. |
| `password_command` | array of strings | required | Command whose first line of output is the password, e.g. `["pass", "show", "twydo/caldav"]`. Written as vdirsyncer's `password.fetch = ["command", …]`. No literal password key exists. |
| `ca_file` | path | none | CA certificate (PEM) to verify a server whose certificate is not otherwise trusted; written as vdirsyncer's `verify`. Leading `~` expands. |

Kept unchanged: `command`, `list_python`, `metasync`, `batch_size`, `max_wait_secs`,
`timeout_secs`, `max_backoff_secs`, `retention_days`, and the retired-and-warned `after_edit`.

`[sync]` without `url`, `username` and `password_command` is a config error naming the missing
key and pointing at `tdo config init`. (An empty `[sync]` table no longer means "sync on with
defaults".)

### Removed keys

`pair`, `local_storage` and `vdirsyncer_config` are removed. A config that still sets any of
them fails to load with an error naming the key and saying to run `tdo config init` (which
rewrites the file without them). They are not silently ignored: the user's stowed config would
otherwise keep pointing at the old hand-written vdirsyncer file without anyone noticing.
*(Builder's call made in the conversation: error, not warning.)*

`contrib/vdirsyncer/config.example` is deleted.

Every strict consumer of `Config` (the daemon, `tdo config check`, every client command that
loads it) fails on a removed key the same way it fails on any other config error today: the
daemon does not start, and the command prints the error with the `tdo config init` hint. Only
`init` reads leniently (a raw `toml_edit` document, from which it takes defaults); there is no
`Config::load_lenient`. *(My call: the user's workflow is "upgrade, run `tdo config init`",
which never needs the daemon, so a hard stop until then is clear and cheap.)*

### Versioning

Removing keys is a breaking config change: the commit that removes them is marked breaking
(`feat(config)!:` with a `BREAKING CHANGE:` footer), so release-plz bumps to `0.3.0`. The
protocol schema (`schema/*.openrpc.json`) does not describe `config.toml`; if a snapshot does
carry `[sync]` fields, it is regenerated as part of the change. *(My call.)*

### Portability

`config.toml` must work unchanged on every machine it is stowed to: `init` writes `~`-relative
paths where the user's answer is under `$HOME`, and nothing machine-specific (no absolute
binary paths, no state paths). Everything machine-specific lives in the state directory.

## The generated vdirsyncer config

Location: under the per-vdir state directory that the daemon already binds
(`store::bind_vdir_state(cfg.state_dir(), cfg.vdir)`, written `<vstate>` below):
`<vstate>/vdirsyncer/config`, with `status_path` = `<vstate>/vdirsyncer/status/`. No new state
layout. The first line is a comment saying the file is generated by Twydo and is overwritten.
`tdo config check` must not bind state as a side effect: it uses
`store::existing_vdir_state` and, if the vdir is not bound yet, renders the config in memory
only and reports "written on first sync". *(My call.)*

Fixed by Twydo, never configurable:

```ini
# Generated by Twydo from config.toml; do not edit, it is overwritten.
[general]
status_path = "<vstate>/vdirsyncer/status/"

[pair twydo]
a = "twydo_remote"
b = "twydo_local"
collections = ["from a", "from b"]
metadata = ["displayname", "color"]
conflict_resolution = ["command", "tdo", "conflicts", "hook"]

[storage twydo_local]
type = "filesystem"
path = "<vdir>/"
fileext = ".ics"

[storage twydo_remote]
type = "caldav"
url = "<url>"
username = "<username>"
password.fetch = ["command", <password_command…>]
# verify = "<ca_file>"   (only when ca_file is set)
```

- Side order is `a` = remote, `b` = local, as in the existing example config.
- The conflict hook calls `tdo` by name, as today: `tdo` must be on the `PATH` vdirsyncer runs
  with (the daemon's), unchanged from the current requirement.
- Pair and storage names are Rust constants; the sync runner, the conflict hook and the list
  helper request use them instead of the removed `pair` / `local_storage` keys.
- Every vdirsyncer and list-helper invocation gets `VDIRSYNCER_CONFIG` pointing at this file.
- Values are escaped correctly for vdirsyncer's config syntax (JSON-style quoted strings).
- The file is regenerated before each vdirsyncer or helper run if its content would change
  (write to a temp file and rename; leave it alone when identical). This covers both "on start"
  and "when the config changes" without a separate trigger. *(Builder's call.)*
- Rediscovery: the generator (in `twydo-core`, called by the sync runner and the list-helper
  launcher) compares the new content with the file on disk before renaming. If the file is
  missing or differs in any way, it writes `<vstate>/sync/rediscover` (the existing marker
  `sync/run.rs` already honours) before replacing the file. Any change counts, including
  `password_command` and `ca_file`: a discovery is cheap and diffing individual keys is not worth
  it. A first run on an upgraded machine has no generated file, so it discovers too. *(My call.)*
- List helper: the request keeps its current shape. `pair` / `local_storage` carry the
  constants, and the config path is always the generated file, so `helper.py` needs no protocol
  change. Its operation-identity digest covers the storage settings and `status_path`, so it
  changes on upgrade; an in-flight list operation from before the upgrade is reported as the
  helper reports a digest mismatch today. That is accepted: the user syncs before upgrading, so
  none should be in flight. *(My call.)*
- Password command: vdirsyncer runs it, not Twydo, except in `config check` (below). Each word
  of `password_command` is written as its own JSON-quoted string.
- `status_path` is inside the state directory, never inside the vdir; the existing helper check
  for that stays.

### Upgrade behaviour (no migration)

On the first sync after upgrading, the old hand-written vdirsyncer file and its status are no
longer used (they are left in place, untouched). vdirsyncer discovers the collections and syncs
with an empty status. Items identical on both sides are recorded as in sync; this is safe
because the user syncs every machine before upgrading. Any item that does differ is handled by
the conflict hook as usual. In-flight list-helper operation records whose identity digest no
longer matches are reported as the helper reports them today; no special handling. Docs say
"sync before upgrading" and how to remove the old vdirsyncer file and status afterwards.

## `tdo config init`

```
tdo config init [--vdir PATH] [--no-sync | --url URL --username USER
                 --password-command CMD…] [--ca-file PATH]
```

Works on the config file `tdo` would load (`Config::path()`, honouring `XDG_CONFIG_HOME`). It
does not need, start or talk to the daemon.

Flow (prompts and progress on stderr):

1. Read the existing file if there is one, leniently: an old config with removed keys must still
   supply defaults. Unknown or removed keys never make `init` fail.
2. Prompt, each with a default shown in brackets that Enter accepts:
   - `vdir` — default: the existing value, else `~/.local/share/twydo/vdir`.
   - `Sync with a CalDAV server? [y/N]` — default yes if the existing config has `[sync]`, else no.
   - If yes: server URL, username, password command (each defaulting to the existing value
     when present). The password command is typed as one line and split shell-style (the
     `shell-words` crate; an unbalanced quote re-prompts).
   - No prompt for `ca_file`; an existing value is kept, `--ca-file` sets it.
3. Write the file:
   - Preserve everything else in an existing file (other tables, other `[sync]` keys, comments,
     ordering) by editing it with `toml_edit`; drop `pair`, `local_storage` and
     `vdirsyncer_config`; with sync declined, remove `[sync]` entirely.
   - A new file contains only the keys answered, no defaults spelled out.
   - The config may be a symlink into a dotfiles checkout (stow). Write through it: replace the
     symlink's target atomically (temp file in the target's directory, then rename), leaving the
     symlink in place.
   - Create the parent directory if missing. The file holds no secrets; normal permissions.
   - Print `wrote <path>`.
4. Create the vdir directory if it does not exist.
5. Run `tdo config check` and print its report; exit with its code.
6. If a daemon is running for this config, say that vdir/sync changes take effect after
   `tdo daemon stop` (the next `tdo` starts a new one).

Non-interactive (stdin not a TTY): every prompt takes its flag, else the existing value, else
the default; a required sync value with none of those is an error naming the flag, and nothing
is written. EOF or Ctrl-C during prompts writes nothing.

## `tdo config check`

Checks only this machine; no network. Each step is pass / fail / skip with a one-line detail
and, on failure, a hint. A failure skips the steps that depend on it. Exit 1 on any failure.

| # | Step | Pass | Fail / skip |
|---|---|---|---|
| 1 | config | path of the file in effect | load error text (including a removed key), hint `run tdo config init` |
| 2 | vdir | the path exists and is a directory | hint to create it or rerun `init` |
| 3 | sync | `[sync]` present | not a failure: "sync off", steps 4–7 skip |
| 4 | vdirsyncer | `<command> --version` runs and reports 0.21.x | not found / unsupported version, hint names `[sync].command` |
| 5 | list helper | `list_python` imports vdirsyncer and its version is 0.21.x | hint names `[sync].list_python` |
| 6 | password | `password_command` exits 0 and prints a non-empty first line (never shown) | its exit status and the last line of its stderr; stdout is never shown. Run with `tdo`'s own environment and stdin inherited (so `pass`/`gpg` can reach their agent or pinentry), killed after 60 s (*my call*) |
| 7 | generated config | the generated vdirsyncer config can be written to the state directory; shows its path | the IO error |
| 8 | ca file | when `ca_file` is set, it exists and is readable | missing/unreadable |

`tdo check` (the compatibility gate check) is unrelated and unchanged.

## Docs

- `docs/config.md`: new and removed `[sync]` keys; `tdo config init` / `check`.
- `docs/sync.md`: setup becomes "run `tdo config init`"; the generated file, its location and
  that it must not be edited; that running `vdirsyncer` by hand now needs
  `VDIRSYNCER_CONFIG=<generated path>` (and still must not race the daemon); `ca_file` for a
  NAS with a private CA; upgrade notes (sync first, old file left unused).
- `docs/install.md`: first-run steps lead with `tdo config init`.
- `docs/README.md` / links stay valid (`python3 tools/check_links.py`).

## Testing

- Generation: golden output for a full `[sync]`, with and without `ca_file`; escaping of quotes
  and backslashes in URL/username/command; regenerate-only-on-change; a changed URL triggers
  rediscovery; an empty status dir discovers.
- Config: removed keys give the named error; missing required sync keys give the named error;
  `[sync]` absent means sync off.
- `init` with a scripted prompter: fresh file; reuse of existing values as defaults; old config
  with removed keys is rewritten without them and with unrelated tables and comments preserved;
  sync declined removes `[sync]`; writing through a symlink keeps the symlink; non-TTY with
  missing required value writes nothing.
- `check`: each step's pass and failure, with fake `vdirsyncer` / `list_python` / password
  scripts; sync off skips 4–8.
- Existing sync end-to-end tests (`crates/tdo/tests/sync_e2e.rs`) run real vdirsyncer against a
  filesystem "server". With the remote storage now always CalDAV, they need a way to point the
  generated remote at a filesystem path. This must be a test-only seam that is not a documented
  user setting (for example a hidden, schema-excluded key or an environment variable read by
  the generator). *(Left to the builder: pick the seam; it must not reintroduce a user-facing
  hand-written vdirsyncer config.)* The seam swaps only the `twydo_remote` storage block; the
  pair, hook, `metadata`, side order and local storage stay as generated, and the tests keep
  putting the built `tdo` on `PATH` for the hook. The list tests that use a synthetic `connection.json` via
  `vdirsyncer_config` need the same kind of seam.
- The schema check (`tools/refresh_released_schema.sh` / `schema/`) is updated for the new keys.

## Non-goals

- Probing the server (TLS, login, collections) from `init` or `check`.
- Migrating vdirsyncer status, pair names or the old vdirsyncer file.
- Reading values out of an old hand-written vdirsyncer config.
- Replacing vdirsyncer or the list helper (#86).
- Literal passwords in `config.toml`.
- `config show`, `config set`, editing single keys.

## Anti-goals

- A generated config that differs from the old example in anything sync-relevant other than
  names and `status_path` (hook, metadata, collections, side order).
- A `config.toml` that only works on the machine that wrote it.
- `init` destroying unrelated settings (list preferences, groups, writes mode) or replacing a
  stow symlink with a plain file.

## Left to the builder

- The test seam for a filesystem remote (above).
- Prompt wording; module layout (`crates/tdo/src/cli/config.rs`, generator in `twydo-core`).
- Whether `password_command` accepts a string as well as an array (array is required to work).
