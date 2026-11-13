# Install

Twydo runs on Linux and macOS. It is developed and used on Linux; **macOS is not tested yet**.

## Prebuilt binaries

Every [GitHub release](https://github.com/bwilczynski/twydo/releases) has `tdo` for macOS (Apple
Silicon and Intel) and Linux (x86_64 and arm64). Install it with [mise](https://mise.jdx.dev/);
the command is the same everywhere:

```sh
mise use -g github:bwilczynski/twydo    # latest release
tdo --version
```

mise picks the archive for your OS and CPU and puts `tdo` on your `PATH`; `mise up` moves to a
newer release.

## Build from source

You need a Rust toolchain; the version is pinned in `rust-toolchain.toml` (and `mise.toml` if you
use mise).

```sh
cargo install --git https://github.com/bwilczynski/twydo tdo
```

or, from a clone:

```sh
git clone https://github.com/bwilczynski/twydo
cd twydo
cargo install --path crates/tdo
```

Either puts the `tdo` binary in `~/.cargo/bin`.

The collection helper is bundled in `tdo`; it does not need a source checkout at runtime.
It needs a Python interpreter with **vdirsyncer 0.21.x** installed. By default Twydo uses
vdirsyncer's own interpreter, read from the shebang of `[sync].command` (system packages,
Homebrew and pipx all name it there), else `python3`. Set
[`[sync].list_python`](config.md#sync) when neither is right (for example, `command` is a
wrapper script). The helper checks its adapter before
network access and uses the vdirsyncer config Twydo generates from `[sync]`.
Task sync still uses `[sync].command` independently.

## Try it without your data

From a clone, generate a vdir of made-up tasks with today's dates and open it:

```sh
python3 tools/demo_vdir.py /tmp/twydo-demo
tdo --vdir /tmp/twydo-demo
```

This is the data the demo at the top of the README was recorded with. Change it freely;
regenerate it into an empty folder to start over.

## Use it with your tasks

1. **Set up.** Run `tdo config init`. It asks where the vdir goes (Enter takes
   `~/.local/share/twydo/vdir`) and whether to sync with a CalDAV server; if so, the server URL,
   the username and a command that prints the password. It writes
   `~/.config/twydo/config.toml` (or under `$XDG_CONFIG_HOME`), creates the vdir and checks the
   setup. Syncing needs [vdirsyncer](https://vdirsyncer.pimutils.org/) 0.21.x installed; Twydo
   writes its config itself ([sync.md](sync.md)). If you use 2Do, it must sync over CalDAV first:
   see [compatibility.md](compatibility.md) for moving from Dropbox. All keys:
   [config.md](config.md). Without a config, `tdo` prints how to create one; `--vdir <path>`
   works without one.
2. **Optional settings.** Add keys such as `timezone = "Europe/Berlin"` (defaults to the system
   zone) to the file by hand.
3. **Check your data.** `tdo check` reports each kind of repeat in your data, whether Twydo has
   verified it against 2Do, and any task it cannot read.
4. **Open it.** `tdo`. Keys: [tui.md](tui.md).
5. **Sync.** If you set up sync in step 1, `tdo sync` and the TUI's `:sync` run vdirsyncer:
   [sync.md](sync.md). To add or change it later, run `tdo config init` again.

## The daemon

`tdo` starts its daemon the first time it needs it (the TUI, `tdo op`, `tdo sync`, `tdo check`,
`tdo export-json`, `tdo status`, `tdo conflicts`), in the background; it keeps running when you
close the TUI, syncs, and is started again on demand. Its log is
`$XDG_STATE_HOME/twydo/daemon-<id>.log`. Setups that must not share the usual daemon (a test
setup) set `TWYDO_SOCKET` next to `XDG_CONFIG_HOME`. For writing your own client, see
[protocol.md](protocol.md).

### Start it at login

A daemon started on demand is detached: it keeps syncing after you close the TUI, until you stop
it, it crashes or the machine reboots. A service unit starts it at login and restarts it after a
failure. `tdo daemon install` needs a configuration (a config file or `--vdir`) and writes that
login service:

```sh
tdo daemon install
tdo daemon status
```

- **Linux:** a systemd user unit, `~/.config/systemd/user/twydo.service` (under
  `$XDG_CONFIG_HOME` when that is set), enabled and started with `systemctl --user`. A setup
  with its own `TWYDO_SOCKET` gets its own unit, `twydo-<socket-id>.service`.
- **macOS:** a launchd agent, `~/Library/LaunchAgents/app.twydo.daemon.plist` (with a
  `.<socket-id>` suffix for another socket), bootstrapped into your login session. macOS is not
  tested yet.

The unit runs `tdo daemon`, passing `--vdir` and `--socket` where they are not the defaults, and
sets `PATH` (the daemon runs vdirsyncer and the conflict hook) and the `XDG_*` directories you
have set. It restarts the daemon when it exits, one second later. A daemon that exits because
another one already serves the vdir is not restarted under systemd (exit status 10); launchd
cannot exclude one exit code, so there a losing daemon is retried every 10 seconds until it wins
or the unit is removed. `install` stops a detached daemon first, so that only happens in a race.
A stop waits up to 90 seconds for the daemon to finish. Reference copies of both units are in
`contrib/`.

`tdo` must have a path that survives upgrades. `install` looks for it on `PATH`, then at
`${MISE_DATA_DIR:-~/.local/share/mise}/shims/tdo`, and skips mise's versioned install directories,
which disappear on `mise up`. If it finds only such a path it refuses: run `mise reshim`, or pass
`--exe <path>`. Running `install` again rewrites the unit (after the path moved, say) and says it
was already installed. If it finds the old `twydo-sync.timer` (or the macOS `app.twydo.sync`
agent), it disables it and says so; the files stay where they are.

With a unit installed, a `tdo` that finds no daemon starts it through the service manager, and
`tdo daemon stop` stops it through the manager too, at once (the manager would otherwise bring it
straight back); a daemon that still answers afterwards, because it was started detached, is asked
to stop once idle. Without a unit, `tdo daemon stop` asks the daemon to stop once it is idle and
waits until it is gone, and prints `no daemon is running` when nothing runs.

While no unit is installed, the TUI says once per run `daemon won't start at login; run tdo
daemon install`. Turn that off with `[daemon] unit_warning = false` ([config.md](config.md#daemon)).

`tdo daemon status` says whether the unit is installed and enabled and whether a daemon answers.
`tdo daemon uninstall` stops the daemon, disables the unit and removes its file; the daemon then
starts on demand again.

### After an upgrade

After upgrading `tdo` (`mise up`) the old daemon keeps running until the next `tdo` that is newer
than it asks it to stop once it is idle and starts its own version, through the unit when there
is one. After changing `timezone`, `vdir`, `[writes]` or `[sync]` in the config, run
`tdo daemon stop`; the next `tdo` (or the unit's restart) starts a new daemon.

## Where Twydo keeps things

| What | Where |
|---|---|
| Config | `$XDG_CONFIG_HOME/twydo/config.toml`, else `~/.config/twydo/config.toml` (also on macOS) |
| State: operation journal, sync status, conflicts | `$XDG_STATE_HOME/twydo`, else `~/.local/state/twydo`; one folder per vdir ([architecture.md](architecture.md#the-operation-journal)) |
| The daemon's socket | `$XDG_RUNTIME_DIR/twydo/tdo.sock` (`$TMPDIR/twydo/tdo.sock` on macOS); `--socket` or `TWYDO_SOCKET` change it |
| Your tasks | only in the vdir; Twydo has no other copy |

## Scripting

`tdo op` makes the same changes as the TUI from the command line (`add`, `edit`, `complete`,
`reopen`, `delete`, `move`, `undo`, and `check` to see what is allowed for a task). It prints the
operation id and exits 0; 1 means the operation was refused, stale or stuck (the reason on
stderr); 2 is a usage, config or IO error. `tdo export-json` dumps every task as JSON (the vdir as the daemon last read it, without
edits still queued behind a sync).
