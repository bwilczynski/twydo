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
It needs a Python interpreter with **vdirsyncer 0.21.x** installed. Set
[`[sync].list_python`](config.md#sync) if that interpreter differs from `python3` (for
example, a virtual environment's `bin/python`). The helper checks its adapter before
network access and uses the existing vdirsyncer pair, credentials and TLS settings.
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

1. **Get your tasks into a vdir.** Twydo reads a local folder that
   [vdirsyncer](https://vdirsyncer.pimutils.org/) keeps in step with your CalDAV server. If you use
   2Do, it must sync over CalDAV first: see [compatibility.md](compatibility.md) for moving from
   Dropbox, and [sync.md](sync.md) for setting up vdirsyncer.
2. **Point Twydo at it.** Create `~/.config/twydo/config.toml` (or under `$XDG_CONFIG_HOME`):

   ```toml
   vdir = "~/.local/share/vdirsyncer/tasks"
   timezone = "Europe/Berlin"   # optional; defaults to the system zone
   ```

   All keys: [config.md](config.md). Without a config, `tdo` prints how to create one; `--vdir <path>` works without
   one.
3. **Check your data.** `tdo check` reports each kind of repeat in your data, whether Twydo has
   verified it against 2Do, and any task it cannot read.
4. **Open it.** `tdo`. Keys: [tui.md](tui.md).
5. **Sync.** Add `[sync]` to the config so `tdo sync` and the TUI's `:sync` run vdirsyncer:
   [sync.md](sync.md).

## The daemon

`tdo` starts its daemon the first time it needs it (the TUI, `tdo op`, `tdo sync`,
`tdo conflicts resolve`), in the background; it keeps running when you close the TUI, syncs, and
is started again on demand. Its log is `$XDG_STATE_HOME/twydo/daemon-<id>.log`. After upgrading
`tdo` or changing `timezone`, `vdir` or `[sync]` in the config, run `tdo daemon stop`; the next
`tdo` starts a new one. A service unit that starts it at login comes in a later release.
Setups that must not share the usual daemon (a test setup) set `TWYDO_SOCKET` next to
`XDG_CONFIG_HOME`. For writing your own client, see [protocol.md](protocol.md).

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
stderr); 2 is a usage, config or IO error. `tdo export-json` dumps every task as JSON.
