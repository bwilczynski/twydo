# Adding, editing and reshaping tasks in the TUI: design

Date: 2026-11-03. Status: draft for review.

This design covers how tasks are added and reshaped in the `tdo` TUI. It has three phases:

1. **Phase 1 (build next):** adding tasks, projects, checklists and their items inline; the
   in-place edit card; changing a task's type; setting any of 2Do's repeat rules.
2. **Later (designed here, built after):** indenting into or out of a parent, moving into a
   parent, yank and paste (duplicate), list management in the sidebar, Project View.
3. **Natural language (proposal only):** decided when that phase starts.

The main spec ([2026-10-31-terminal-tasks-design.md](2026-10-31-terminal-tasks-design.md)) still
governs everything not said here. Where the two differ, this document records the user's newer
decision; see [Changes to the main spec](#changes-to-the-main-spec).

## 1. Intent

The user captures fast and sorts later. Most new things go into Inbox as a bare title, often
without a due date or repeat. Later the user comes back and reshapes them:

- a task **grows into a project or a checklist** (the common case) and gets items added under it;
- sometimes it goes into an existing project, or a new project is made for it;
- it moves to the right list, and that list is created if it does not exist yet.

The TUI should make both halves quick: capture with a few keystrokes, then reshape without
leaving the list.

**Look and feel:** vim keys for movement and editing; 2Do's layout and behaviour underneath,
adapted to a terminal. When the two clash, vim wins for the key, 2Do wins for what happens.
2Do's behaviour is documented in [`docs/reference/2do/`](../../reference/2do/README.md)
(01 §1 task types, §6 repeats, §16 quick entry, §17 inline editor; 03 §4 the editor card;
04 the shortcuts).

## 2. Goals, non-goals, anti-goals

**Goals (phase 1)**

- Add a task where you are looking, typed straight into the list.
- Fill a checklist or project quickly: `Enter` after an item opens the next one.
- Turn a plain task into a project or checklist, and convert between the three types.
- Edit every phase-1 field in one in-place card, the same card for new and existing tasks.
- Set, change and remove **every** repeat rule 2Do has (01 §6), typed with a live preview.
- Everything Twydo writes must look right in 2Do on the iPhone (verified mode, §7 below).

**Non-goals (phase 1)**

- Indent/outdent, moving into a parent, duplicate, list management, Project View: designed in
  §5, built later.
- Natural-language parsing of the title: proposal in §6, decided later.
- Alerts, tags, priority, star, duration: not in this design.
- A global quick-entry hotkey (main spec §10 step 7) stays a later addition.

**Anti-goals: it fails even if it works when**

- a task Twydo created or reshaped looks wrong, loses data, or behaves differently in 2Do on the
  iPhone;
- an unverified write reaches the production account (verified mode must refuse it with a reason);
- capture gets slower: more keystrokes than today's 2Do for a bare-title Inbox task;
- a key breaks a vim habit the TUI already relies on (`j`/`k`, `dd`, `u`, `x`, `gg`/`G`).

## 3. Phase 1: adding and editing

### 3.1 The new row (`o` / `O`)

`o` opens a blank row below the cursor, `O` above it. The title is typed straight into the list
(2Do's in-place editor, 01 §17). The status line shows `INSERT` and where the task will go:
`New task in Inbox · Enter save  Tab more  Esc cancel`.

Where the new task goes:

| Where the cursor is | The new task |
|---|---|
| a list | goes into that list |
| **Today** | goes to the default list, **due today** (2Do: "new tasks created while viewing Today are due today", 01 §3) |
| All, Scheduled, Done, Next 3 Days, other smart lists | goes to the default list, open, no dates |
| an item inside a project or checklist (in any view) | becomes a sibling item in the same parent (in Today, a project item is due today; a checklist item has no dates) |
| an empty list or view | as for that list or view |

The **default list** is `default_list` in the config (it already exists in
`twydo_core::config`), else the list named Inbox, else the first list. The status line names it,
so the user is never surprised.

Keys on the row (same editing keys as today's form): typing, `Backspace`, `Ctrl-u` clears,
`Ctrl-w` deletes a word, paste goes in as one line.

- `Enter` saves the task. Outside a parent, the row closes and the cursor goes to the new task.
  **Inside a project or checklist, `Enter` saves and opens the next item row** below it
  (2Do: "↩ while editing a sub-task title creates the next sub-task", 01 §1). `Enter` on an
  empty row closes it without writing.
- `Tab` expands the row into the card (§3.3), keeping what was typed.
- `Esc` cancels the row; nothing is written, even if a title was typed (the user's decision:
  `Esc` always cancels, on the row, in the card and on questions). Items already saved in an
  `Enter` chain stay.
- An empty title is never saved.

After saving, the task sits where the view's sort puts it, and the cursor follows it. If it
does not belong in the current view (for example a new undated task added while in Scheduled),
the status line says `Added "…" to Inbox` and the cursor stays.

`o` and `O` differ only while typing until manual ordering exists (Plan 5b). After that, the
row's position becomes the task's manual position.

### 3.2 Adding inside a parent (`a`)

`a` on a project or checklist opens a new item row at the **end of its items**, expanding the
parent if it was collapsed. The `Enter` chain works as in §3.1.

`a` on a **plain task** first asks in the status line:
`Add items: make it a (p)roject or (c)hecklist?`. `p` or `c` converts the task (§3.4) and opens
the item row; any other key cancels. Converting and the first item are separate writes; undo
takes them back one at a time.

`a` on an item inside a parent adds a sibling at the end of that parent.

### 3.3 The card (`Tab` while adding, `i` on any task)

The row expands **in place** into a card (2Do's inline editor, 03 §4). The rows below shift
down, and the list and Details pane stay visible. The card replaces today's centered edit form.
`i`, `Enter` on a task without items, and `r` (opens on the Repeat field) all open it.

```
│ ▌○ Call the plumber                                 │
│  ┌──────────────────────────────────────────────┐   │
│  │ ☐ Plan the garden▏                           │   │
│  │  Inbox ▸   Checklist    due Sat 7 Nov       │   │
│  │ Due     sat           → Sat 7 Nov            │   │
│  │ Start   —                                    │   │
│  │ Repeat  none                                 │   │
│  │ Notes   —   (Enter opens $EDITOR)            │   │
│  └─ Ctrl-s save · Tab next · Esc cancel ────────┘   │
│ ▌○ Renew library books                 Yesterday    │
```

Fields, in `Tab` order:

| Field | Input |
|---|---|
| Title | text; must not be empty |
| List | the list tab, in the list's colour. `Enter` opens the list picker (same as `m`). On an existing task, changing it is a move (same op and rules as `m`). Not shown on items inside a parent (they live in the parent's list) |
| Type | Task / Project / Checklist; `Space` or `h`/`l` cycles. Changing it is a conversion (§3.4), with its warnings shown when you save |
| Due | as today (`twydo_core::dateinput`): `fri`, `3d`, `15th 5pm`, … with the `→` preview line |
| Start | as today |
| Repeat | typed rule with a preview line (§3.5); shown on every task, `none` when it does not repeat |
| Notes | `Enter` opens `$VISUAL`/`$EDITOR`, as today |

Checklist items show only Title and Notes: they have no dates, repeat or alerts of their own
(2Do 01 §1). Project items show the full card.

Keys stay as in today's form: `Tab`/`Shift-Tab` (or `↓`/`↑`) move, `Enter` saves (except on
List and Notes, where it opens the picker or editor), `Ctrl-s` saves from any field, `Esc`
cancels, only changed fields are written. Below 60 columns (or when the card cannot fit), it
falls back to today's centered overlay with the same fields.

### 3.4 Changing the type (`ct` / `cp` / `cc`)

`ct` task, `cp` project, `cc` checklist, on the selected task; also the card's Type field.
Behaviour follows 2Do (01 §1):

- **Task → project or checklist:** the task keeps its title, dates, repeat, notes; it now has
  no items.
- **Checklist → project:** items stay; they have no dates.
- **Project → checklist:** asks first in the status line:
  `Items lose their dates and repeats. Convert? (y/n)`. Items lose dates, repeats and alerts.
- **Project or checklist with items → task:** asks first. What 2Do does with the items here is
  not documented yet (it is marked `[?]`): the verification round (§7) records it, and Twydo
  does the same. Until then, Twydo refuses this conversion with a reason.
- An empty project or checklist → task: no question.
- Converting to the type it already has does nothing.

### 3.5 The Repeat field

Typed, with a live preview line, like Due and Start (`→ Monthly on the 1st · from due date`).
Every rule kind in 2Do's table (01 §6, "Exact encoding") is covered:

| Typed (examples) | 2Do rule |
|---|---|
| `none`, empty | no repeat |
| `daily`, `every day`, `3d`, `every 3 days` | every N days |
| `weekly`, `biweekly`, `2w`, `every 2 weeks` | every N weeks |
| `monthly`, `3m`, `every 3 months` | every N months, on the Due date's day (the field takes no day; set it in Due) |
| `yearly`, `2y`, `every 2 years` | every N years |
| `mon thu`, `mon,thu`, `every mon and thu` | specific weekdays |
| `workdays` | specific weekdays = the work days from a new `work_days` config key, default Mon–Fri (bitmask `124`; 2Do's "Work days" preset) |
| `last day`, `last day every 2m` | last day of the month, every N months |
| `2nd tue`, `last fri`, `1st sun` | Nth weekday of each month |
| `1st weekday`, `last weekday` | Nth workday (Mon–Fri) of each month |
| `1st weekend day`, `last weekend day` | Nth weekend day of each month |

Modifiers, after any rule: `after done` (repeat from completion date), `until 2027-06-01` or
any Due-style date (end on a date), `10 times` (end after N occurrences).

Rules for the field:

- On an existing repeating task, the field starts with the rule written in this syntax, so
  editing it is changing text, not starting over. This replaces today's `every _ months` field.
- When the field is empty and focused, the preview line shows a few examples as a hint.
- A repeat needs a due date. If Due is empty when a repeat is set, the save fills Due with the
  rule's first occurrence from today, and the preview says so. (Builder: confirm what 2Do does
  in the verification round; follow 2Do if it differs.)
- `monthly` on a due date of the 29th to 31st, and `yearly` on 29 February, are their own
  matrix classes (`months-day-29-31`, `years-feb-29`); they need their own verification.
- The preview uses 2Do's own wording where 01 §6 records it ("Bi-weekly", "The second Tue of
  each month", "On the last day of every month"), so the TUI and the phone say the same thing.
- What 2Do's "Every Day" preset writes (every-1-day or the all-weekdays bitmask, both labelled
  "Daily") is not recorded. The verification round records it; `daily` writes the same.
- The parser lives in `twydo-core` next to `dateinput`, so phase 3 can reuse it.

### 3.6 Writes, undo and sync

- Every add, conversion and repeat change goes through the existing operation journal, so it
  is undoable (`u`/`Ctrl-r`) and triggers the after-edit sync like any other write.
- Each saved row in an `Enter` chain is one write and one undo step.
- One card save is **all or nothing and one undo step**, even when it changes fields, moves
  the task and changes its type together. The gate checks every part before anything is
  written; if any part is refused, nothing is written and the status line says which part.
  This likely needs a grouped journal entry in the core, with its own recovery tests.
- Nothing new in the sync, conflict or recovery contracts; this design only adds operations.
- `tdo op` gets the same operations (create with parent, type and repeat; convert; set repeat),
  so they can be tested and scripted outside the TUI, as Plan 4 did for the existing ones.

## 4. Phase 1 keys

| Key | Where | What it does |
|---|---|---|
| `o` / `O` | Tasks | new row below / above (§3.1) |
| `a` | Tasks | new item inside the selected parent; on a plain task, asks project or checklist (§3.2) |
| `Enter` | new row | save; inside a parent, save and open the next item |
| `Tab` | new row | expand into the card |
| `Esc` | new row, card | cancel |
| `i` | Tasks, Details | card for the selected task |
| `r` | Tasks, Details | card, on the Repeat field (now on every task, not only repeating ones) |
| `ct` / `cp` / `cc` | Tasks, Details | change type to task / project / checklist (§3.4) |

`c` becomes a prefix key, like `d` and `g` today. All existing keys keep their meaning.

## 5. Later features (designed now, built after phase 1)

Each needs its own plan and, where it writes something new, its own verification round.

| Key | Where | What it does |
|---|---|---|
| `>>` | Tasks | indent: the task becomes the last item of the nearest parent above it in the same list. Into a checklist, it loses its dates (asks first, as for project → checklist). List views only |
| `<<` | Tasks | outdent: the item leaves its parent and becomes a task in the same list. A checklist item comes out undated |
| `m` | Tasks | the picker shows each list **with its projects and checklists nested under it** (like 2Do's sidebar), so a task can move into a parent. Typing filters; when nothing matches, the last row is `+ New list "…"` |
| `yy` | Tasks | yank the selected task (with its items, when it is a parent). Nothing is written; the status line says `yanked "…"` |
| `p` / `P` | Tasks | paste a copy below / above the cursor. The copy goes where `o` / `O` would put a new task at that spot (§3.1), so `yyp` is 2Do's Duplicate and yank, go elsewhere, `p` copies across lists and into parents. On a parent row, the copy goes after the parent as a sibling (vim's linewise rule); `a` stays the way to add inside. A copy pasted into a checklist loses its dates. The copy is open, with new UIDs |
| `o` | sidebar | new list as an inline row: type the name, `Enter`. It takes the next colour from 2Do's palette (03 §7) and goes into the group under the cursor |
| `i` | sidebar | list card: name, colour (palette picker plus any RGB), group, and "show in All" / "show in Today" |
| `dd` | sidebar | delete the list; asks first and says how many tasks it holds |
| `Enter` | parent row in Today, Next 3 Days, Scheduled, Done | opens **Project View**: only that parent and its items (2Do ⌥⌘J; issue #8). `Esc` or `h` goes back. In lists and All, `Enter` keeps expanding/collapsing |

Decided here so phase 1 does not paint into a corner:

- `dd` stays a plain delete and does **not** fill the paste register. A delete plus a paste
  would be a new task with a new identity, which sync treats differently from a move. Moving
  stays with `m`.
- Paste is session-only (no system clipboard) in this design.
- **Lists are the riskiest piece.** A new list is a new CalDAV collection: vdirsyncer must create
  it on the server (`discover` asks per collection today), and metasync carries its name and
  colour. Whether 2Do on the iPhone picks up a list created that way, and what list metadata
  2Do expects, is unverified. Deleting a collection on the server is not something vdirsyncer
  does. The list-management plan starts with a verification round and may need a small
  server-side step; it must not touch the production account until verified.
- Where list flags (show in All / Today) and groups are stored follows the main spec §5: the
  verified shared encoding if 2Do has one, else documented TOML in the config.
- The main spec's `a` for alerts moves: alerts become a row in the card when they are built.

## 6. Phase 3: natural language (proposal, decided later)

Recognised words are taken out of the title and fill the fields; the user sees this as it
happens. Proposed patterns, all reusing phase-1 parsers:

| Pattern | Fills | Example |
|---|---|---|
| a date phrase at the end, using the Due field's words | Due | `Call the plumber fri 5pm` |
| `every …`, `daily`, `weekly`, `monthly`, `yearly`, `workdays`, `last day`, `2nd tue` + modifiers | Repeat (the §3.5 parser) | `Pay rent 1st monthly` (Due the 1st, monthly on that day) |
| `starting <date>` | Start | `Prepare taxes starting mar 1` |
| `@Name` | List (an existing list; `@"Two words"` for spaces) | `Buy stamps @Errands` |

Behaviour proposed after 2Do (01 §16): recognised phrases are highlighted on the row while
typing, the card's fields fill in as you type, and one key (proposal: `Ctrl-z`) turns the last
highlighted phrase back into plain title text. An explicit `@` marker is proposed over 2Do's
guessing from list names, to avoid surprises with list names that are ordinary words. A config
switch turns detection off. The main spec §5 also lists `list(Bills)`; whether to accept 2Do's
function syntax too is part of that phase's decision.

## 7. Verification round (needed before phase 1 ships)

Production runs in verified mode, and most of what phase 1 writes has not been checked against
the iPhone. From `compat/matrix.toml` today:

- `create`, `twydo_to_iphone`: **pass** only for one-time, date-only, timed and start-date tasks.
  Unverified for checklist, project, project child, recurring project, monthly, yearly and
  from-completion.
- Rule classes: only `months-day-1-28` and `years` pass. `days`, `weeks`, `weekdays-set`,
  `months-day-29-31`, `months-last-day`, `months-nth-weekday`, `months-nth-workday`,
  `months-nth-weekend-day`, `years-feb-29`, `mod:from-completion`, `mod:count` and `mod:until` are
  all unverified.
- Converting types has no matrix row at all.

So the phase-1 plan includes a round on the **test account**, run the same way as the gap
session (`docs/reference/phase0/gap-session.md`): rounds written into a session doc, the user
approves, then captures; fixtures and matrix columns are updated. Seed data with
`tools/seed_fixture.py` so the user does not type on the phone. AGENTS.md applies: ask before
anything touches production data, the NAS or the iPhone.

The round covers, Twydo → iPhone and back:

1. creating a project and a checklist with items, and adding an item to an existing parent;
2. creating a repeating task for every rule class above, and each modifier;
3. setting, changing and removing a repeat on an existing task (edit, per class);
4. each type conversion, including what 2Do does with items when a parent becomes a task;
5. what 2Do writes for its "Every Day" preset, and whether setting a repeat without a due date
   fills one in.

New matrix rows are needed for conversions (`op = "convert"` with a case per direction). Until a
case passes, verified mode refuses that write with the usual `… is not verified yet` message,
and the rest of phase 1 still works. Only the user's daily shapes (plain tasks, projects,
checklists, monthly on a day, yearly) must pass before phase 1 is used on production data.

## 8. Tests

- Core: the repeat parser in both directions (text → rule → the 2Do encoding, and an existing
  rule → text) for every row of 01 §6's table and every modifier, with the preview wording;
  round trips through the existing preservation tests (an edit changes only the intended
  properties and their verified metadata).
- Core: creating with a parent, with each type, with each repeat; each conversion, including
  the date-dropping ones; byte-level comparison against the fixtures captured in §7.
- TUI: key tests in the style of the existing ones for `o`/`O` placement per view, the `Enter`
  chain, `Esc`, `a` on each row kind, the card's fields per type, `ct`/`cp`/`cc` with their
  questions, and refusals in verified mode.
- Docs: `docs/tui.md` (keys, the card, the new row), `docs/tasks.md` (types and repeats), and
  the `?` help. `python3 tools/check_links.py` must pass.

## 9. Left to the builder

- Exact glyphs for the three types (today: `○`, `↺`, `[n]`; 2Do uses a briefcase for projects and
  a list box for checklists), within the 16-colour theme rule.
- How the card is drawn inside the Tasks pane (scrolling so it fits, its exact width), and the
  narrow-terminal fallback threshold.
- Core API shape: whether `NewTask` grows `parent`, `kind` and `repeat`, or new ops are added;
  how `Change` expresses a repeat rule and a type change.
- Whether `ct`/`cp`/`cc` wait for the second key with a timeout, as `dd` and `gg` do today.
- Help text, status messages, empty states.

## 10. My calls (not the user's decisions)

These were not discussed; the user may override them.

- In All, Scheduled, Done and smart lists, `o` goes to the default list with no dates.
- A repeat set without a due date fills Due with the first occurrence (to confirm against 2Do).
- Converting a parent with items to a task is refused until 2Do's behaviour is recorded.
- The card falls back to the centered overlay on narrow terminals.
- `r` opens the card on Repeat for every task, not only repeating ones.
- One card save is all or nothing and one undo step (§3.6).
- `workdays` reads a new `work_days` config key, default Mon–Fri. The Nth-workday rules stay
  Mon–Fri, as 2Do's encoding fixes them.
- The Repeat field never names a day of the month: monthly and yearly repeat on the Due date's
  day, as 2Do stores them.
- `p` drops dates when pasting into a checklist; paste is session-only.
- Natural language uses an explicit `@List` marker (proposal only).

## Changes to the main spec

- §4 decision 5 ("`o` opens a one-line natural-language input with a preview") is replaced by
  the user's decision here: `o` opens an inline row in the list, the card holds the fields, and
  natural language comes later on that same row (§6).
- §8 keymap: `O` is "new row above" (was "new item under the selected parent", now `a`); `a` is
  "add item inside" (was "edit alerts"; alerts move into the card); `c` is the type-change prefix;
  `r` opens the card on Repeat; `yy`/`p`/`P` and `>>`/`<<` are reserved as in §5.
- §10 step 4 ("Natural-language add (`o`) with preview") becomes the three phases above.
