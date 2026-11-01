# 2Do reference (reverse-engineered)

> **Reference material, not the plan.** What to build is defined by the design spec
> [`docs/superpowers/specs/2026-10-31-terminal-tasks-design.md`](../../superpowers/specs/2026-10-31-terminal-tasks-design.md).
> If this folder and the spec disagree, the spec wins. Use these docs to match 2Do's
> behaviour, formats and encodings exactly.

A behaviour, data-format and look-and-feel reference for 2Do for macOS (v2026.4.6), recovered
from the shipped app with static analysis (Ghidra via REA, the SQLite schema embedded in
`TodoDAOFramework`, the main menu nib, `Localizable.strings`, `Assets.car` colours, the app's own
feature screenshots) and checked against one user's Dropbox sync data. The app was not run.

## Files

| File | What it covers | Most relevant for |
|---|---|---|
| [01-features.md](01-features.md) | Every core feature and its rules: task types, dates, scheduling, **repeat encoding** (§6), alerts, lists, focus lists, **search syntax**, **sort codes and date-group thresholds** (§12), completion, natural-language input | core library, views |
| [02-data-model.md](02-data-model.md) | 2Do's SQLite schema with column meanings, tag string format, sentinels | understanding fields |
| [03-look-and-feel.md](03-look-and-feel.md) | Layout, row anatomy, editor, pickers, **exact colour palette** (light/dark), typography, icons | TUI styling |
| [04-keyboard-shortcuts.md](04-keyboard-shortcuts.md) | 2Do's full keymap from the main menu | keymap design |
| [05-migrating-your-data.md](05-migrating-your-data.md) | The Dropbox sync format fully decoded (file format, every field) | migration inventory, fixtures |
| [06-sync.md](06-sync.md) | How 2Do syncs (change tracking, conflicts), **CalDAV mapping and `X-2DOAPP-*` / `<2Do Meta>` JSON and `@key:value` metadata** (§1.7, §2.6), Dropbox protocol details | CalDAV interop |
| [07-reverse-engineering-notes.md](07-reverse-engineering-notes.md) | Low-level notes: architecture, IPC, automation socket, **CalDAV static-analysis evidence and limits** (§7) | reproducing analysis, CalDAV interop |
| [`tools/decode_2do_dropbox.py`](../../../tools/decode_2do_dropbox.py) | Read/write 2Do Dropbox sync files (`archive()` writer verified round-trip) | migration inventory |

## Confidence markers

- **[obs]** seen directly in the app or the user's data (code, schema text, UI strings, menus, colours)
- **[inf]** inferred from several observations; very likely but not proven
- **[?]** not resolved; verify in the spec's phase-0 compatibility gate

## Notes

- Vendor promotional screenshots are © Beehive Innovations and are excluded from Git.
  Descriptions record observations from those images without redistributing them.
- Paths such as `0x5aada8` are addresses in the arm64 slices of 2Do 2026.4.6 binaries; they only
  matter if someone re-runs the analysis.
