# Task type indicator (#28)

## Intent

Today a row's type (task, project, checklist) is either hidden or shown inconsistently. The
glyph column mixes status with a `[n]` item count. The words `project` / `checklist` appear in
the hint column only when there is no repeat hint. An empty project looks like a plain task, and
the Details pane never names the type. Every row in every view should show its type the same
way, and Details should name it.

## Goals

- The type of each row is visible in every view (Today, All, lists, Done, and the others),
  whether the parent is collapsed, expanded, empty, repeating, completed or cancelled.
- Status stays readable at a glance: `○` open, `✓` done, `✗` cancelled, `↺` repeating, on
  parents too.
- The Details pane names the type.

## Non-goals

- No change to the data model, writes, CLI output or the protocol.
- No new key bindings. Type changes stay on `ct` / `cp` / `cc`.

## Approaches

- **A, chosen:** keep the status glyph first and add a type-mark column after it. The item
  count moves right, before the hint.
- **B:** the type mark replaces the status glyph, and status is shown some other way. Rejected
  because it costs the at-a-glance `✓` / `↺` on parents.

## Design

Row layout (Tasks pane):

```
▌<status> <type> <title> <flag/pending> … [n]  <repeat hint>  <date>
```

- **Status glyph:** `✓` completed, `✗` cancelled, `↺` repeating, `○` otherwise. This now
  applies to parents as well, which today show `[n]` in this column.
- **Type mark:** `▤` project, `≡` checklist, blank for a plain task. A plain task with items
  shows `▤`, matching the documented "a plain task with items shows as a project". The column
  is 2 cells wide (mark plus a space) and is drawn only when some visible row has a mark, so a
  view with no parents keeps its current width. *(My call.)*
- **Item count:** `[n]` (all items, as today) in a right-hand column before the hint, only on
  rows with items.
- **Hint column:** repeat hint only. The `project` / `checklist` words are dropped.
- **Items** carry their own status and type mark like any row. They are always blank-typed,
  because the model is flat. Indentation is unchanged.
- **The new-task row** keeps `○` and a blank type mark.

Details pane:

- Adds a `Type` line (`Task` / `Project` / `Checklist`) under `List`, using the same rule for
  a plain task with items (`Project`).
- Its `── Tasks ──` lines use the same status glyph and type mark as rows.

## Constraints

- Keep the marks single-width. The first pick for checklists, `☰` (U+2630), rendered two cells wide
  in the demo terminal (Unicode 16 made the trigram block wide), so checklists use `≡` (U+2261). Before settling on `▤` / `≡`, check that they render one cell
  wide in a common terminal font; swap them for other single-width glyphs if they don't.
- Update `docs/tui.md` (the Tasks-pane description and the ASCII sketch).

## Demo

Rewrite `docs/demo/tui.tape` around this flow and re-render `docs/demo/tui.gif` with
`docs/demo/render.sh`:

1. Start the TUI.
2. Go to the Inbox (`0`) and create an item (`o`).
3. Move it to another list (`m`), go to that list, convert it to a checklist (`cc`) and add
   a few items (`a`, then `Enter` for each item).
4. Go to Today, see what's due and complete a task (`x`).
5. Go to All (`2`), find a task (`/`), and edit its note in the card (`i`, Tab to Notes,
   save).
6. Run a sync by hand (`:sync`), showing the spinner and then `synced just now`.

The demo data stays synthetic (`tools/demo_vdir.py`), and the config and state stay in the
script's temporary directory. The user reviews the re-rendered GIF before it is committed.

### Sync in the demo

There is no server in the demo, so `render.sh` adds a `[sync]` table to the demo config and
points `command` at a stand-in vdirsyncer script in the temporary directory. The script waits
about 1.5 s and exits 0, and `metasync = false` is set. This mirrors `fake_vdirsyncer` in
`crates/twydo-daemon/tests/common/mod.rs`. `password_command` is a harmless `["echo", "demo"]`,
and `url` / `username` are placeholders (`https://dav.example.test/demo/`, `demo`).

- **Chosen:** the stand-in. It is fast and repeatable and touches no network. The demo shows
  the UI's sync states, not a real exchange.
- **Rejected:** a throwaway local Radicale server with the real vdirsyncer. It is a true round
  trip, but there are more moving parts, and the first sync has to bind and upload every list.

Left to the builder:

- Making sure automatic sync does not start mid-demo, so the spinner appears only at step 6.
  For example, set `:sync off` before recording, or raise `batch_size` / `max_wait_secs`.
- Keeping first-run list discovery from calling the list helper against the stand-in. If it
  does, the stand-in or the demo state must satisfy it, or the step must be skipped the way
  the daemon tests do.

## Left to the builder

- Where the type-mark and count widths are computed in `ui/tasks.rs` (alongside `glyph_w`,
  `hint_w`).
- Which `ui_render.rs` / `app_keys.rs` expectations need updating, and the new render tests:
  one per case in Goals.
