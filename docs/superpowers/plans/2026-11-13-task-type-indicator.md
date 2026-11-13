# Task Type Indicator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every task row shows its status glyph and its type mark (project, checklist or task)
the same way in every view, and the Details pane names the type. Ships with a re-recorded
demo.

**Architecture:** This is rendering only, in `crates/tdo/src/ui/tasks.rs` (row layout) and
`crates/tdo/src/ui/details.rs`. Small helpers in `tasks.rs` decide the glyph, mark and count
for a row; `details.rs` reuses them. The demo changes `docs/demo/render.sh`, which gets an
isolated socket, a stand-in sync tool and a pinned editor, and rewrites `docs/demo/tui.tape`.

**Tech Stack:** Rust, ratatui (`TestBackend` render tests), VHS, bash.

**Spec:** `docs/superpowers/specs/2026-11-13-task-type-indicator-design.md`

## Global Constraints

- Glyphs: status `○` open, `✓` completed, `✗` cancelled, `↺` repeating. Type `▤` project,
  `☰` checklist, blank for a plain task. A plain task with items counts as a project.
- Item count is `[n]`, where n is all of the parent's items (as `snapshot.children(idx).len()`
  today).
- The hint column carries the repeat hint only. The words `project` / `checklist` are gone.
- The type column is 2 cells (mark plus a space), drawn only when a visible row has a mark.
- No change to the data model, writes, CLI, protocol or key bindings.
- All test and demo content is synthetic. Demo config and state stay in `render.sh`'s
  temporary directory. Never touch the user's daemon, config or vdir.
- Commits: commitlint conventional (`type(scope): subject`, lowercase subject, at most 100
  characters, no trailing full stop). Per the user's global rules, there is no agent
  attribution trailer.
- Isolated daemons are stopped only with `tdo daemon stop` on their own socket.

## Review Focus

1. **A very narrow Tasks pane (the 80-column layout, deep nesting).** The title keeps at
   least `MIN_TITLE`. When space runs out, the hint is dropped first, then the count. Nothing
   panics. Test: `narrow_pane_drops_hint_before_count` (Task 1).
2. **The new-task row (`o` / `a`) in a view with a type column.** Its title lines up with the
   row titles. Test: `new_row_title_lines_up_with_type_column` (Task 1).
3. **A view with no parents** keeps today's layout, with no blank type column. Test:
   `no_type_column_without_parents` (Task 1).
4. **A completed or cancelled parent** shows `✓` / `✗` plus its mark, not `[n]` in the status
   column. Test: `finished_parent_keeps_status_and_mark` (Task 1).
5. **Demo isolation.** If the user's own daemon is running during `mise run demo`, the
   recording must not connect to it. The tape exports `TWYDO_SOCKET` (Task 4, Step 1).

---

### Task 1: Row status, type mark and count columns

**Files:**
- Modify: `crates/tdo/src/ui/tasks.rs` (`glyph`, `hint`, `render` column widths, `row_line`,
  `new_row_line`)
- Test: `crates/tdo/tests/ui_render.rs`

**Interfaces:**
- Produces (in `ui/tasks.rs`, `pub(super)` for `details.rs`):
  - `fn glyph(app: &App, idx: usize) -> &'static str`: status only (`✓` / `✗` / `↺` / `○`,
    in that precedence, as today minus the `[n]` branch).
  - `fn shown_kind(app: &App, idx: usize) -> TaskKind`: `t.kind`, except a `Task` with
    children is `Project`.
  - `fn type_mark(app: &App, idx: usize) -> &'static str`: `"▤"` / `"☰"` / `""` from
    `shown_kind`.
- Private: `fn count(app: &App, idx: usize) -> String` gives `"[n]"`, or `""` without
  children. `fn hint(t: &Task) -> String` gives the repeat hint or `""`.
- Layout, left to right: `›`, bar, indent, glyph (width 1), space, type column (`type_w` = 2
  when any visible row has a mark, else 0), flag column, title… then the right side: count
  (`count_w` = widest count, plus 1 gap when > 0), hint, date. `new_row_line` takes
  `(glyph_w, type_w, mark_w)` and pads the type column with blanks. Narrowing order: drop
  the hint, then the count, when `width < left_fixed + right + MIN_TITLE`.

- [ ] **Step 1: Write the failing tests** in `ui_render.rs`. Add a helper
  `fn row_text(buf: &Buffer, title: &str) -> String`, which returns the Tasks-pane line
  containing `title` (built on `find_in_tasks`). Fixtures use `todo()` / `vdir_with()`;
  kinds are `X-2DOAPP-METADATA:@tasktype:project|checklist`.

  - Update `glyphs_and_counts`. Drop the `"project"` / `"checklist"` asserts. In All, the row
    for `Prepare tax return` contains `"▤"` and `"[2]"`. `Packing list` contains `"☰"` and
    `"[3]"`. `Water plants` contains `"↺"`, does not contain `"▤"` or `"☰"`, and contains
    `"weekly"`.
  - `type_marks_are_the_same_in_every_view`: for `View::Today`, `View::All` and
    `View::List(<personal collection>)`, wherever `Packing list` is visible, its row contains
    `"☰"` and its first glyph is `○`.
  - `repeating_parent_shows_repeat_and_mark`: a checklist with `RRULE:FREQ=MONTHLY`, a due
    date and one item. Its row contains `"↺"`, `"☰"`, `"[1]"` and `"monthly"`.
  - `empty_project_is_marked`: a project with no items. The row contains `"○"` and `"▤"`,
    and no `"["`.
  - `plain_task_with_items_shows_as_project`: a task without `@tasktype` and one
    `RELATED-TO` child. The row contains `"▤"` and `"[1]"`.
  - `finished_parent_keeps_status_and_mark`: reuse the all-completed `project_vdir()` set-up
    from `completed_parents_and_nested_items_have_crossed_out_titles`. The parent row
    contains `"✓"`, `"▤"` and `"[3]"`. With the parent expanded, the `Finished child` row
    contains `"✓"` and no `"▤"`.
  - `no_type_column_without_parents`: a vdir with two plain tasks. In each row the glyph is
    followed by exactly one space and then the title (`row_text` contains `"○ Plain one"`).
  - `titles_line_up_with_and_without_marks`: in All on the sample vdir, the x of
    `Prepare tax return`, as found by `find_in_tasks`, equals the x of `Renew library books`.
  - `new_row_title_lines_up_with_type_column`: All on the sample, cursor on
    `Renew library books`, press `o` and type `New`. The x of `New▏` equals the x of
    `Renew library books`.
  - `narrow_pane_drops_hint_before_count`: render the repeating checklist fixture at
    widths 80, 60 and 40. Never panic. At every width where `"monthly"` is shown, `"[1]"` is
    shown too.

- [ ] **Step 2: Run the tests and check that they fail**

  Run: `cargo test -p tdo --test ui_render`
  Expected: the new and updated tests fail (no `▤` / `☰`, and `[n]` in the glyph column).
  The other tests pass.

- [ ] **Step 3: Implement the helpers and layout in `ui/tasks.rs`** as given in Interfaces.
  `glyph_w` becomes the constant 1. In `row_line`, after the glyph and its space, push
  `pad(type_mark, type_w)` styled with the row's `text` style, before the flag column. Push
  the count (dimmed) after the title filler, before the hint.

- [ ] **Step 4: Run the tests and check that they pass**

  Run: `cargo test -p tdo`
  Expected: all pass. Fix any other `ui_render.rs` / `app_keys.rs` expectations that relied
  on `[n]` in the glyph column or on the hint words, without weakening what they test.

- [ ] **Step 5: Lint**

  Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
  Expected: clean.

- [ ] **Step 6: Commit**

  `git commit -m "feat(tui): mark projects and checklists on every row"`

### Task 2: Details pane names the type

**Files:**
- Modify: `crates/tdo/src/ui/details.rs`
- Test: `crates/tdo/tests/ui_render.rs`

**Interfaces:**
- Consumes: `glyph`, `type_mark` and `shown_kind` from Task 1.

- [ ] **Step 1: Write the failing tests** (`details_lines` helper exists):
  - `details_show_the_type`: selecting `Prepare tax return` gives a line containing `Type`
    and `Project`. `Packing list` gives `Checklist`, and `Pay water bill` gives `Task`. The
    `Type` line comes right after the `List` line.
  - `details_type_of_plain_task_with_items_is_project`: use the fixture from
    `plain_task_with_items_shows_as_project`. The result is `Project`.
  - Extend `details_list_a_projects_open_tasks_and_completed_count`: the child lines stay
    `"○ Sooner child"` / `"↺ Later child"`, since items have no mark.

- [ ] **Step 2: Run the tests and check that they fail.** Run:
  `cargo test -p tdo --test ui_render details_`. Expected: the two new tests fail.

- [ ] **Step 3: Implement.** Add `field("Type", …)` after `List`, with the words `Task` /
  `Project` / `Checklist` from `shown_kind`. In `children()`, prefix each line with
  `glyph`, then `type_mark` plus a space when the mark is non-empty, then the title.

- [ ] **Step 4: Run** `cargo test -p tdo && cargo clippy --workspace --all-targets -- -D warnings`.
  Expected: all pass, and clippy is clean.

- [ ] **Step 5: Commit**: `git commit -m "feat(tui): name the task type in details"`

### Task 3: Docs

**Files:**
- Modify: `docs/tui.md` (the ASCII sketch at the top, the Tasks bullet at line ~31, and the
  Details bullet at line ~36)

- [ ] **Step 1: Update the text.** Tasks: `○` open, `✓` done, `✗` cancelled, `↺` repeating
  with its rule. `▤` a project and `☰` a checklist, after the status glyph. `[3]` gives
  its item count. A plain task with items shows as a project. Details: add the type. Redraw
  the sketch rows so at least one shows `○ ☰ Packing list  [3]`. Keep the sketch's widths
  consistent.

- [ ] **Step 2: Check the links.** Run: `python3 tools/check_links.py`. Expected: exit 0.

- [ ] **Step 3: Commit**: `git commit -m "docs(tui): describe type marks and the details type"`

### Task 4: Re-record the demo

**Files:**
- Modify: `docs/demo/render.sh`, `docs/demo/tui.tape`, `docs/demo/common.tape` (the hidden
  prologue), `docs/demo/tui.gif`

**Interfaces:**
- `render.sh` exports `TWYDO_DEMO_DIR` (as today). The tape's hidden prologue exports
  `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `TWYDO_SOCKET=$TWYDO_DEMO_DIR/tdo.sock` and
  `VISUAL='nvim --clean'`.

- [ ] **Step 1: Isolate the demo and add the stand-in sync tool in `render.sh`.**
  - Write `$demo_dir/vdirsyncer`, executable, with the body
    `#!/bin/sh\n[ "$1" = __probe ] && exit 0\nsleep 1.5\nexit 0`.
  - Append to the demo `config.toml`:

    ```toml
    [daemon]
    unit_warning = false

    [sync]
    url = "https://dav.example.test/demo/"
    username = "demo"
    password_command = ["echo", "demo"]
    command = "<demo_dir>/vdirsyncer"
    metasync = false
    batch_size = 1000
    max_wait_secs = 3600
    ```

    These settings keep automatic sync out of the recording.
  - Add `TWYDO_SOCKET` to the tape prologue export in `common.tape`, and to the `--shell`
    branch. Add a trap that runs `tdo daemon stop` against that socket, before `rm -rf`.

- [ ] **Step 2: Rehearse in the demo shell.** Run `docs/demo/render.sh --shell` and do
  this by hand:
  - Run `tdo sync`, then `tdo status`.
  - Start `tdo` and check the status line. Expect `synced … ago`, no `sync off`, and no
    unit warning.
  - Run `:sync` and check for the `Syncing…` spinner, then `synced just now`.
  - Check `:errors` for a discovery warning. The startup sync's discovery runs the list
    helper, which can't use the stand-in, and the docs say that warning is not saved. If a
    warning still shows anywhere visible, run the startup sync to completion in the tape's
    hidden prologue (`tdo sync` before `Show`) so it is off-screen, and re-check.
  - Exit with `tdo daemon stop`.

  Record what you saw in the commit body.

- [ ] **Step 3: Rewrite `tui.tape`** to the spec's flow. Keep `Output`, `Width 2040`,
  `Height 1100` and `Source`, and use the existing pacing style. Comments name each step.
  1. `tdo`, Enter, and pause.
  2. `0` for the Inbox. `o`, type `Plan the reunion`, Enter, and pause.
  3. `m`, type `pers`, Enter (Personal). Wait for the move to finish. Then `:list Personal`,
     Enter, and `/reunion` Enter to select it. `cc`, pause, then `a`. Type `Book a room`
     Enter, `Send invites` Enter, `Order cake` Enter, then Esc. Clear the search with `/`
     Esc and pause on the `☰ … [3]` row.
  4. `1` for Today, and pause. `/dentist` Enter (`Call the dentist`, due today), `x`, pause,
     then `/` Esc.
  5. `2` for All. `/water bill` Enter, `i`, then Tab to Notes and Enter (nvim opens). Type
     `GoPay online this month`, Escape, `:wq` Enter. Then `Ctrl+S` and pause on the Details
     pane showing the note. Clear the search.
  6. `:sync` Enter. Pause through the spinner until `synced just now` shows (about 3 s).
     Then `q`.

  Before `Show`, hidden: whatever Step 2 required. After `q`, hidden: `tdo daemon stop`.
  Update the tape's header comment to describe the new tour.

- [ ] **Step 4: Render.** Run: `mise run demo`. Expected: exit 0, and
  `docs/demo/tui.gif` is rewritten. If a step misfires (for example a filter matching two
  tasks, or a move picker name), adjust the tape and re-render.

- [ ] **Step 5: Check the glyph widths in the GIF.** Extract a frame from step 3's
  checklist row and look at it:
  `ffmpeg -y -i docs/demo/tui.gif -vf "select=eq(n\,<frame>)" -vframes 1 <scratch>/frame.png`.
  Expected: `▤` / `☰` take one cell and the titles line up. If not, swap the glyphs
  (spec constraint), redo Tasks 1–3's constants and docs, and re-render.

- [ ] **Step 6: The user reviews the GIF.** Stop and hand the user the GIF path. Do not
  commit until they approve it. Apply their changes and re-render as needed.

- [ ] **Step 7: Commit**:
  `git commit -m "docs(demo): re-record the tour with type marks and a manual sync"`
