# Discard Confirmation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `Esc` on a task edit card with unsaved changes asks `y`/`n` before discarding them (#99).

**Architecture:** The pure `EditForm` state machine (`crates/tdo/src/form.rs`) gains a
`discarding` flag and a dirty check; the app's `form_key` needs no change because the form only
returns `FormAction::Cancel` once discarding is confirmed. The card renderer swaps its footer hint
for the question while discarding.

**Tech Stack:** Rust, ratatui, crossterm.

**Spec:** `docs/superpowers/specs/2026-11-14-discard-confirmation-design.md`

## Global Constraints

- Task edit card only; the list card (`listform.rs`), the new row and `Ctrl-c` are unchanged.
- Footer question, verbatim: ` Discard changes? y discard · n/Esc keep editing `
- Nothing is ever written by `Esc` or by confirming the discard.
- Commits pass commitlint (`type(scope): lowercase subject`), no agent attribution trailer.

## Review Focus

- Dirty card, `Esc`, `n`: every field keeps its typed value and focus is unchanged.
- Dirty card, `Esc`, `Esc`: same as `n` (back to editing), not a second cancel.
- Card whose fields were edited back to their original text: clean, closes on one `Esc`.
- List changed through the picker only: dirty.
- Pasting while the question is shown: ignored, the question stays.

---

### Task 1: Discard question in the edit form

**Files:**
- Modify: `crates/tdo/src/form.rs` (struct `EditForm`, `with`, `key`, `paste`)
- Test: `crates/tdo/tests/form.rs`, `crates/tdo/tests/app_keys.rs`

**Interfaces:**
- Produces: `pub discarding: bool` field on `EditForm` (starts `false`);
  `pub fn is_dirty(&self) -> bool` on `EditForm`.

- [ ] **Step 1: Write failing tests in `crates/tdo/tests/form.rs`**

Using the file's helpers (`form`, `key`, `code`, `type_str`, `focus`, `ctrl`):

- `clean_card_cancels_on_esc`: `form("Pay water bill")`; `f.key(code(Esc)) == FormAction::Cancel`;
  `!f.discarding`.
- `edited_back_card_is_clean`: type `"x"` into Title, then `Backspace`; `!f.is_dirty()`;
  `Esc` returns `Cancel`.
- `dirty_card_asks_before_discarding`: type `"!"` into Title; `f.is_dirty()`;
  `f.key(code(Esc)) == FormAction::None` and `f.discarding`; `f.key(key('y')) == FormAction::Cancel`.
- `keep_editing_keeps_fields`: type `"!"`, `Esc`, then for each of `key('n')` and (after another
  `Esc`) `code(Esc)`: returns `FormAction::None`, `!f.discarding`, title still ends with `!`,
  focus still `Field::Title`.
- `discard_question_ignores_other_keys_and_paste`: dirty, `Esc`; `f.key(key('z'))` and
  `f.key(ctrl('s'))` return `FormAction::None`, `f.discarding` stays true, title unchanged;
  `f.paste("abc")` leaves title unchanged.
- `picked_list_makes_card_dirty`: `f.set_list("other".into())`; `f.is_dirty()`.
- `new_task_card_dirty_while_titled`: `EditForm::new_task(target, "Socks".into(), NY)` is dirty;
  with title `""` it is clean (build `NewTarget { collection: "personal".into(), parent: None,
  parent_kind: None, due_today: false }`).

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p tdo --test form`
Expected: compile errors on `discarding` / `is_dirty`.

- [ ] **Step 3: Implement in `crates/tdo/src/form.rs`**

- `is_dirty`: for a new task (`uid.is_none()`), `!self.title.trim().is_empty()`; otherwise the
  current values differ from `self.initial` (compare a `Values` built from the fields with
  `self.initial`; `Values` already derives `PartialEq`).
- `key`: when `self.discarding`, handle only `y` → `Cancel`, `n`/`Esc` → clear flag, return
  `None`; everything else returns `None`. Otherwise `Esc` returns `Cancel` when clean, or sets
  `discarding` and returns `None` when dirty.
- `paste`: return early while `discarding`.
- Update the module doc comment to mention the discard question.

- [ ] **Step 4: Update app tests in `crates/tdo/tests/app_keys.rs`**

`esc_cancels_without_write` and the test ending with `ctrl('u')` then `Esc` (around line 1036)
now edit the card: after `Esc` assert `a.form.as_ref().unwrap().discarding`, then press `key('y')`
and keep the existing assertions (form closed, `Mode::Normal`, no message, `Outcome::Continue`).
Add `esc_keeps_dirty_card_on_n`: `i`, type `!`, `Esc`, `n` → `a.form.is_some()`,
`a.mode == Mode::Insert`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p tdo --test form --test app_keys`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/tdo/src/form.rs crates/tdo/tests/form.rs crates/tdo/tests/app_keys.rs
git commit -m "feat(tui): ask before discarding an edited card"
```

### Task 2: Footer question and docs

**Files:**
- Modify: `crates/tdo/src/ui/card.rs` (`HINTS`, `block`, its callers)
- Modify: `docs/tui.md` (edit card section, the `Esc cancels` bullet near line 292)
- Test: `crates/tdo/tests/ui_render.rs`

**Interfaces:**
- Consumes: `EditForm::discarding` from Task 1.

- [ ] **Step 1: Write the failing test `card_footer_asks_to_discard` in `ui_render.rs`**

`i`, type `!`, `Esc`; render 120×30: text contains `Discard changes? y discard · n/Esc keep editing`
and not `Ctrl-s save`. Render 59×30 (overlay): same question shown.

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p tdo --test ui_render card_footer_asks_to_discard`
Expected: FAIL (footer still shows hints).

- [ ] **Step 3: Implement**

`block` takes the form (or the footer text): the question constant above in `theme::accent()`
style while `e.discarding`, else `HINTS` in `dim()`. Both the in-place and overlay paths pass it.

- [ ] **Step 4: Update `docs/tui.md`**

Replace the `Esc` bullet: unchanged card closes at once; a changed card asks
`Discard changes?` — `y` discards (nothing written), `n` or `Esc` returns to the card with
everything typed. Run `python3 tools/check_links.py`.

- [ ] **Step 5: Run the full check**

Run: `cargo test -p tdo && cargo clippy -p tdo --all-targets -- -D warnings && cargo fmt --check`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/tdo/src/ui/card.rs crates/tdo/tests/ui_render.rs docs/tui.md
git commit -m "feat(tui): show the discard question in the card footer"
```
