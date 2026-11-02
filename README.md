# Twydo: a terminal task app to replace 2Do

A personal, keyboard-driven (vim-style) terminal task manager for **Omarchy (Linux/Hyprland)** and
**macOS**. Tasks are stored as standard iCalendar (`VTODO`) and synced through CalDAV, so the
2Do iPhone app keeps working against the same data. Command: `tdo`.

Status: **implementation in progress** (plans in `docs/superpowers/plans/`). The phase-0 CalDAV
compatibility gate has passed (see [`docs/phase0/README.md`](docs/phase0/README.md)). Done so far:
a read-only foundation (`export-json`, `check`, the TUI), the writable core (`tdo op`), the sync
runner with conflict handling (`tdo sync`, [`docs/sync.md`](docs/sync.md)), editing in the TUI
([`docs/tui.md`](docs/tui.md)), and checklists, projects and recurring projects plus repeats with
a due time, start date or missed occurrence (Plan 6). Not yet: adding tasks in the TUI, the
notifier, manual order and list management. Writes the compatibility matrix has not verified
(`compat/matrix.toml`) are refused with a reason.

## Start here

| Read | Why |
|---|---|
| [`AGENTS.md`](AGENTS.md) | Handoff instructions for the implementing agent |
| [`docs/superpowers/specs/2026-10-31-terminal-tasks-design.md`](docs/superpowers/specs/2026-10-31-terminal-tasks-design.md) | **The spec**: what to build, decisions, contracts, tests, phases |
| [`docs/design/tui-layout.md`](docs/design/tui-layout.md) | The chosen screen layout |
| [`docs/reference/2do/`](docs/reference/2do/README.md) | Reverse-engineered 2Do behaviour, formats and encodings (reference) |
| [`docs/phase0/README.md`](docs/phase0/README.md) | Phase-0 compatibility gate checklist (user and agent steps) |
| [`tools/decode_2do_dropbox.py`](tools/decode_2do_dropbox.py) | Read/write 2Do's Dropbox sync files (migration inventory) |

## Layout

```
README.md, AGENTS.md, CLAUDE.md
docs/
  superpowers/specs/   design spec (source of truth)
  design/              chosen UI layout
  reference/2do/       2Do reverse-engineering reference (01–07)
  phase0/              phase-0 compatibility gate checklist
  config.md            tdo configuration reference
crates/
  twydo-core/          library: data model, storage, views
  tdo/                 binary: CLI and TUI
compat/                phase-0 compatibility matrix
contrib/               example vdirsyncer configuration
tools/                 helper scripts
```
