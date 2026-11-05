# Twydo (terminal 2Do replacement): design

Date: 2026-10-31 · Status: revised after RE review; CalDAV compatibility gate pending
App name: **Twydo** · pronounced **“twhy-doo”** · terminal command: **`tdo`**

Background research on 2Do, recovered from the shipped app, is in `docs/reference/2do/`
(01 features, 02 data model, 03 look and feel, 04 shortcuts, 05 data, 06 sync, 07 RE notes).
The chosen screen layout is in `docs/design/tui-layout.md`.
This document says what to build. Those docs distinguish observed, inferred, and unresolved
2Do behaviour; they are not proof of CalDAV or iPhone interoperability. Requirements below
are product decisions unless explicitly identified as behaviour to verify in phase 0.

## 1. Intent

A personal task app that replaces 2Do for one user on **Omarchy (Linux, Hyprland)** and **macOS**.
The app must ensure that **recurring obligations are never forgotten**, including monthly and
yearly tasks and recurring parent tasks with tick-off items. It replaces a closed app and format
with open ones, uses vim-style keys, and follows Omarchy themes.

Success: the user stops opening 2Do on the desktop; every due task shows up in Today and as a
notification; the data is plain iCalendar on their own NAS; and the 2Do iPhone app can still
be used occasionally against the same data.

## 2. Goals, non-goals, anti-goals

**Goals (v1)**
- One-time and recurring tasks with due date and optional time, list, notes.
- Preserve the full 2Do repeat vocabulary: every N days/weeks/months/years, chosen weekdays, Nth weekday of month,
  last day of month, Nth weekday / weekend day of month, repeat from due date or from completion,
  end never / after N / on a date. Creating/editing and executing a rule require separate
  compatibility checks (§6); preservation alone does not establish either capability.
  The user's monthly and yearly obligations must pass those checks before daily use.
  (Reference: `docs/reference/2do/01-features.md` §6.)
- Projects and checklists with children ticked one by one, retaining their distinct scheduling
  semantics (§5). The parent is closed with its **own manual tick** (no auto-close).
  Recurring parent reset and history behaviour must be verified before enabling completion.
- Lists with colours, grouped in list groups.
- Views: Today, All, Scheduled (future start date), Done, each list, and one fixed "Next 3 Days" view.
  Date grouping and Today rules as in 2Do (01 §11–12).
- Desktop notifications at alert time, **even when the app is not running**.
- Add tasks in-app with a one-line natural-language input and a preview before saving.
- Vim-style keyboard control throughout.
- Sync through CalDAV on the user's Synology NAS. Data on disk is standard iCalendar.
- Runs on Omarchy and macOS.

**Non-goals (v1)**
- Tags, priorities and stars, locations, attachments and audio, sections,
  durations, user-defined smart lists, global quick-entry hotkey (planned later), multiple sync
  providers, sharing, a mobile app of our own, Dropbox or iCloud sync.

**Anti-goals (failure even if it "works")**
- Missing a reminder: a recurring task silently not appearing in Today or not notifying.
- Damaging data the 2Do iPhone app relies on: dropping `X-2DOAPP-*` properties or 2Do's metadata,
  so the phone loses start dates, checklists or repeat-from-completion.
- A proprietary or app-private storage format.
- A look that ignores the Omarchy theme.

## 3. Constraints

- Platforms: Omarchy (Arch, Hyprland, Wayland, terminal-centric; themes switch the terminal palette)
  and macOS (user's own terminal, e.g. Ghostty).
- Single user, single sync method.
- CalDAV server on a Synology NAS (Synology Calendar or a container such as Radicale).
- The 2Do iPhone app must keep working against the same CalDAV data.

## 4. Decisions made by the user

1. **Terminal app (TUI)** on both platforms. Chosen over Tauri and Quickshell+SwiftUI after
   comparing mockups (the chosen layout is reproduced in `docs/design/tui-layout.md`).
2. **CalDAV on the Synology NAS** as the single sync method and source of truth.
3. **Feature scope** as in §2 (confirmed: "looks right").
4. **Parent tasks need a manual final tick**, like 2Do. No auto-close for now.
5. **Adding tasks:** `o` opens an inline row in the list for the title, and a card holds the other
   fields; natural-language input comes later on that same row (replaced 2026-11-03, see
   [the add-and-reshape design](2026-11-03-tui-add-and-structure-design.md)). A global hotkey for
   quick entry is a later addition.
6. **2Do on the Mac is retired** once Twydo is working. Until then it supplies compatibility
   fixtures and performs the migration (§10). After that, the only writers are Twydo (Linux and macOS) and, occasionally,
   the 2Do iPhone app.
7. **Notifications are plain reminders in v1**: no Complete or Snooze buttons.
8. **App name: Twydo**, pronounced “twhy-doo”, with `tdo` as the terminal command.
   Chosen for its short, distinctive spelling and similar feel to 2Do; the name hints at
   doing things again, matching recurring tasks. This replaces the placeholder `tick`.
   The shorter command `tdo` echoes “to-do”; configuration stays under `~/.config/twydo/`.

## 5. Decisions made by me (open to change)

- **Language/toolkit:** Rust + `ratatui` (crossterm backend). One binary for Linux and macOS.
- **Storage and sync split:** the app reads and writes a local **vdir** (a folder per list, one `.ics`
  file per task, the khal/todoman convention). **`vdirsyncer`** syncs the vdir with CalDAV. The app has
  no network code, works offline, and the format is readable by other tools (todoman, khal).
  An item retains its existing filename on edits; identity comes from its UID, not its filename.
  The app reloads when files change and offers `:sync`. A shared sync runner invokes
  `vdirsyncer sync` and configured `vdirsyncer metasync` for `displayname` and `color`.
  It runs after edits and every minute in the background, including while the TUI is closed.
  Serialize runner invocations and coordinate them with local edits, undo, and recovery.
  Show last successful sync, pending changes, conflicts, and failures; retry transient failures
  with backoff. A sync failure must not disable reminders from the local data.
- **Notifications:** a separate small command (`tdo notify`) run every minute by a **systemd user
  timer** (Linux) or a **launchd agent** (macOS). It reads the vdir and calls `notify-send` (Linux,
  shows in Omarchy's mako) or `osascript` (macOS). It records what it has sent in a small state file
  to deduplicate successful deliveries across ordinary restarts. Crash recovery favours a possible
  duplicate over a silently lost reminder; exactly-once desktop delivery is not promised. Triggers:
  - each `VALARM` at its time;
  - **every open task with a due date notifies when due, even without an alert**: at its due
    time, or for date-only tasks at a configurable morning time (default 09:00);
  - one daily summary of overdue tasks at the same morning time.

  Plain reminders only: the notifier never writes task resources. Its delivery contract is in §7.
- **Repeat rules vs the iPhone:** existing rules are preserved without normalization on unrelated
  edits. A versioned compatibility matrix records preservation, execution/completion, and
  creation/editing support separately. No rule, including monthly/yearly, is presumed safe before
  verification. Unsupported operations are blocked with a reason; tasks remain visible and their
  known current due dates still drive reminders. Unrelated safe edits may proceed.
- **Parent tasks:** projects and checklists may share a visual presentation but retain their types.
  Project children keep independent dates, recurrence, Today membership, and notifications.
  Checklist items inherit the parent's scheduling and do not notify independently. Completing
  all children never closes a parent. Completing a parent with unfinished children asks for
  confirmation. Date shifts, history copies, and resets follow verified rules (§6).
  Acceptance case **P1** covers a recurring monthly project whose children carry their own dates,
  and **P2** covers an independently recurring child under a
  non-recurring project. Real titles are kept outside the repository (see `AGENTS.md`).
- **Conflicts:** do not configure unconditional `a wins` or `b wins`. Preserve both versions of a
  conflicted resource in a recoverable conflict store outside the synchronized vdir, report the
  conflict, and leave that resource unresolved until the user chooses a version. Never silently
  discard a completion, recurrence advancement, or deletion. Other resources continue syncing.
  Vdirsyncer integration must demonstrate this behaviour, including delete-versus-edit; its
  default conflict error alone does not satisfy preservation and continued progress requirements.
  Treat Linux, macOS, and iPhone as independent writers. Conflicted tasks show a warning;
  evaluate known outstanding triggers from both versions, deduplicating identical triggers.
- **Theming:** use the terminal's **16 ANSI colours** for all UI chrome (so any Omarchy theme
  applies with no code and re-colours live). List colours are the exception: they come from each
  list's CalDAV `calendar-color` and are drawn in true colour as the coloured bar.
- **Natural-language input:** dates via a chrono-style parser plus the 2Do shortcuts documented
  in 01 §16 (`3d`, `2w`, `15th`, `fri 5pm`, `every month`, `list(Bills)`).
- **Config:** TOML at `~/.config/twydo/config.toml` (vdir path, notifier settings, week start,
  work days, optional default due time, default destination list, keymap overrides).
  List groups, list/project manual ordering, sort preferences, and archive/visibility fallbacks
  live in documented TOML keyed by stable collection/task identity when no verified shared
  encoding exists. This is UI configuration, not a replacement task store. Carry it between
  desktops explicitly; it is not claimed to sync through CalDAV.
- **Migration:** no importer. First validate 2Do's upload and vdirsyncer's pull on a separate
  test collection/account using backed-up data. Production migration uses the verified flow
  (§10). *Replace Server Data* is destructive and is used only on an explicitly selected empty
  or dedicated destination. A backup and reconciliation are required before retiring Mac 2Do.

## 6. Data model (iCalendar, as 2Do writes it)

Ground truth is whatever **2Do writes to CalDAV**, because the iPhone app must keep working.
Before implementing writable model/sync operations, phase 0 must capture versioned `.ics`
fixtures and a compatibility matrix from the test server, recording Mac/iPhone versions,
server version, action sequence, and before/after resources. Include a one-time task, monthly
and yearly repeats, repeat-from-completion, a recurring project with children, a checklist,
a project with an independently recurring child, completed history, date-only and timed tasks,
start dates, multiple/absolute/relative alarms, and unknown properties. Do not put credentials
in fixtures. Record unsupported or unverified cases explicitly.

The matrix covers create, edit, complete, reopen, move, and delete. Phase 0 records the Mac/iPhone
baseline, expected resource changes, and the acceptance cases for Twydo. Before enabling a Twydo
operation, verify it in both directions, including a subsequent iPhone edit on Twydo-written data.
Passing unchanged round trips is insufficient. Capturing the baseline and acceptance cases is a
prerequisite; testing the implementation against them follows implementation and precedes release.

Expected mapping (from `docs/reference/2do/06-sync.md` §1.7, §2.6; to be confirmed by the samples):

| App concept | iCalendar |
|---|---|
| List | CalDAV calendar collection supporting `VTODO`; name `displayname`, colour `calendar-color` |
| List group | no verified CalDAV mapping; preserve metadata if present, use documented local configuration for the UI |
| Task | one `VTODO`: `UID`, `SUMMARY`, `DESCRIPTION`, `DUE` (date or date-time), `DTSTART` (start), `STATUS`/`COMPLETED`, `CREATED`, `LAST-MODIFIED` |
| Repeat | `RRULE` (mapping table in 01 §6), plus 2Do's `@repeatfromcompletion` metadata flag |
| Parent / items | items carry `RELATED-TO` = parent `UID`; parent type from 2Do metadata `@tasktype:checklist` or `project` |
| Alert | `VALARM` with `TRIGGER`; verify due/start-relative and absolute forms, including tasks without a due date |
| 2Do extras | `X-2DOAPP-*`; over CalDAV the metadata is `<2Do Meta>` URL-encoded JSON, and its `StartDate` (not `DTSTART`, a copy of `DUE`) is the start date (06 §2.6, verified in phase 0) |

**Preservation and editing:** unknown properties, parameters, and components remain intact.
Unchanged resources are not rewritten, including modification timestamps. A known field edit
updates every verified representation of that field, including metadata, while preserving
unrelated content. Phase 0 defines precedence if standard fields and metadata disagree.
New history copies receive consistent identities and parent links in both standard properties
and metadata. Never copy an opaque identity token into a new task without understanding it.

**Recurring completion:** copy-and-advance is observed in Dropbox data but remains unverified
for CalDAV. Enable it only after the fixtures establish the exact resource model. If confirmed,
save a completed occurrence and advance the live task while preserving its UID. Resolve these
cases experimentally and turn each result into an explicit input/output test:

| Case | Required verified result |
|---|---|
| Monthly on the 15th, completed three months late | Advance once versus skip missed dates; history retained |
| Monthly on the 31st; yearly on February 29; missing fifth weekday | Skip versus clamp and preservation of the original anchor |
| Repeat from completion | Completion/start anchor, date-only versus timed result, timezone |
| End after N / on a date | Remaining versus original count, inclusive boundary, final occurrence state |
| Parent with complete and incomplete children | Confirmation, copied child identities/links, reset and date-shift rules |
| Project child with its own recurrence | Interaction with parent completion/advancement without double advancement |
| Start/due offsets and relative/absolute alarms | Exactly which dates and triggers shift or are retained |
| Reopen / undo before and after synchronization | Restoration of history, live dates, child state, and repeat counts |

Do not substitute library defaults for unresolved 2Do behaviour. Completing a parent does not
implicitly establish how its children's dates or recurrence should advance.

**Recovery:** multi-resource completion, parent edits, moves, and undo use a durable operation
journal outside the vdir with stable operation/copy IDs. Write individual resources atomically.
Local sync and notifier readers must not consume partially committed operations; finish recovery
before they run. After interruption, resume or roll back without duplicate history or a lost next
occurrence. CalDAV uploads are not assumed atomic across resources: verify partial-upload and
retry behaviour, including an iPhone reading between uploads. Retain recovery information until
sync is confirmed. Undo after a remote edit uses conflict handling rather than overwriting it.

Task data remains iCalendar; delivery state, conflict copies, and the recovery journal use
documented, inspectable auxiliary formats and do not become an alternative task source of truth.

## 7. Behaviour

- **Today** = due today, overdue, and tasks whose start date has passed, subject to list visibility
  and future-start hiding. Never shows completed tasks. Project children qualify independently;
  checklist items appear under their qualifying parent. Duration/star inclusion is outside v1.
- **Date groups** (from the start of today): Overdue · In Progress · Later Today · Today · Tomorrow ·
  Day After Tomorrow · Next 7 Days · Next 2 Weeks · Next 30 Days · Future · No Due Date.
  Use 01 §12 thresholds and context rules, with fixtures for precedence: overdue first; in Today,
  started tasks not otherwise overdue group as In Progress; remaining timed tasks due later today
  group as Later Today. Use local calendar days, not elapsed 24-hour intervals, across DST.
- **Scheduled**: tasks with a future start date stay out of Today and Next 3 Days until they
  start. **Revised 2026-11-05 for issue #11:** All and ordinary lists always include future-start
  tasks under Future and tasks completed in the last 30 local calendar days including today
  under Done (newest completion first), omitting empty
  sections. No visibility controls or preferences. Children in another active/future-start/
  completed section appear separately; children in the same section retain expansion and
  item ordering. Counts remain unfinished-only, including future-start tasks. The dedicated
  Done view retains its completed total. See the
  [list visibility design](2026-11-05-issue-11-list-visibility-design.md).
- **Next 3 Days** (product decision): unfinished tasks due from the start of today through the
  end of the day after tomorrow: `[today, today + 3 calendar days)`. Exclude overdue, undated,
  archived, and future-start tasks. Project children qualify independently. This boundary is not
  claimed as a recovered 2Do rule.
- **Lists**: preserve archived/hidden state and inclusion flags where their encoding is verified;
  otherwise initialize documented local fallbacks from the pre-migration inventory. Archived
  lists are hidden from ordinary views, including Today, All, Scheduled, and Next 3 Days; their
  completed tasks remain available in Done subject to show-in-Done. Hidden lists and per-view
  exclusion flags are respected. As a v1 product decision, archived/hidden lists do not notify;
  inclusion flags for All/Today alone do not disable notifications. Make this visible in settings.
  An existing archived list (acceptance case **L1**) is a migration acceptance case.
- **Overdue** considers the due time if one is set.
- **Sorting** per list/project: due date (default), start date, creation, title, manual, ascending
  or descending. Manual order persists in the documented configuration fallback; moving an item
  inserts it at the destination's end. Only date-based sorts use date buckets.
- **Creation and editing**: require a nonempty title. New tasks use the current ordinary list,
  otherwise the configured default destination (prompt if missing). In Today they default to due
  today unless input explicitly overrides it. New children inherit the parent's list and retain
  checklist/project semantics. Allow start/due date/time editing and explicit removal; block
  removal if its effect on an existing recurrence has not been verified.
- **Default due time** is optional, disabled by default. When enabled, an explicit due-date edit
  on an untimed task applies it; recurrence advancement follows verified semantics instead.
  An explicit “date only” selection wins. 07:30 is a documented example, not a proven 2Do default.
- **List management**: create, rename, recolour, archive/unarchive, and regroup from commands/forms.
  Collection creation and metadata propagation go through the sync runner, with failure feedback.
  Group creation/rename/order and membership use the documented fallback unless phase 0 verifies
  a shared representation. V1 does not delete entire collections; individual task deletion supports undo.

### Notification delivery contract

- Identity is `(task UID, occurrence identity, trigger identity)`, not UID alone. Derive a stable
  occurrence anchor from verified fields so later occurrences of the same UID can notify.
- Multiple alerts are supported. Resolve relative triggers against the verified due/start anchor;
  absolute triggers retain their instant. Checklist items do not emit separate reminders.
- Combine an alarm and implicit due reminder for the same task/occurrence at the same instant
  into one delivery, recording both trigger identities. Separate alarm times remain separate.
- Read a consistent current snapshot before delivery; suppress completed/deleted tasks and
  superseded triggers after edits. Do not fire from a partially committed local completion.
- On first run and subsequent runs, including after sleep or sync, send outstanding triggers
  from the last 24 hours (configurable). Older still-open due obligations go in one catch-up
  summary rather than a notification per historical occurrence. Later daily overdue summaries
  remain intentional repeat reminders. Record summary delivery separately by local date and task/
  occurrence identity so older obligations do not re-notify on every minute's scan. Newly received
  older obligations may produce a supplementary summary once. Coalesce catch-up and daily summaries
  when they coincide.
- Retry delivery-command failures without marking success. Persist successful identities
  atomically; a crash between sending and recording may duplicate a reminder. Serialize notifier
  runs. Clock rollback must not replay recorded identities; forward jumps follow catch-up rules.
- Preserve floating, date-only, and zoned values distinctly. Use the configured local timezone
  for date-only morning reminders and fixture-tested DST handling for timed recurrences.
- Read list `displayname`/`color` files populated by `metasync`. Missing/invalid metadata falls
  back to collection identity and an ANSI colour; it never prevents notification delivery.

## 8. Screen and keys

Layout (mockup: `docs/design/tui-layout.md`): three panes plus a status line.
- **Left:** views (Today, All, Scheduled, Done, Next 3 Days) with counts in the form due-or-overdue|total,
  then list groups and lists, each with its colour bar.
  This count definition is a product decision: count each eligible unfinished task once,
  including independently eligible project children; checklist items do not inflate counts.
  Done shows its completed total only.
- **Middle:** the current view's tasks under date-group headers. Each row has a list-colour bar,
  a checkbox glyph (`○` task, `↺` repeating, `[n]` parent with n items), title, repeat hint,
  and a right-aligned relative date. Overdue rows are in the theme's red.
- **Right:** details of the selected task (list, due, repeat in words, alert, start, notes).
  Collapsible on narrow terminals.
- **Bottom:** vim-style mode indicator (NORMAL / INSERT / COMMAND) and key hints.

Keymap (my proposal, builder may refine; 2Do equivalents in `docs/reference/2do/04-keyboard-shortcuts.md`):

| Key | Action |
|---|---|
| `j`/`k`, `gg`/`G`, `Ctrl-d`/`Ctrl-u` | move |
| `h`/`l` | focus left / right pane |
| `Tab` | next pane |
| `x` / `Space` | toggle done |
| `o` / `O` | new task row below / above the cursor (an item, inside a parent) |
| `Enter` | on a parent, expand/collapse; on a leaf, edit |
| `i` | edit selected task, including a parent |
| `J` / `K` | move task down/up when manually sorted |
| `dd` | delete (with undo) |
| `u` / `Ctrl-r` | undo / redo |
| `t`, `T` | due today / due tomorrow |
| `]` / `[`, `}` / `{` | due +1 / −1 day, +1 / −1 week |
| `r` | edit card, on the Repeat field (any task) |
| `a` | add an item inside the selected parent (alerts move into the card later) |
| `c` + `t`/`p`/`c` | change type to task / project / checklist |
| `yy`, `p` / `P` | reserved: yank, paste below / above |
| `>>` / `<<` | reserved: indent into / outdent out of a parent |
| `m` | move to another list |
| `/`, `n`/`N` | search in view |
| `1`–`5` | Today, All, Scheduled, Done, Next 3 Days |
| `:` | command line (`:sync`, `:list <name>`, list/group management, `:conflicts`, `:q`) |
| `?` | help overlay |

## 9. Testing and success checks

- **Compatibility gate:** phase 0 supplies the versioned fixtures/matrix in §6. Daily-use release
  requires preservation of all imported resources and verified execution of every active rule in
  the user's inventory. Unverified rule editors remain unavailable with a reason.
- **Core tests:** every enabled repeat rule and edge case in §6; local-day/DST boundaries, Today,
  Next 3 Days, Scheduled precedence, grouping, archive flags, and independent project children.
- **Round trips:** an unchanged resource is byte-equal, including timestamps; an edit changes only
  intended properties and their verified metadata counterparts. Test unknown parameters/components,
  Unicode, folded lines, new identities, parent links, and conflicting metadata representations.
- **Interop:** create/edit/complete/reopen/move/delete from both Twydo and iPhone, sync, then edit
  again from the other side. Check history, next occurrence, child dates/repeats, alarms, and metadata.
- **Failures/conflicts:** concurrent desktop/phone completion, completion-versus-edit,
  delete-versus-edit, partial upload, and interruptions at each local journal stage. Both versions
  remain recoverable, unrelated tasks sync, and retries do not duplicate history or lose occurrences.
  Test undo before sync and after remote modification.
- **Notifier:** separate occurrences sharing a UID, simultaneous alarm/due coalescing, multiple
  alarms, restart deduplication, failed delivery retry, first run, sleep, late sync, clock changes,
  DST, completion/deletion before delivery, conflict snapshots, and catch-up summaries. Verify the
  documented duplicate-on-crash tradeoff rather than asserting exactly-once delivery.
- **Setup:** background sync/notifier run with the TUI closed on both platforms; names/colours
  propagate through metasync; offline failures are visible and local reminders continue.
- **Migration reconciliation:** compare source inventory and destination tasks, completed history,
  parent links, recurrence/end counts, start/due dates and times, alarms, and archived lists. Explain
  special-list count differences rather than expecting each Dropbox `.col` to become a collection.
  Verify acceptance cases P1 (with its children), P2 and L1.

## 10. Rough implementation plan

0. **Compatibility gate:** back up the 2Do database and Dropbox snapshot and record the inventory.
   Set up an isolated CalDAV test account/collection on the NAS (confirm Synology accepts `VTODO`,
   otherwise use Radicale). Use a backed-up/restorable 2Do test setup so changing providers cannot
   replace production data or connect the production iPhone database to an incomplete test server.
   Validate the migration flow, set up vdirsyncer including metasync, and capture §6 fixtures.
   Record bidirectional Mac/iPhone behaviour and recurrence edge-case results in the matrix.
   Complete this gate before writable core implementation; unresolved cases remain explicit.
1. **Core library:** preserving vdir read/write, verified metadata edits and recurrence handling,
   recoverable completion/undo, project/checklist semantics, Today and date grouping. Tests from §9.
2. **TUI shell:** three-pane layout, views, list colours, ANSI theming, navigation keys.
3. **Editing:** toggle done (incl. parents), date shortcuts, edit form, delete and undo, move.
4. **Adding and reshaping tasks** (design: `2026-11-03-tui-add-and-structure-design.md`): phase 1
   inline add, the in-place card, type changes and all repeat kinds; then the later features
   (indent, yank and paste, list management, Project View); then natural-language input with
   preview.
5. **Sync/conflicts and notifier:** background runner, metasync, conflict preservation/resolution,
   delivery state and catch-up contract, systemd and launchd setup. Prove failure recovery.
6. **Interop and production migration:** pass §9 with the iPhone on the test setup; take fresh
   backups, select the dedicated production destination, migrate through 2Do, and reconcile the
   inventory before connecting the production iPhone with the verified initial-sync choice.
   Begin daily use only after all active obligations pass. Keep backups and a tested restore path.
   Disable Mac 2Do sync and retire it only after acceptance, with no pending/unresolved operations.
7. Later: global quick-entry hotkey (Hyprland bind or macOS shortcut launching `tdo add`).

## 11. Left to the builder

Crate choices beyond ratatui (iCalendar parser, RRULE, date parsing, file watching), module layout,
config format details, exact visual spacing, help text, error and empty states, packaging
(AUR / Homebrew tap), and anything marked "my proposal" above. These choices cannot override
the compatibility gate, preservation, conflict, recovery, or notification contracts.

Technical references for the vdir integration:
[storage format and atomic writes](https://vdirsyncer.pimutils.org/en/latest/vdir.html),
[conflict configuration](https://github.com/pimutils/vdirsyncer/blob/main/docs/config.rst),
[metadata synchronization](https://github.com/pimutils/vdirsyncer/blob/main/docs/tutorial.rst).
