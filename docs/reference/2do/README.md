# 2Do reference (reverse-engineered)

> **Reference material, not the plan.** What to build is defined by the design spec
> [`docs/superpowers/specs/2026-10-31-terminal-tasks-design.md`](../../superpowers/specs/2026-10-31-terminal-tasks-design.md).
> If this folder and the spec disagree, the spec wins. Use these docs to match 2Do's
> behaviour, formats and encodings exactly.

A behaviour, data-format and look-and-feel reference for 2Do for macOS (v2026.4.6), recovered
from observing the app (its menus, UI text, colours, database schema and feature screenshots) and
checked against one user's Dropbox sync data and, later, CalDAV sessions with 2Do on an iPhone.

## Files

| File | What it covers | Most relevant for |
|---|---|---|
| [01-features.md](01-features.md) | Every core feature and its rules: task types, dates, scheduling, **repeat encoding** (§6), alerts, lists, focus lists, **search syntax**, **sort codes and date-group thresholds** (§12), completion, natural-language input | core library, views |
| [02-data-model.md](02-data-model.md) | 2Do's SQLite schema with column meanings, tag string format, sentinels | understanding fields |
| [03-look-and-feel.md](03-look-and-feel.md) | Layout, row anatomy, editor, pickers, **exact colour palette** (light/dark), typography, icons | TUI styling |
| [04-keyboard-shortcuts.md](04-keyboard-shortcuts.md) | 2Do's full keymap from the main menu | keymap design |
| [05-migrating-your-data.md](05-migrating-your-data.md) | The Dropbox sync format fully decoded (file format, every field) | migration inventory, fixtures |
| [06-sync.md](06-sync.md) | How 2Do syncs (change tracking, conflicts), **CalDAV mapping and `X-2DOAPP-*` / `<2Do Meta>` JSON and `@key:value` metadata** (§1.7, §2.6), Dropbox protocol details | CalDAV interop |
| [`tools/decode_2do_dropbox.py`](../../../tools/decode_2do_dropbox.py) | Read/write 2Do Dropbox sync files (`archive()` writer verified round-trip) | migration inventory |

## Confidence markers

- **[obs]** seen directly in the app or the user's data (code, schema text, UI strings, menus, colours)
- **[inf]** inferred from several observations; very likely but not proven
- **[?]** not resolved; verify in the spec's phase-0 compatibility gate

## Notes

- Vendor promotional screenshots are © Beehive Innovations and are excluded from Git.
  Descriptions record observations from those images without redistributing them.
