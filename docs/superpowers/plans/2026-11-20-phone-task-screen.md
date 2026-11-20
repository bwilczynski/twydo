# Phone task screen (Plan A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tapping a row on the phone pushes a full task screen with everything the TUI details
pane shows and every child of a project or checklist, each completable and reopenable; the
new-task sheet becomes a full-screen sheet sliding up from the bottom.

**Architecture:** A new daemon read `tasks.get` returns one task worded for display (shared
wording with the TUI, moved into `twydo_core::details`) plus its children. The gateway lets
phones and displays call it and lets phones submit `Reopen`. The PWA gains a task signal in its
data source, a hash/history route stack, a `TaskScreen` component replacing `DetailSheet`, and a
restyled `CaptureSheet`.

**Tech Stack:** Rust (twydo-core, twydo-daemon, twydo-gateway), schemars/OpenRPC, Preact +
@preact/signals, Vitest + Testing Library, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-20-phone-detail-and-editing-design.md` (§5 Plan A,
§6.1 without `form`, §7 Plan A rows, §8.1, §8.2, §8.3 last bullet, §8.5, §9, §10).

## Global Constraints

- Thin client: the PWA formats and lays out only; it never interprets iCalendar, 2Do metadata or
  task kinds beyond the fields `tasks.get` returns.
- New methods and types are additions to `schema/tdo.openrpc.json` and pass the released-schema
  test (`cargo test -p twydo-daemon --test schema`); regenerate with
  `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema`, then `cd web && npm run gen`.
- Phone role after this plan: methods `views.get`, `views.list`, `tasks.get`, `events.subscribe`,
  `events.unsubscribe`, `op.submit`, `op.undo`; ops `create` (unchanged checks), `complete`,
  `reopen`. Display role: `views.get`, `views.list`, `tasks.get`, `events.subscribe`,
  `events.unsubscribe`; no ops. Everything else stays refused (kind `invalid`).
- Test fixtures are synthetic; no personal data. Tests isolate `XDG_CONFIG_HOME` and
  `TWYDO_SOCKET`. No device or server rounds.
- Never put a cargo target dir under `/tmp` (quota tmpfs; rust-lld SIGBUS). Use the default
  `target/` or `~/.cache`.
- Commits pass commitlint: `type(scope): lowercase subject`, no trailing full stop, header ≤ 100,
  body lines ≤ 100. No agent attribution lines.
- Copy fixed by the spec: banner `That task is gone`; conflict line
  `This task has a sync conflict. Resolve it in tdo.`; banners `Completed "<title>"` (existing)
  and `Reopened "<title>"`.
- Before finishing: `mise run test`, `mise run lint`, `mise run web-check`, `mise run web-e2e`,
  `python3 tools/check_links.py`.

## Review Focus

1. **A task deleted or moved away while its screen is open** (sync, `tdo`, 2Do): the screen pops
   with "That task is gone" instead of spinning or showing stale data. → Task 5 test
   `pops a task that is gone`.
2. **Ticking a child of a recurring checklist or a child of a completed parent**: the gate's
   family rules decide; a refusal must untick the row and show the message, never leave it
   ticked. → Task 5 test `a refused child toggle rolls back`.
3. **Reload or opening the app on `#/task/<uid>`** after the task is gone or before the socket
   opens: shows the spinner then the task, or pops to the tab root with the banner; never a blank
   screen. → Task 4 test `reload on a task route` and Task 5 `pops a task that is gone`.
4. **Offline while on a task screen**: all checkboxes disabled and the screen dimmed, the last
   result still shown. → Task 5 test `offline disables the task screen`.
5. **Undo after ticking a child from the task screen** reopens that child and the screen shows it
   open again after the refetch. → Task 6 e2e `tick and untick a checklist item` (Undo step).

---

## File Structure

- `crates/twydo-core/src/details.rs` (create): pure wording shared by the TUI details pane and
  `tasks.get`: shown kind, repeat text, alert lines, children order.
- `crates/twydo-core/src/lib.rs` (modify): `pub mod details;`.
- `crates/tdo/src/ui/details.rs`, `crates/tdo/src/ui/tasks.rs` (modify): call the core helpers.
- `crates/twydo-daemon/src/protocol/tasks.rs` (create): `tasks.get` params and result types.
- `crates/twydo-daemon/src/protocol/mod.rs` (modify): `pub mod tasks;`.
- `crates/twydo-daemon/src/server/tasks.rs` (create): `get`.
- `crates/twydo-daemon/src/server/views.rs` (modify): expose `list_color` and a `due_flags`
  helper as `pub(crate)`.
- `crates/twydo-daemon/src/server/dispatch.rs`, `src/server/mod.rs`, `src/schema.rs` (modify).
- `crates/twydo-daemon/tests/tasks.rs` (create).
- `crates/twydo-gateway/src/bridge.rs`, `crates/twydo-gateway/tests/bridge.rs` (modify).
- `web/src/data.ts`, `web/src/data.test.ts` (modify): task signal, `Visible` task target,
  `reopen`, `onTaskGone`.
- `web/src/route.ts`, `web/src/route.test.ts` (create): hash parsing and building.
- `web/src/components/TaskScreen.tsx` (create); `web/src/components/DetailSheet.tsx` (delete).
- `web/src/App.tsx`, `web/src/components/App.test.tsx`, `web/src/components/CaptureSheet.tsx`,
  `web/src/styles.css` (modify).
- `web/e2e/setup.ts`, `web/e2e/app.spec.ts` (modify).
- `docs/gateway.md`, `docs/protocol.md` (modify).

---

### Task 1: Shared details wording in `twydo-core`

**Files:**
- Create: `crates/twydo-core/src/details.rs`
- Modify: `crates/twydo-core/src/lib.rs`, `crates/tdo/src/ui/details.rs:42-72,106-120`,
  `crates/tdo/src/ui/tasks.rs:43-50`
- Test: unit tests in `crates/twydo-core/src/details.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces (all `pub` in `twydo_core::details`):
  - `fn shown_kind(snap: &Snapshot, idx: usize) -> TaskKind`: the task's kind, except a plain
    task with children shows as `Project` (moved from `tdo::ui::tasks::shown_kind`).
  - `fn repeat_text(t: &Task, tz: Tz) -> Option<String>`: `None` without `rrule`; else the
    `describe(...)` text the TUI shows today, or `"unsupported rule"` when it does not parse.
  - `fn alert_lines(t: &Task, tz: Tz) -> Vec<String>`: `describe_alarm` per alarm, in order.
  - `fn child_order(snap: &Snapshot, tz: Tz, idx: usize) -> Vec<usize>`: open children
    (status not `Completed`/`Cancelled`) sorted with `compare_items`, then the rest by the
    instant their `completed` resolves to in `tz`, newest first; those without `completed` last,
    in `snap.children(idx)` order.

- [ ] **Step 1: Write the failing tests** in `details.rs`, building a `Snapshot` the way existing
  core view tests do (synthetic tasks, `RELATED-TO;RELTYPE=PARENT`):
  - `shown_kind_plain_parent_is_project`: plain task with one child → `TaskKind::Project`; a
    checklist stays `Checklist`; a childless task stays `Task`.
  - `repeat_text_none_and_unsupported`: no rrule → `None`; `RRULE:FREQ=DAILY` → `Some` text equal
    to what `describe(&parse_rrule(..), ..)` gives; `RRULE:FREQ=SECONDLY` → `"unsupported rule"`.
  - `child_order_open_then_newest_completed`: children `a` (open, due later), `b` (open, due
    earlier), `c` (completed 2026-11-10), `d` (completed 2026-11-12), `e` (completed, no
    `COMPLETED`) → `[b, a, d, c, e]`.
- [ ] **Step 2: Run** `cargo test -p twydo-core details::` — expected FAIL (module missing).
- [ ] **Step 3: Implement** the four functions; add `pub mod details;` to `lib.rs`.
- [ ] **Step 4: Switch the TUI to them:** `ui/tasks.rs::shown_kind` calls
  `details::shown_kind(&app.snapshot, idx)`; `ui/details.rs` uses `repeat_text` and
  `alert_lines` for the Repeat and Alert fields (alerts keep the label on the first line only).
  The TUI's children section keeps its own open/done partition and count; no visible change.
- [ ] **Step 5: Run** `cargo test -p twydo-core details:: && cargo test -p tdo` — expected PASS.
- [ ] **Step 6: Commit** `refactor(core): share task details wording with the tui`.

### Task 2: `tasks.get` in the daemon

**Files:**
- Create: `crates/twydo-daemon/src/protocol/tasks.rs`, `crates/twydo-daemon/src/server/tasks.rs`,
  `crates/twydo-daemon/tests/tasks.rs`
- Modify: `crates/twydo-daemon/src/protocol/mod.rs`, `src/server/mod.rs`,
  `src/server/dispatch.rs:165-166`, `src/server/views.rs` (make `list_color` `pub(crate)`; extract
  `pub(crate) fn due_flags(snap: &Snapshot, idx: usize, now: DateTime<Tz>) -> (bool, bool)`
  returning `(overdue, due_today)` from the logic in `row()` and use it there),
  `src/schema.rs:81-87`, `schema/tdo.openrpc.json`, `docs/protocol.md`

**Interfaces:**
- Consumes: Task 1's `details::*`; `RowList` from `protocol::views`.
- Produces (`twydo_daemon::protocol::tasks`, all `Serialize, Deserialize, JsonSchema`, doc
  comments as in `protocol/views.rs`):

```rust
pub struct TasksGetParams { pub uid: String }
pub struct TasksGetResult {
    pub epoch: String, pub revision: u64,
    pub now: DateTime<FixedOffset>, pub today: NaiveDate,
    pub task: TaskDetail,
    /// Every child: open ones in sibling order, then completed ones, newest first.
    pub children: Vec<ChildRow>,
}
pub struct TaskDetail {
    pub uid: String, pub title: String,
    /// The shown kind: a plain task with items is a project.
    pub kind: TaskKind, pub list: RowList,
    pub due: Option<String>, pub overdue: bool, pub due_today: bool,
    pub start: Option<String>, pub repeat: Option<String>, pub alerts: Vec<String>,
    pub warnings: Vec<String>, pub blocked: Option<String>, pub conflict: bool,
    pub note: Option<String>, pub completed: bool, pub pending: bool,
    pub parent: Option<TaskParent>,
}
pub struct TaskParent { pub uid: String, pub title: String, pub kind: TaskKind }
pub struct ChildRow {
    pub uid: String, pub title: String, pub kind: TaskKind,
    pub due: Option<String>, pub overdue: bool, pub due_today: bool,
    pub completed: bool, pub pending: bool, pub has_note: bool, pub child_count: u32,
}
```

- Produces: `twydo_daemon::server::tasks::get(shared: &Arc<Shared>, p: TasksGetParams) ->
  Result<Value, RpcError>`; dispatch `"tasks.get"`; schema entry
  `m("tasks.get", false, of::<TasksGetParams>, of::<TasksGetResult>)` after `views.get`.

Field rules: `due`/`start` via `format::relative_date` (start shown whenever set, unlike rows);
`completed` is status ≠ Open; `note` is `description` with blank text as `None`; `has_note` the
same test; `conflict` from `admin::unresolved_conflict_uids`; `pending` from `snap.is_pending`;
`kind` (detail and child) from `details::shown_kind`; `child_count` = `snap.children(c).len()`.
Unknown uid → `RpcError::refused(ErrorKind::Invalid, "no such task")`.

- [ ] **Step 1: Write the failing tests** in `tests/tasks.rs`, reusing the `ics`, `morning`,
  `start` helpers' pattern from `tests/views.rs` (clock 2026-11-17 09:00 UTC, tz UTC):
  - `checklist_with_children`: checklist `chk` (`X-2DOAPP-METADATA:@tasktype:checklist`, due
    `20261117T170000`, `DESCRIPTION:Bring the form`), items `i1` open, `i2` open with
    `DESCRIPTION`, `i3` `STATUS:COMPLETED` + `COMPLETED:20261116T100000Z`. Assert
    `task.kind == Checklist`, `task.due == Some("Today 17:00")`, `due_today`, `note`,
    `children` uids `[i1, i2, i3]` (open order per `compare_items`), `i3.completed`,
    `i2.has_note`, `parent == None`, and `epoch`/`revision` equal `daemon.info`/`snapshot.get`.
  - `item_names_its_parent`: `tasks.get` on `i1` → `parent == {uid: chk, title, kind: Checklist}`
    and `children` empty.
  - `repeat_and_alerts_worded_like_the_tui`: a daily-repeating task with a
    `VALARM` `TRIGGER:-PT15M` → `repeat == details::repeat_text(..)` and `alerts ==
    details::alert_lines(..)` computed from `fx.snapshot()`.
  - `conflict_and_pending_flags`: reuse the conflict setup from `tests/views.rs`
    (`ConflictStore`) → `task.conflict`; a queued edit laid over (as views.rs does) →
    `task.pending`.
  - `unknown_uid_is_invalid`: `kind(&err) == ErrorKind::Invalid`.
- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test tasks` — expected FAIL (no method).
- [ ] **Step 3: Implement** the protocol types, `server::tasks::get` (snapshot and clock read as
  `views::get` does), `due_flags`, dispatch and schema entry.
- [ ] **Step 4: Regenerate the schema:** `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema`,
  then `cargo test -p twydo-daemon` — expected PASS, released-schema test included.
- [ ] **Step 5: Document** `tasks.get` in `docs/protocol.md` next to `views.get` (params, result
  outline, `invalid` for an unknown uid, refetch on `data.changed`/`status.changed`).
- [ ] **Step 6: Commit** `feat(daemon): add tasks.get for one task and its children`.

### Task 3: Gateway allows `tasks.get` and `reopen`

**Files:**
- Modify: `crates/twydo-gateway/src/bridge.rs:14-27,46-60,108-115`,
  `crates/twydo-gateway/tests/bridge.rs`, `docs/gateway.md:115-116`

**Interfaces:**
- Consumes: `TasksGetParams` from Task 2.
- Produces: `inbound` forwards `tasks.get` for both roles with params re-serialized from
  `TasksGetParams` (unknown fields dropped; missing/non-string `uid` → `invalid`);
  `phone_may_submit` returns true for `Op::Reopen { .. }`.

- [ ] **Step 1: Write the failing tests** in `tests/bridge.rs`:
  - `tasks_get_allowed_for_both_roles`: for Phone and Display,
    `run(role, "tasks.get", Some(json!({"uid": "u1", "x": 1})))` →
    `("tasks.get", json!({"uid": "u1"}))`; `Some(json!({}))` and `None` → `assert_refused`.
  - `phone_may_reopen`: `run(Role::Phone, "op.submit", submit(json!({"Reopen": {"uid": "u"}})))`
    is ok; the same for Display is refused.
  - Extend `delete_move_edit_refused` with `{"Move": {"uid": "u", "to": "c"}}`,
    `{"Save": {"uid": "u", "changes": [], "to": null}}` and
    `{"Reparent": {"uid": "u", "collection": "c", "parent": null}}`.
- [ ] **Step 2: Run** `cargo test -p twydo-gateway --test bridge` — expected FAIL.
- [ ] **Step 3: Implement** (add `"tasks.get"` to `PHONE` and `DISPLAY`, a `"tasks.get"` arm
  beside `"views.get"`, `Op::Reopen { .. } => true`).
- [ ] **Step 4: Run** `cargo test -p twydo-gateway` — expected PASS.
- [ ] **Step 5: Update** `docs/gateway.md`: both roles may call `tasks.get`; the phone may
  complete and reopen any task and capture plain tasks.
- [ ] **Step 6: Commit** `feat(gateway): let devices read a task and phones reopen`.

### Task 4: Data source and route for a task screen

**Files:**
- Create: `web/src/route.ts`, `web/src/route.test.ts`
- Modify: `web/src/data.ts`, `web/src/data.test.ts`, `web/src/gen/protocol.d.ts` (regenerated by
  `npm run gen`)

**Interfaces:**
- Consumes: `TasksGetResult` from the generated types.
- Produces (`data.ts`):
  - `export type TaskRef = { task: string }`;
    `export type Visible = ViewRef | "views" | TaskRef`;
    `export const isTask = (v: Visible): v is TaskRef`.
  - `DataSource.task(uid: string): ReadonlySignal<TasksGetResult | undefined>`.
  - `DataSource.refresh(v: Visible)` fetches `tasks.get` for a `TaskRef`.
  - `DataSource.onTaskGone(cb: (uid: string) => void): () => void`: fires once when the visible
    task's `tasks.get` is refused `invalid`; its signal is cleared and the visible target is left
    as is (the screen pops it).
  - `DataSource.reopen(uid: string): Promise<WriteResult>`: `op.submit` with
    `{ Reopen: { uid } }`, retry key `reopen:<uid>`, refusal kind `{ kind: "reopen", uid }`.
  - `Refusal` gains `| { kind: "reopen"; uid: string; message: string }`.
  - Writes while a task is visible use that task result's `epoch`/`revision` as `based_on`.
- Produces (`route.ts`): `taskHash(uid: string): string` → `#/task/<encodeURIComponent(uid)>`;
  `parseTaskHash(hash: string): string | undefined` (inverse; anything else → `undefined`).

- [ ] **Step 1: Write the failing tests:**
  - `route.test.ts`: round trip for a uid with `/`, `@` and spaces; `""`, `#/`, `#/task/` →
    `undefined`.
  - `data.test.ts` (FakeWebSocket handlers as in existing tests):
    `fetches tasks.get for a visible task` (signal filled; `sent` has
    `tasks.get {uid}`); `a write from a task screen is based on the task result` (complete while
    `{task}` visible → `based_on` equals the task result's epoch/revision);
    `reopen submits Reopen`; `a gone task fires onTaskGone once` (`tasks.get` fails with
    `FakeRpcError(…, "invalid")`; callback once with the uid; signal `undefined`);
    `data.changed refetches the visible task`.
- [ ] **Step 2: Run** `cd web && npm run gen && npx vitest run src/route.test.ts src/data.test.ts`
  — expected FAIL.
- [ ] **Step 3: Implement** in `data.ts` (task signals in a `Map<string, Signal>` like views;
  `sameTarget` and the `fetchVisible` loop learn `TaskRef`) and `route.ts`.
- [ ] **Step 4: Run** the same command — expected PASS; then `npm run check`.
- [ ] **Step 5: Commit** `feat(web): fetch one task and reopen from the data source`.

### Task 5: Task screen and navigation

**Files:**
- Create: `web/src/components/TaskScreen.tsx`
- Delete: `web/src/components/DetailSheet.tsx`
- Modify: `web/src/App.tsx`, `web/src/components/App.test.tsx`, `web/src/styles.css`

**Interfaces:**
- Consumes: Task 4's `task`, `refresh({task})`, `onTaskGone`, `reopen`, `complete`,
  `taskHash`/`parseTaskHash`.
- Produces: `TaskScreen` props:

```ts
{
  result: TasksGetResult;
  /** Target state shown for a task while its write is in flight (true = done). */
  toggled: Map<string, boolean>;
  busy: Set<string>;
  disabled: boolean;
  backLabel: string;
  onBack: () => void;
  onToggle: (uid: string, title: string, done: boolean) => void;
  onOpen: (uid: string) => void;
}
```

Behaviour (spec §8.1–8.2, Plan A): App keeps `stack: string[]` of open task uids. Opening a row
or child does `history.pushState({ stack }, "", taskHash(uid))`; `popstate` sets the stack from
`event.state?.stack`, else from `parseTaskHash(location.hash)` (one entry) or `[]`. On load, a
task hash starts with that one-entry stack. The back button (`<ChevronLeftIcon/>` + label) calls
`history.back()` when this page pushed the entry, else replaces the URL with the tab root and
clears the stack. `backLabel` is the previous stack entry's task title, else the view title
(`titleOf`), else `Views`. Tapping the active tab clears the stack (`history.go(-pushed)` when
entries were pushed, `replaceState` otherwise). While the stack is non-empty, the visible target
is `{ task: top }`, the FAB is hidden and the tab bar stays. A gone task pops one entry and
shows the banner `That task is gone`. The screen shows: a large title with its own checkbox
(`role="checkbox"`, `aria-label` `Complete <title>` when open, `Reopen <title>` when done), a
`<kind> · <list name>` line with the list colour dot (kind word omitted for `Task`), a `<dl>` of
Due, Repeat, Alert (one `dd` per line), Start, and the parent (`In`, a button opening it),
omitting empty ones; warnings and `blocked` as `!` lines; the conflict sentence; an `Items`
card (`Items · N open`) whose rows reuse `TaskRow`'s look with a checkbox labelled the same way
and enabled for completed children, with the waiting dot for `pending` rows (and for the task
itself beside its title); a `Notes` card with the full note, line breaks kept and
`http(s)://` URLs as `<a target="_blank" rel="noopener">`. Toggling: complete for an open task,
`reopen` for a done one; banner `Completed "<t>"` / `Reopened "<t>"` with Undo; a refusal rolls
the row back and shows the message. Offline: `is-offline` dimming and every checkbox disabled.

- [ ] **Step 1: Write the failing tests** in `App.test.tsx` (extend the fake `DataSource` with
  `task`, `reopen`, `onTaskGone`; a `taskResult(...)` builder like `mkView`):
  - `tapping a row pushes the task screen`: click the row body → heading with the title,
    `location.hash === "#/task/u1"`, back button labelled with the view title, FAB absent.
  - `shows every detail field`: Due, Repeat, Alert lines, Start, `In <parent>`, a warning, the
    conflict sentence, note text with a link whose `href` is the URL.
  - `ticks and unticks children`: open child → `ds.complete("c1")`; completed child checkbox is
    enabled, labelled `Reopen <title>`, click → `ds.reopen("c2")`; banner
    `Reopened "<title>"`.
  - `a refused child toggle rolls back`: `complete` rejects → checkbox unchecked, message shown.
  - `opening a child pushes its screen; back returns to the parent` (back label is the parent's
    title; `popstate` restores).
  - `reload on a task route`: start with `location.hash = taskHash("u1")` → task screen shown;
    back goes to the tab root.
  - `pops a task that is gone`: trigger `onTaskGone("u1")` → list screen and banner
    `That task is gone`.
  - `offline disables the task screen`: connection `closed` → all checkboxes disabled.
  - `tapping the active tab pops to the root`.
- [ ] **Step 2: Run** `cd web && npx vitest run src/components/App.test.tsx` — expected FAIL.
- [ ] **Step 3: Implement** `TaskScreen.tsx`, wire it into `App.tsx`, delete `DetailSheet.tsx`
  and its CSS (`.sheet dl/dt/dd`, `.sheet h2` if unused), add styles in the existing card and
  large-title idiom.
- [ ] **Step 4: Run** `npm run check` — expected PASS.
- [ ] **Step 5: Commit** `feat(web): open a full task screen with its items`.

### Task 6: Full-screen new-task sheet and end-to-end checks

**Files:**
- Modify: `web/src/components/CaptureSheet.tsx`, `web/src/styles.css`,
  `web/src/components/App.test.tsx`, `web/e2e/setup.ts:165-175`, `web/e2e/app.spec.ts`,
  `docs/gateway.md`

**Interfaces:**
- Consumes: Task 5's screen; existing `CaptureSheet` props unchanged.
- Produces: `CaptureSheet` renders `<form class="sheet full" aria-label="New task">` with a
  header `Cancel` / `New task` (`h2`) / `Add`, filling the viewport above the safe-area insets,
  sliding up from the bottom (`transform: translateY(100%)` → `0`, about 250 ms, none under
  `prefers-reduced-motion: reduce`); the note area grows to fill the remaining height. Tapping
  outside no longer applies (there is no outside).
- `e2e/setup.ts`: `ics(uid, summary, due?, extra = "")` appends `extra` lines; seed a synthetic
  checklist `chk1` "Synthetic packing" (`X-2DOAPP-METADATA:@tasktype:checklist`) in `inbox`
  with items `chk1a` "Synthetic socks" and `chk1b` "Synthetic charger"
  (`RELATED-TO;RELTYPE=PARENT:chk1`).

- [ ] **Step 1: Write the failing tests:**
  - `App.test.tsx` `new task sheet is full screen`: after tapping `New task`, the form has class
    `full` and a heading `New task`; Cancel closes it; existing capture tests still pass.
  - `app.spec.ts` `tick and untick a checklist item`: open "Synthetic packing" from Inbox →
    heading visible, `Items · 2 open`; tick `Complete Synthetic socks` → banner
    `Completed "Synthetic socks"`; reload → `Reopen Synthetic socks` checked; tap it → banner
    `Reopened "Synthetic socks"`, unchecked; tick again, press Undo → unchecked after refetch;
    browser back → Inbox heading. Must not change the counts other tests assert (they target
    other tasks; adjust the Inbox count assertion if one exists).
  - `app.spec.ts` `new task sheet slides up full screen`: tap `New task` → the form's bounding
    box height equals the viewport height (±1 px); add "Synthetic captured" → banner.
- [ ] **Step 2: Run** `npx vitest run src/components/App.test.tsx` and
  `mise run web-e2e` — expected FAIL on the new tests.
- [ ] **Step 3: Implement** the sheet markup and CSS; extend `ics` and the seed.
- [ ] **Step 4: Run** `mise run web-check && mise run web-e2e` — expected PASS (both
  light and dark projects).
- [ ] **Step 5: Update** `docs/gateway.md` "what the phone shows" section: tapping a task opens
  its screen with its items; items can be ticked and unticked; New task opens full screen.
  Run `python3 tools/check_links.py`.
- [ ] **Step 6: Final verification:** `mise run test && mise run lint && mise run web-check &&
  mise run web-e2e` — all PASS.
- [ ] **Step 7: Commit** `feat(web): open new task as a full-screen sheet`.
