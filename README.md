# Twydo

A keyboard-driven terminal task manager for Linux and macOS that works with **2Do on your
iPhone**. Tasks stay in standard iCalendar on your own CalDAV server, so the 2Do app keeps
syncing the same lists, projects, checklists and repeats while you work from the terminal.
The command is `tdo`. Free and open source (MIT).

![Twydo in a terminal: the Today and All views, completing a task, a checklist, editing a date, and the help](docs/demo/tui.gif)

## Status

Twydo is young: it has been tested against one iPhone running 2Do. It is careful rather than
complete: it only writes changes that have been checked against 2Do on the iPhone, and refuses
the rest with a reason, so the phone never gets something it would misread.

**Works today**

- Views: Today, All, Scheduled, Done, Next 3 Days, your lists, grouped by date like 2Do
  (Overdue, Today, Tomorrow, Next 7 Days, …)
- Complete, reopen, delete, move to another list, undo and redo
- Change dates with single keys (today, tomorrow, ±1 day, ±1 week) or type them (`fri 5pm`,
  `3d`, `15th`); edit titles, notes (in your `$EDITOR`) and a repeat's interval
- Checklists, projects and recurring projects, acting on the whole family as 2Do does
- Every repeat Twydo creates (daily, weekly, set weekdays, monthly on any day or the Nth
  weekday, yearly, from completion, ending after a count), including start dates, due times and
  missed occurrences, completed the way 2Do completes them
- Sync through vdirsyncer with conflict handling: when the phone and Twydo change the same task,
  both versions are kept until you choose
- Crash-safe writes: every change is journaled and recovered after an interruption
- `tdo op` to script every change, `tdo export-json` to read everything

**Not yet**

- Natural-language input on the new row (planned: `Pay rent fri every month @Bills`, typed
  with the title); today the title goes in the row and dates and repeats in the card
- Desktop notifications for alerts, and editing alerts
- Manual ordering, and creating or renaming lists
- Setting or removing daily, yearly and from-completion repeats, and repeat changes on projects
  and checklists, in verified mode: Twydo can write them, but refuses until they are checked on
  the phone (weekly and monthly repeats, and type changes except empty project to checklist,
  are allowed)
- Completing a repeat that ends on a date, or a recurring project that ends or repeats from
  completion; editing an existing from-completion task; reopening the last occurrence of an
  ended repeat (undo works)
- Repeats on the last day of the month, the last weekday and the Nth weekend day: the iPhone
  (2Do 3.91) misreads them, so Twydo shows them but never writes them
- Prebuilt releases (build from source for now)
- macOS is expected to work but is not tested yet

Details: [what is verified](docs/compatibility.md#what-is-verified-today).

## How it works with 2Do

```
 2Do on iPhone ──CalDAV──▶ your CalDAV server ◀──CalDAV── vdirsyncer ──▶ local folder ◀── tdo
```

2Do on the iPhone syncs with a CalDAV server (its Dropbox sync is not supported).
[vdirsyncer](https://vdirsyncer.pimutils.org/) mirrors the server into a local folder of `.ics`
files, and `tdo` reads and writes those files, changing only what each operation is about and
keeping everything else, including 2Do's own metadata, byte for byte.
[More on living alongside 2Do](docs/compatibility.md).

## Install and quick start

```sh
mise use -g github:bwilczynski/twydo   # prebuilt tdo for macOS and Linux

# try it on made-up tasks (from a clone)
python3 tools/demo_vdir.py /tmp/twydo-demo && tdo --vdir /tmp/twydo-demo
```

Then point it at your own tasks and set up sync: [install.md](docs/install.md),
[sync.md](docs/sync.md). Press `?` in the TUI for every key.

## Documentation

| Page | |
|---|---|
| [Install](docs/install.md) | install with mise or build, first run, where things live |
| [The TUI](docs/tui.md) | the screen and every key |
| [How tasks behave](docs/tasks.md) | checklists vs projects, repeats, views and date groups |
| [Sync](docs/sync.md) | vdirsyncer setup, `tdo sync`, conflicts |
| [Living alongside 2Do](docs/compatibility.md) | moving from Dropbox, verified mode, what is verified |
| [Configuration](docs/config.md) | every key |
| [Architecture](docs/architecture.md) | the write path, journal and recovery, locking |
| [Data model](docs/data-model.md) | what a task looks like on disk, 2Do's metadata |
| [Reference](docs/reference/README.md) | 2Do's behaviour and formats as observed, test session logs |

## Contributing

```sh
mise run lint    # clippy and rustfmt
mise run test    # Rust tests, tool tests, documentation links
mise run demo    # re-record the README demo (needs vhs, ttyd, ffmpeg)
```

Commits and pull request titles follow [Conventional Commits](https://www.conventionalcommits.org/).
`feat` and `fix` changes open a release PR (version bump and `CHANGELOG.md`); merging it tags the
version and publishes the binaries to a GitHub release.

Test data is synthetic. Never commit real tasks, list names or account details; fixtures come
from a separate test account ([AGENTS.md](AGENTS.md) has the full rules).

## License

[MIT](LICENSE). Twydo is an independent project, not affiliated with 2Do or Beehive Innovations.
