# Twydo documentation

## Using Twydo

| Page | What it covers |
|---|---|
| [install.md](install.md) | Installing or building `tdo`, trying it on made-up data, pointing it at your tasks |
| [tui.md](tui.md) | The screen, every key, the edit card, and what a refused write means |
| [tasks.md](tasks.md) | How tasks behave: checklists and projects, repeats, views and date groups, alerts |
| [sync.md](sync.md) | Setting up vdirsyncer and `tdo sync`, conflicts, emptied lists, known limits |
| [compatibility.md](compatibility.md) | Using Twydo alongside 2Do on the iPhone: moving from Dropbox, verified mode, what is verified |
| [config.md](config.md) | Every configuration key |

## How it works

| Page | What it covers |
|---|---|
| [architecture.md](architecture.md) | The daemon, the crates, the write path, the operation journal and recovery, locking |
| [data-model.md](data-model.md) | What a task looks like on disk, 2Do's metadata, how families and repeats are stored |
| [protocol.md](protocol.md) | The daemon's socket protocol, for people writing their own client |
| [testing.md](testing.md) | Running the tests, coverage ownership and the daemon migration cleanup |

## Background

| Folder | What it holds |
|---|---|
| [reference/](reference/README.md) | 2Do's observed formats and operation evidence, known failures, unresolved questions and capture tools |
| [superpowers/](superpowers/) | The design spec and the implementation plans Twydo was built from |
| [demo/](demo/) | The VHS tape and script that record the README demo (`mise run demo`) |
