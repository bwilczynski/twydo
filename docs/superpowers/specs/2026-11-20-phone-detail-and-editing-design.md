# Phone task screen and editing

Date: 2026-11-20. Status: proposed. Builds on the
[phone companion MVP](2026-11-17-phone-companion-mvp-design.md) and its follow-ups (Views tab,
#145). Changes the MVP's scope (§12 below).

## 1. Intent

The phone app is close to replacing the 2Do iPhone app for the user's away-from-desk moments.
Planning and reshaping stay in the terminal, but when only the phone is in reach the user wants
to do real work there: tick off checklist items one by one, read everything about a task, fix a
date, add items to a checklist, write a longer note, move something out of Inbox, set up a
repeat.

Today the phone cannot do most of that. It cannot even tick a single checklist item, because
views arrive collapsed and the phone never sees the children; and the tap-to-preview sheet shows
only title, dates, list, parent and note.

The MVP limited the phone to capture and completion as a scope choice, not for safety: every
phone write already goes through `op.submit`, the gate, the queue and the journal, exactly as a
`tdo` write does. This design lifts that limit to a useful editing set and leaves the rest to the
TUI.

## 2. Goals, non-goals, anti-goals

**Goals**

- **Task screen.** Tapping a row pushes a full screen with everything the TUI details pane shows:
  title, list, type, due, repeat, alerts, start, warnings, blocked reason, conflict, notes, and
  the parent it belongs to.
- **Children.** On a project or checklist the task screen lists every child, open and completed,
  each with a checkbox to complete or reopen it. Tapping a child pushes its own task screen.
- **Complete and reopen** any task from its screen (reopening is new on the phone).
- **Edit** a task in a full-screen form with the TUI card's fields for its kind: title, list,
  type, due, start, repeat, alert, notes. One Save is one write and one undo step.
- **New task** in the same full-screen form, sliding up from the bottom: title, list, type, due,
  start, repeat, notes (no alert, as in the TUI: alerts are set on an existing task).
- **Add an item** to a project or checklist from its task screen.
- **Delete** a task (a parent with its items), with a confirmation and the usual Undo.

**Non-goals** (stay in the TUI)

- Reordering, indent and outdent, reparenting by drag.
- List management (create, rename, recolour, archive, delete), conflict resolution, journal
  recovery, sync control.
- Natural-language input on the phone; Web Push; offline editing.
- Swipe gestures (swipe-to-complete, swipe-down-to-dismiss). Buttons only in this round.

**Anti-goals** (failure even if it works)

- A second copy of the task model or the form's rules in TypeScript: which fields a kind has,
  how typed dates and repeats read, what a save writes. The phone shows and sends what the
  daemon computes.
- A phone write that `tdo` would not make for the same input, or that bypasses the gate.
- An edit silently overwriting a change made elsewhere (in `tdo`, by sync, or on 2Do) after the
  form was opened, or silently throwing away what the user typed.
- Opening reorder, list, conflict, journal or sync methods to the phone role.

## 3. Constraints

- Everything in the MVP spec's §2 anti-goals still holds (daemon stays on the Unix socket, role
  allowlist in the gateway, no REST resources of the gateway's own).
- The phone is a thin client: it may format and lay out, never interpret iCalendar or 2Do
  metadata.
- New methods and types go into `schema/tdo.openrpc.json` as additions and must pass the
  released-schema test.
- No device rounds are authorized by this design. Verification is local (unit, daemon,
  gateway, browser tests); the check on the user's iPhone is the user's, after release.
- Commit messages and PR titles pass commitlint (AGENTS.md).

## 4. Approaches considered

1. **A task read plus one whole-form save** *(chosen by the user)*. A new daemon read returns
   one task worded for display, its children, and the edit form's starting values; a second
   method checks the form's typed text with the TUI's own form code and returns previews,
   errors and the finished `Op`; Save submits that one `Op::Save` (or `Op::Create`). One write,
   one journal entry, one undo step, the same rules and wording as the TUI card.
2. **Each field saves on its own** (iOS Reminders style). Native feel, but every field is its
   own write and undo step, a repeat needing a due date becomes two writes, and each can bounce
   on the stale check. Wastes the existing one-save path. Rejected.
3. **The phone reads the snapshot and works things out itself.** Needs kinds, field rules and
   repeat wording in TypeScript: a second task model, which the MVP names as an anti-goal.
   Rejected.

## 5. Delivery

Two plans, shipped in order; each is releasable on its own.

- **Plan A: task screen.** `tasks.get` (§6.1), the task screen with children (§8.2), complete
  and reopen from it, the gateway allowing `tasks.get` and `reopen`, and the new-task sheet
  restyled as a full-screen bottom sheet with today's fields (title, note). This alone fixes the
  checklist problem.
- **Plan B: editing.** The form moved into `twydo-core` (§6.2), `tasks.form` (§6.3), the edit
  form, the full new-task form, Add item, Delete, and the gateway allowing `save`, full
  `create` and `delete`.

## 6. Daemon

### 6.1 `tasks.get`

Params: `{ uid }`. Result:

- `epoch`, `revision`, `now`, `today`: as `views.get` (`based_on` for writes from this screen).
- `task`: the task worded for display, using the same `twydo_core::format` helpers the TUI
  details pane uses (`crates/tdo/src/ui/details.rs`):
  - `uid`, `title`, `kind` (the shown kind, as the TUI's `shown_kind`: an untyped parent shows
    as a project), `list` (id, name, colour);
  - `due` (relative words, e.g. `Today 17:00`), `overdue`, `due_today`, `start` (relative
    words), `repeat` (the TUI's described rule, or `unsupported rule`), `alerts` (one line each,
    `describe_alarm`);
  - `warnings` (lines), `blocked` (text or null), `conflict` (true when a recorded conflict
    exists; the phone says "resolve it in tdo");
  - `note`, `completed`, `pending`;
  - `parent`: uid, title and kind when the task is an item.
- `children`: every child, open ones first in sibling order (`compare_items`), then completed
  ones, newest completion first *(my call; the TUI shows only a count, but the phone must be
  able to reopen them)*. Each child carries `uid`, `title`, `kind` marker, `due` (relative
  words), `overdue`, `due_today`, `completed`, `pending`, `has_note`, `child_count`. Empty for a
  task without children.
- `form`: the edit form's starting state (§6.2), built with `EditForm::open` (the daemon looks
  up the parent's kind itself for an item): `fields` in display order; `values`, the text of
  each field as the TUI card shows it (`title`, `list` id, `kind`, `due`, `start`, `repeat`,
  `alert`, `notes`); `previews`, the preview lines the card shows on opening (so a kept,
  unwritable repeat rule shows `(kept)` before anything is typed); `alert_editable: false` when
  the task has alerts Twydo cannot write. Present in Plan A already so the result shape does
  not change in Plan B.

Completed children are ordered newest `COMPLETED` first; children without a completion time
come last, in sibling order *(my call)*.

The task as `tasks.get` returns it is the daemon's snapshot with queued writes laid over, as
views are. A `pending` task can be opened and edited like any other *(my call)*; the write
path decides as it does for `tdo`.

A uid that no longer exists is refused with kind `invalid` (as a gone list is for `views.get`).
The phone refetches `tasks.get` on `data.changed` and `status.changed`, like a view.

The wording moves out of `crates/tdo/src/ui/details.rs` into `twydo-core` (pure functions over
the snapshot) so the TUI pane and `tasks.get` share one source *(my call)*; the TUI pane keeps
rendering it.

### 6.2 The form in `twydo-core`

`crates/tdo/src/form.rs` (`EditForm`) is pure apart from key handling. Its model moves to
`twydo_core::form`: the field set per kind and per parent kind (a checklist item has Title and
Notes; no item has List; a new task has no Alert), the starting values from a task, the previews,
`changes`, `save` (`Op::Save` for an edit, `Op::Create` for a new task, with parent for an item)
and `rebase`. Key handling, focus and the discard prompt stay in `tdo`. The TUI's behaviour
does not change; its tests move or stay with the part they test.

### 6.3 `tasks.form`

Checks a form without writing; a pure read over the current snapshot (with queued writes laid
over), holding no state between calls. Params:

- `uid` (an edit) or `new`: `{ collection, parent?, due? }` (a new task; `due` lets Today's
  capture start with today, as now). `parent` is a uid; the daemon resolves its kind.
- `initial`: for an edit, the `form.values` the form was opened with (from `tasks.get`, or from
  the last accepted rebase). The phone carries them back so the daemon can tell what the user
  typed from what changed elsewhere, as `EditForm::rebase` does with its `mine` card; the
  daemon keeps no old snapshots.
- `values`: the typed text of each field, as `form.values`.

Result:

- `fields`: the field set for these values (changing Type, or a new item's parent kind, changes
  it).
- `previews`: per field, the TUI's preview line (`→ Fri 21 Nov`, `→ every 2 weeks on Mon`, the
  examples line, `(kept)` for an unwritable rule left untouched) or an error message.
- `op`: the finished `Op` when every field reads and something would be written; null otherwise,
  with `error` set to the TUI's message (`no changes`, `title must not be empty`, …).
- `changed_since`: true when the current task's starting values differ from `initial` (a
  per-task comparison, matching `op.submit`'s per-task stale check, not the global revision).
  Then `rebased` carries the form rebased onto the current task with `EditForm::rebase` (typed
  fields kept, fields changed elsewhere and not touched by the user taken from the current
  task), with its new `initial` and basis; `overwritten` lists the fields both sides changed,
  where the user's typing wins (`Rebased::overlap`); `dropped` lists typed fields the current
  task's form can no longer take (it became an item, its alert became unwritable), whose typing
  is lost (`Rebased::dropped`). The phone shows both lists before the user saves again.
- The `op` in the result is built against the current task; the phone submits it with the
  basis `tasks.form` returned.

`input.parse` stays as it is; the phone does not need it.

## 7. Gateway

The phone role's table changes to:

| Role | Methods | `op.submit` ops |
|---|---|---|
| `phone` | `views.get`, `views.list`, `tasks.get`, `tasks.form`, `events.subscribe`, `events.unsubscribe`, `op.submit`, `op.undo` | `create` (any `NewTask`: list, kind, due, start, repeat, parent), `save`, `complete`, `reopen`, `delete` |
| `display` | `views.get`, `views.list`, `tasks.get`, `events.subscribe`, `events.unsubscribe` | none |

Plan A adds `tasks.get` and `reopen`; Plan B adds `tasks.form`, `save`, the wider `create` and
`delete`. Still refused for the phone: `edit`, `move` (both covered by `save`), `create_at`,
`reorder`, `reparent`, and every method not listed. The listed ops are allowed with any payload
the daemon accepts (`phone_may_submit` drops its per-field checks on `create`); the gate and
write policy decide each write exactly as for `tdo`. The gateway passes `based_on` through
unchanged, as today.

## 8. Phone app

### 8.1 Navigation

- The task screen is pushed over the current tab with `history.pushState` and a hash route
  (`#/task/<uid>`); the back button in the top-left (labelled with where it returns to: the
  view's title or the parent's title) and the browser's back both pop it. A child's screen
  pushes on top of its parent's.
- A reload on a task route reopens that task if it still exists *(my call)*.
- A task that is gone (refused `invalid`, or deleted) pops back with the banner "That task is
  gone".
- The tab bar stays visible under the task screen; tapping the active tab pops to its root.

### 8.2 Task screen

```
‹ Today                          Edit
○  Pack for the trip            (large)
   Checklist · Personal ●
   Due       Tomorrow 09:00
   Repeat    every year on 21 Nov
   Alert     1 hour before
   ! conflict — resolve it in tdo
┌──────────────────────────────────┐
│ Items · 3 open                   │
│ ○ Charger                        │
│ ○ Passport               Today   │
│ ○ Book                           │
│ ● Tickets  (done, dim)           │
│ + Add item                       │
└──────────────────────────────────┘
┌──────────────────────────────────┐
│ Notes                            │
│ …full text, links tappable…      │
└──────────────────────────────────┘
          Delete task (red)
```

- The big circle next to the title completes or reopens the task itself. Completing a parent
  also completes its open items and reopening it reopens its items (2Do's H5/H7, the gate's
  rules); the confirmation-free tap matches the list, with Undo in the banner.
- Each child's checkbox completes or reopens that child; the row stays in place, shown done.
- Rows marked `pending` show the waiting dot, as in lists.
- Fields with no value are left out. The note is shown in full, preserving line breaks, with
  URLs made tappable *(my call)*.
- Offline, every control is disabled and the screen is dimmed, as lists are.
- **Plan A** ships this screen with no Edit button, no Add item and no Delete (no
  placeholders).

### 8.3 Edit and new-task form

A full-screen sheet that slides up from the bottom (`Cancel` left, `Save`/`Add` right,
title in the middle), over the current screen:

- Fields in the order `form.fields` gives. Title is a text input; List is a picker of the lists
  from `views.list`; Type is a segmented control (Task, Project, Checklist); Due, Start, Repeat
  and Alert are text inputs in the TUI's grammar with the preview line from `tasks.form`
  underneath; Notes is a growing text area.
- Due and Start also offer a calendar button that opens the native date picker and writes the
  chosen date into the field as text *(my call)*. Alert offers chips for `15m`, `1h`, `1d`,
  `none` *(my call)*. Repeat shows the examples line while empty.
- The form calls `tasks.form` debounced (about 250 ms) as the user types. Save is enabled only
  when the last answer has an `op`; tapping it submits that `op` with the form's `based_on`.
- On success the sheet closes, the task screen (or the list) refreshes, and the banner says
  `Saved "<title>"` (or `Added …`) with Undo.
- **Changed elsewhere:** when `tasks.form` reports `changed_since`, the form shows a notice
  ("Changed since you opened it") and offers **Review**, which loads the rebased values and
  names any overwritten and dropped fields; Save then uses the new `initial` and basis. A stale refusal from `op.submit`
  leads to the same notice. Nothing typed is dropped without the user seeing it.
- Cancel with changes asks "Discard changes?" (as the TUI does).
- A refusal from the daemon keeps the sheet open with the message, as capture does today; a
  queued write refused later reopens the form from the banner's **Edit**, as capture does.
- New task from a view: `new.collection` and `new.due` from the view's `capture`; the List
  picker can change it. Add item from a task screen: `new.parent` is that task, and the form
  shows the fields the TUI shows for an item of that kind.
- **Plan A** ships only the presentation: the existing title-and-note capture moves into this
  full-screen bottom sheet.

### 8.4 Delete

`Delete task` at the bottom of the task screen asks for confirmation (an action sheet:
"Delete <title>?" and, for a parent, "and its N items"), submits `delete`, pops back, and shows
`Deleted "<title>"` with Undo.

### 8.5 Data source

`DataSource` grows a task signal per uid alongside the view signals, `task(uid)`, refreshed like
the visible view; `Visible` gains `{ task: uid }`, so `based_on` for writes from a task screen is
the task result's basis. New calls: `reopen`, `save(op)`, `remove`, `checkForm`. Refusals
(`onRefusal`) gain `save` and `delete` kinds. Request-id reuse for retries works as today.

## 9. Testing

- **Core:** the moved form's existing tests; new tests that `tasks.get` wording matches the
  TUI pane for the same snapshot (shared function, so one test per field family); children
  order including completed ones.
- **Daemon:** `tasks.get` for a task, a parent with open and completed children, an item, a
  gone uid; `tasks.form` previews, errors, `op` for an edit, a new task, a new item; stale and
  rebased answers. Schema addition test.
- **Gateway:** the allowlist table: allowed and refused ops and methods per role, including
  `edit`, `move`, `reorder`, `reparent` refused for the phone.
- **Web unit:** navigation (push, back, reload on a route, gone task), children ticking and
  unticking, form state (debounced checks, Save enablement, changed-since flow, discard prompt).
- **Browser (Playwright, phone size, light and dark):** open a checklist, tick and untick an
  item; edit a due date and a repeat; add an item; delete with undo; create a task with a list
  and a due date. Against an isolated test daemon (isolated `XDG_CONFIG_HOME` and
  `TWYDO_SOCKET`), synthetic data only.

## 10. Documentation

Update `docs/gateway.md` (role table, what the phone can do), `docs/protocol.md` (`tasks.get`,
`tasks.form`) and `docs/tui.md` only if the details pane wording moves visibly.
`python3 tools/check_links.py` must pass.

## 11. Left to the builder

- Exact field names and nesting of `tasks.get` and `tasks.form` results, within the content
  above.
- Whether the details wording is one core function returning lines or a struct per field.
- Visual details within the existing styles: spacing, the sheet's slide animation (respecting
  reduced motion), how the big title checkbox looks.
- The refusal kind for a phone op outside the table (the gateway's existing choice).

## 12. Changes to earlier designs

- **Phone companion MVP §2:** "Editing, moving, deleting, reopening; checklist items" and
  "Dates, list or repeat at capture" move from non-goals to goals. Natural-language input stays
  a later step.
- **MVP §7 role table:** replaced by §7 here.
- **MVP §10 item 5** ("Moving a task to another list, more editing"): done by this design.
