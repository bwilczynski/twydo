# Phone editing (Plan B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The phone edits a task in a full-screen form with the TUI card's fields, creates
tasks with list, type, dates, repeat and notes, adds items to a project or checklist, and
deletes tasks, each as one write and one undo step decided by the daemon.

**Architecture:** The TUI's edit-card model moves from `crates/tdo/src/form.rs` into
`twydo_core::form` (key handling, focus, discard prompt and error line stay in `tdo` as a thin
wrapper). The daemon adds a `form` block to `tasks.get` and a pure read `tasks.form` that runs
the phone's typed text through the core form and returns fields, previews, the finished `Op`
and a rebase when the task changed elsewhere. The gateway lets phones call `tasks.form` and
submit `save`, any `create` and `delete`. The PWA replaces `CaptureSheet` with a `TaskForm`
sheet that asks `tasks.form` as the user types and submits the returned `Op`.

**Tech Stack:** Rust (twydo-core, tdo, twydo-daemon, twydo-gateway), schemars/OpenRPC, Preact +
@preact/signals, Vitest + Testing Library, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-20-phone-detail-and-editing-design.md` (§5 Plan B,
§6.1 `form`, §6.2, §6.3, §7, §8.2 Edit/Add item/Delete, §8.3, §8.4, §8.5, §9, §10).

## Global Constraints

- Thin client: the PWA never decides which fields a kind has, how typed dates/repeats/alerts
  read, or what a save writes. It shows `fields`, `previews` and `error` from the daemon and
  submits the daemon's `op` unchanged. The only form logic in TypeScript is "values differ from
  initial" (the dirty check for the discard prompt).
- The TUI's behaviour does not change: `crates/tdo/tests/form.rs` and `app_keys.rs` pass with
  at most import-path edits.
- New methods and types are additions to `schema/tdo.openrpc.json` and pass the released-schema
  test; regenerate with `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema`, then
  `cd web && npm run gen`. A fresh worktree needs `cd web && npm ci && npm run gen` first.
- Phone role after this plan: methods `views.get`, `views.list`, `tasks.get`, `tasks.form`,
  `events.subscribe`, `events.unsubscribe`, `op.submit`, `op.undo`; ops `Create` (any
  `NewTask`), `Save`, `Complete`, `Reopen`, `Delete`. Display role unchanged (no `tasks.form`,
  no ops). Still refused for the phone: `Edit`, `Move`, `CreateAt`, `Reorder`, `Reparent`
  (refusal kind `invalid`, message `not allowed for this device`).
- Copy fixed by the spec: banners `Saved "<title>"`, `Added "<title>"` (existing capture keeps
  `Added "<title>" to <list>`), `Deleted "<title>"`, each with Undo; notice
  `Changed since you opened it` with a `Review` button; prompt `Discard changes?`; delete
  sheet `Delete "<title>"?`, for a parent `Delete "<title>" and its N items?`; tasks.form error
  strings are the TUI's (`no changes`, `title must not be empty`, …).
- Debounce for `tasks.form` while typing: 250 ms.
- Test fixtures synthetic only. Tests isolate `XDG_CONFIG_HOME` and `TWYDO_SOCKET`. No device or
  server rounds. Never put a cargo target dir under `/tmp`.
- Commits pass commitlint: `type(scope): lowercase subject`, no trailing full stop, header ≤ 100,
  body lines ≤ 100. No agent attribution lines (no `Co-Authored-By`).
- Before finishing: `mise run test`, `mise run lint`, `mise run web-check`, `mise run web-e2e`,
  `python3 tools/check_links.py`.

## Review Focus

1. **The task changes elsewhere while the form is open** (sync, `tdo`): the form must not save
   over it silently; `changed_since` shows the notice, Save stays disabled until Review, and
   Review names overwritten/dropped fields. → Task 3 test `changed_elsewhere_rebases`, Task 6
   test `changed elsewhere asks for review before saving`.
2. **Responses arriving out of order while typing fast**: an older `tasks.form` answer must
   never replace a newer one's previews or `op` (Save would submit stale text). → Task 6 test
   `ignores an out-of-date check`.
3. **Saving an unwritable repeat rule or an alert Twydo can't edit untouched**: the form opens
   with `(kept)` / the read-only preview and the Alert input disabled, and saving another field
   writes nothing to them. → Task 2 test `form_block_keeps_unwritable_rule`, Task 6 test
   `alert input is disabled when not editable`.
4. **A queued save or delete refused later** (`op.finished` refused): the banner names it, and
   for a save `Edit` reopens the form with the typed values, never losing them. → Task 7 test
   `a refused queued save reopens the form`.
5. **Deleting the task whose screen is open, then Undo**: the screen pops before the refetch
   says the task is gone (no "That task is gone" banner replacing `Deleted "<title>"`), and
   Undo brings it back in the list. → Task 7 test `delete pops without the gone banner`, Task 8
   e2e `delete with undo`.

---

## File Structure

- `crates/twydo-core/src/form.rs` (create): the form model moved from `tdo` plus the
  serializable `FormValues`, `Field` (serde, JsonSchema) and the thin-client helpers.
- `crates/twydo-core/src/lib.rs` (modify): `pub mod form;`.
- `crates/tdo/src/form.rs` (modify): `EditForm` wraps `twydo_core::form::Form` with focus,
  discard prompt, error and keys; re-exports the moved names.
- `crates/twydo-daemon/src/protocol/tasks.rs` (modify): `FormState`, `FieldPreview`,
  `TasksFormParams`, `NewTarget` (wire), `TasksFormResult`, `RebasedForm`; `TasksGetResult.form`.
- `crates/twydo-daemon/src/server/tasks.rs` (modify): `form_state`, `get` fills `form`, `form`.
- `crates/twydo-daemon/src/server/dispatch.rs`, `src/schema.rs`, `schema/tdo.openrpc.json`.
- `crates/twydo-daemon/tests/tasks.rs` (modify).
- `crates/twydo-gateway/src/bridge.rs`, `crates/twydo-gateway/tests/bridge.rs` (modify).
- `web/src/data.ts`, `web/src/data.test.ts` (modify).
- `web/src/formState.ts`, `web/src/formState.test.ts` (create): the form's state hook.
- `web/src/components/TaskForm.tsx` (create), `web/src/components/ConfirmSheet.tsx` (create);
  `web/src/components/CaptureSheet.tsx` (delete).
- `web/src/components/TaskScreen.tsx`, `web/src/App.tsx`, `web/src/components/App.test.tsx`,
  `web/src/styles.css` (modify).
- `web/e2e/app.spec.ts` (modify).
- `docs/protocol.md`, `docs/gateway.md` (modify).

---

### Task 1: Move the form model into `twydo-core`

**Files:**
- Create: `crates/twydo-core/src/form.rs`
- Modify: `crates/twydo-core/src/lib.rs`, `crates/tdo/src/form.rs`, `crates/tdo/src/app.rs`,
  `crates/tdo/src/app/newrow.rs`, `crates/tdo/src/ui/card.rs` (only where field access needs
  it)
- Test: unit tests in `crates/twydo-core/src/form.rs`; `crates/tdo/tests/form.rs` unchanged
  apart from imports

**Interfaces:**
- Produces (`twydo_core::form`, all `pub`):
  - `enum Field { Title, List, Type, Due, Start, Repeat, Alert, Notes }` with `name()`, now
    also `Serialize, Deserialize, JsonSchema` with `#[serde(rename_all = "snake_case")]`.
  - `struct FormValues { title, list, kind: TaskKind, due, start, repeat, alert, notes }`
    (the private `Values`, now public; strings; `Serialize, Deserialize, JsonSchema, Clone,
    PartialEq, Eq, Debug`).
  - `struct Rebased { overlap: Vec<Field>, dropped: Vec<Field> }`, `struct NewTarget` (as
    today), `const NO_CHANGES`, `const ALERT_READ_ONLY`.
  - `struct Form` with today's public data fields of `EditForm` except `focus`, `error`,
    `discarding`: `insertion`, `uid`, `title`, `list`, `kind`, `due`, `start`, `repeat`,
    `alert`, `notes`, `item_of`; private ones as today.
  - Moved unchanged: `Form::open(&Task, Option<TaskKind>, Tz)`, `Form::new_task(NewTarget,
    String, Tz)`, `fields()`, `keeps_rule()`, `changes(..)`, `save(..)`, `set_notes`,
    `set_list`, `rebase(&self, onto: Form) -> (Form, Rebased)`.
  - Changed: `fn preview(&self, f: Field, today: NaiveDate, work_days: &[Weekday], focused:
    bool) -> Option<String>` (the examples line shows only when `focused`).
  - New: `fn values(&self) -> FormValues`; `fn initial(&self) -> &FormValues`;
    `fn set_values(&mut self, v: FormValues)` (typed text only, keeps `initial`);
    `fn with_initial(self, initial: FormValues) -> Form` (replaces the starting values: what
    "the form was opened with" when the daemon rebuilds an older form);
    `fn is_dirty(&self) -> bool` (as today); `fn alert_editable(&self) -> bool`
    (`stored_alert != Unsupported`); `fn previews(&self, today, work_days) -> Vec<(Field,
    String)>`: for each shown field among Due, Start, Repeat, Alert, the card's rule (dates
    only when their text is non-empty, Repeat and Alert always) with `focused = true`.
- Produces (`tdo::form`): `pub struct EditForm { pub form: Form, pub focus: Field, pub error:
  Option<String>, pub discarding: bool }` with `Deref<Target = Form>` and `DerefMut`;
  `EditForm::open`, `new_task`, `rebase(&self, onto: EditForm) -> (EditForm, Rebased)`,
  `preview(f, today, work_days)` (passes `focused = self.focus == f`), `key`, `paste`,
  `is_dirty`; re-exports `pub use twydo_core::form::{Field, NewTarget, Rebased, NO_CHANGES,
  ALERT_READ_ONLY};`; `FormAction`, `edit_line`, `one_line` stay.

- [ ] **Step 1: Write the failing core tests** in `twydo-core/src/form.rs` `mod tests`, on a
  synthetic `Task` loaded from `tests/data/sample-vdir` as `tdo/tests/form.rs` does:
  - `values_round_trip`: `Form::open(t)`, `set_values(v)` with a new title → `values() == v`,
    `initial()` unchanged, `save` gives `Op::Save` with `Change::Summary`.
  - `with_initial_then_rebase_reports_overlap`: open form `cur`; `mine = cur.clone()
    .with_initial(old)` where `old.title` differs from the task's title, `set_values` with a
    third title → `mine.rebase(cur).1.overlap == [Field::Title]`.
  - `previews_follow_the_card_rule`: an empty-due task's `previews(..)` has no Due entry, has
    Repeat == `repeatinput::EXAMPLES`; after `set_values` with due `"tomorrow"` the Due entry
    starts with `"→ "`.
  - `field_serializes_snake_case`: `serde_json::to_value(Field::Due) == json!("due")`.
- [ ] **Step 2: Run** `cargo test -p twydo-core form::` — expected FAIL (module missing).
- [ ] **Step 3: Move** the model into `twydo-core/src/form.rs` (no `ratatui` there), add the new
  functions, and make `tdo::form::EditForm` the wrapper described above. Update callers in
  `tdo` only where `Deref` does not cover them (constructing an `EditForm`, `rebase`).
- [ ] **Step 4: Run** `cargo test -p twydo-core form:: && cargo test -p tdo && cargo clippy
  --workspace --all-targets -- -D warnings` — expected PASS, `tests/form.rs` unchanged except
  imports.
- [ ] **Step 5: Commit** `refactor(core): move the edit form model into twydo-core`.

### Task 2: `tasks.get` returns the edit form

**Files:**
- Modify: `crates/twydo-daemon/src/protocol/tasks.rs`, `src/server/tasks.rs`,
  `schema/tdo.openrpc.json`, `crates/twydo-daemon/tests/tasks.rs`, `docs/protocol.md`

**Interfaces:**
- Consumes: Task 1's `Form`, `FormValues`, `Field`.
- Produces (`protocol::tasks`):

```rust
pub struct FieldPreview { pub field: Field, pub text: String }
/// An edit form's state, as the TUI card would show it.
pub struct FormState {
    /// The fields, in display order.
    pub fields: Vec<Field>,
    pub values: FormValues,
    pub previews: Vec<FieldPreview>,
    /// False when the task has alerts Twydo cannot write: Alert is read-only.
    pub alert_editable: bool,
}
// TasksGetResult gains:
    /// The edit form's starting state.
    pub form: FormState,
```

- Produces (`server::tasks`): `pub(crate) fn form_state(f: &Form, today: NaiveDate, work_days:
  &[Weekday]) -> FormState`; `pub(crate) fn open_form(snap: &Snapshot, idx: usize) -> Form`
  (`item_of` = the parent's `kind` field, as `tdo`'s `card_for`). `today` is the daemon clock's
  date in the snapshot tz; `work_days` from `snap.config.work_days`.

- [ ] **Step 1: Write the failing tests** in `tests/tasks.rs`:
  - `form_block_for_a_task`: a task due `20261118` with `RRULE:FREQ=WEEKLY` →
    `form.fields == ["title","list","type","due","start","repeat","alert","notes"]`,
    `form.values.due == "2026-11-18"`, `form.values.repeat` non-empty, a `repeat` preview
    starting with `→`, `alert_editable`.
  - `form_block_for_a_checklist_item`: item of `chk` → `fields == ["title","notes"]`.
  - `form_block_keeps_unwritable_rule`: `RRULE:FREQ=MONTHLY;BYSETPOS=2;BYDAY=MO,TU` (or another
    rule `repeatinput::to_text` cannot write; assert that in the test first) →
    `values.repeat == ""` and the repeat preview ends with `(kept)`.
  - `form_block_alert_read_only`: a task with two `VALARM`s → `alert_editable == false`.
- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test tasks` — expected FAIL.
- [ ] **Step 3: Implement** the types and `form_state`/`open_form`; fill `form` in `get`.
- [ ] **Step 4: Bless and run** the schema test, then `cargo test -p twydo-daemon` — PASS.
- [ ] **Step 5: Document** the `form` block in `docs/protocol.md` under `tasks.get`.
- [ ] **Step 6: Commit** `feat(daemon): return the edit form with tasks.get`.

### Task 3: `tasks.form`

**Files:**
- Modify: `crates/twydo-daemon/src/protocol/tasks.rs`, `src/server/tasks.rs`,
  `src/server/dispatch.rs`, `src/schema.rs`, `schema/tdo.openrpc.json`,
  `crates/twydo-daemon/tests/tasks.rs`, `docs/protocol.md`

**Interfaces:**
- Consumes: Task 2's `FormState`, `form_state`, `open_form`.
- Produces:

```rust
pub struct TasksFormParams {
    /// The task being edited; exactly one of `uid` and `new`.
    pub uid: Option<String>,
    pub new: Option<NewTaskTarget>,
    /// An edit's values when its form was opened (or last rebased); ignored for a new task.
    pub initial: Option<FormValues>,
    /// The typed values; none asks for the starting values.
    pub values: Option<FormValues>,
}
pub struct NewTaskTarget { pub collection: String, pub parent: Option<String>,
                           pub due: Option<NaiveDate> }
pub struct TasksFormResult {
    /// The basis to submit `op` with.
    pub epoch: String, pub revision: u64,
    /// The form these values make: on `changed_since`, the rebased one.
    pub form: FormState,
    /// The write, when every field reads and something would change.
    pub op: Option<Op>,
    /// Why `op` is none (`no changes`, `title must not be empty`, …).
    pub error: Option<String>,
    pub changed_since: bool,
    pub rebased: Option<RebasedForm>,
}
pub struct RebasedForm {
    /// The new starting values to send as `initial` from now on.
    pub initial: FormValues,
    pub overwritten: Vec<Field>,
    pub dropped: Vec<Field>,
}
```

- `server::tasks::form(shared, p) -> Result<Value, RpcError>`; dispatch `"tasks.form"`; schema
  `m("tasks.form", false, of::<TasksFormParams>, of::<TasksFormResult>)` after `tasks.get`.

Algorithm (one published snapshot, queued writes laid over, no state kept):
- Neither or both of `uid`/`new` → `invalid` (`give uid or new`). Unknown `uid` or `new.parent`
  → `invalid` `no such task`.
- New: `Form::new_task(NewTarget { collection, parent, parent_kind: parent's kind, due_today:
  new.due == Some(today) }, String::new(), tz)`, then `set_values(values)` if given;
  `changed_since = false`, `rebased = None`.
- Edit: `cur = open_form(snap, idx)`. With `initial` absent or equal to `cur.initial()`:
  `cur.set_values(values)`; `changed_since = false`. Otherwise `mine = cur.clone()
  .with_initial(initial)`, `mine.set_values(values)`, `(form, r) = mine.rebase(cur)`;
  `changed_since = true`; `rebased = { initial: form.initial(), overwritten: r.overlap,
  dropped: r.dropped }`.
- `op`/`error` from `form.save(today, work_days)`; `form` from `form_state(&form, ..)`.

- [ ] **Step 1: Write the failing tests** in `tests/tasks.rs`:
  - `form_previews_and_op_for_an_edit`: values from `tasks.get`'s `form.values` with
    `due = "tomorrow"` → a `due` preview `→ Wed 18 Nov` (clock 2026-11-17), `op ==
    {"Save": {"uid", "changes": [{"Due": …}], "to": null}}`, `changed_since == false`, basis
    equals `tasks.get`'s.
  - `form_errors`: unchanged values → `op == null`, `error == "no changes"`; empty title →
    `"title must not be empty"`; `due = "zzz"` → the due preview is the parse error and `error`
    starts with `due:`.
  - `form_type_change_changes_fields`: values with `kind = "Checklist"` on a task → `fields`
    unchanged for the task itself; a new item under that checklist → `["title","notes"]`.
  - `form_for_a_new_task`: `new: {collection: "inbox", due: today}`, no values → `form.values.due
    == "today"`, `fields` without `alert`; with title `"Synthetic new"` → `op` is
    `{"Create": {... "due": {"Date": "2026-11-17"} ...}}`.
  - `form_for_a_new_item`: `new: {collection, parent: "chk"}`, title → `Create` with
    `parent == "chk"`.
  - `changed_elsewhere_rebases`: open `tasks.get`, then rewrite the task's `.ics` on disk with a
    new SUMMARY and DESCRIPTION and wait for the publish (as other daemon tests do); call
    `tasks.form` with the old `initial` and values where only notes and title were typed →
    `changed_since`, `rebased.overwritten == ["title","notes"]`, `form.values.title` is the
    typed title, `op` is a `Save` whose changes are only the typed fields.
  - `form_unknown_uid_and_bad_params`: unknown uid and `{}` → `invalid`.
- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test tasks` — expected FAIL.
- [ ] **Step 3: Implement** as above.
- [ ] **Step 4: Bless and run** schema; `cargo test -p twydo-daemon` — PASS.
- [ ] **Step 5: Document** `tasks.form` in `docs/protocol.md` (params, result, the rebase rule,
  that it never writes and holds no state).
- [ ] **Step 6: Commit** `feat(daemon): check a task form with tasks.form`.

### Task 4: Gateway allows editing for phones

**Files:**
- Modify: `crates/twydo-gateway/src/bridge.rs`, `crates/twydo-gateway/tests/bridge.rs`,
  `docs/gateway.md`

**Interfaces:**
- Consumes: `TasksFormParams` from Task 3.
- Produces: `"tasks.form"` in `PHONE` only, forwarded re-serialized through `TasksFormParams`
  (unparsable → `invalid tasks.form params`); `phone_may_submit` true for `Create(_)`,
  `Save { .. }`, `Complete`, `Reopen`, `Delete { .. }`, false otherwise.

- [ ] **Step 1: Write the failing tests** in `tests/bridge.rs`:
  - `tasks_form_phone_only`: Phone forwards `{"uid":"u","values":null,"x":1}` without `x`;
    Display refused.
  - `phone_may_save_delete_and_create_anything`: `Save`, `Delete`, and `Create` with a parent,
    a repeat, a start and kind `Checklist` all ok for Phone, all refused for Display.
  - `phone_refused_edit_move_create_at_reorder_reparent`: each refused for Phone.
- [ ] **Step 2: Run** `cargo test -p twydo-gateway --test bridge` — FAIL.
- [ ] **Step 3: Implement**; drop `Create`'s per-field checks and the now-unused imports.
- [ ] **Step 4: Run** `cargo test -p twydo-gateway` — PASS.
- [ ] **Step 5: Update** `docs/gateway.md`'s role table to spec §7 and the "what the phone can
  do" text.
- [ ] **Step 6: Commit** `feat(gateway): let phones edit, create and delete tasks`.

### Task 5: Data source: form checks, save, delete

**Files:**
- Modify: `web/src/data.ts`, `web/src/data.test.ts`

**Interfaces:**
- Consumes: generated `TasksFormParams`, `TasksFormResult`, `FormValues`, `Op`, `BasedOn`.
- Produces on `DataSource`:
  - `checkForm(p: TasksFormParams): Promise<TasksFormResult>`: a plain `tasks.form` call.
  - `save(op: Op, basedOn: BasedOn, what: SaveInfo): Promise<WriteResult>`: `op.submit` with
    that basis (not the visible one); retry key `save:<JSON.stringify(op)>`; pending
    `{ kind: what.uid ? "save" : "add", ...what }`.
  - `remove(uid: string, title: string): Promise<WriteResult>`: `{ Delete: { uid } }`, key
    `delete:<uid>`, pending `{ kind: "delete", uid, title }`.
  - `type SaveInfo = { uid?: string; title: string; form: FormOpen }` where
    `type FormOpen = { uid?: string; new?: NewTaskTarget; initial?: FormValues; values?:
    FormValues }` (what is needed to reopen the form).
  - `Refusal` adds `{ kind: "add"; title; form: FormOpen; message }`,
    `{ kind: "save"; uid; title; form: FormOpen; message }` and
    `{ kind: "delete"; uid; title; message }`. The old `create(title, note, capture)` and its
    `create` refusal stay untouched here so `App.tsx` keeps compiling; Task 7 removes both.
  - `submit` takes an optional explicit basis; a `stale` refusal still refetches.

- [ ] **Step 1: Write the failing tests** in `data.test.ts`: `checkForm calls tasks.form`;
  `save submits the op with the given basis` (visible view at revision 5, basis 7 given →
  `based_on.revision === 7`); `remove submits Delete`; `a queued save refused later reports
  kind save with the form` (`op.finished` refused); same for `delete`; `a retried save reuses
  the request id` (same op twice after a lost connection).
- [ ] **Step 2: Run** `cd web && npm run gen && npx vitest run src/data.test.ts` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same, then `npm run check` — PASS.
- [ ] **Step 5: Commit** `feat(web): check, save and delete tasks from the data source`.

### Task 6: The form sheet

**Files:**
- Create: `web/src/formState.ts`, `web/src/formState.test.ts`,
  `web/src/components/TaskForm.tsx`, `web/src/components/ConfirmSheet.tsx`
- Modify: `web/src/styles.css`

**Interfaces:**
- Consumes: Task 5's `checkForm`, `FormOpen`; `ViewsListResult` for the list picker.
- Produces:
  - `useFormState(ds: DataSource, open: FormOpen, opts?: { debounceMs?: number })` returning
    `{ values, setField(field, text), answer?: TasksFormResult, changed: boolean, review(),
    dirty: boolean, canSave: boolean, basis?: BasedOn, initial?: FormValues }`. It calls
    `checkForm({uid, new, initial, values})` at once on open (values may be undefined for a
    new task: the answer's `form.values` become `values` and `initial`), then 250 ms after the
    last edit. Each call gets a sequence number; an answer older than the newest sent is
    dropped. `canSave = !!answer?.op && !answer.changed_since && !inFlight`. `review()` sets
    `values = answer.form.values`, `initial = answer.rebased.initial`, keeps the list of
    overwritten/dropped fields for display, and re-checks. `dirty` = values differ from
    `initial` (new task: from the starting values).
  - `ConfirmSheet({ title, message?, confirmLabel, destructive?, onConfirm, onCancel })`: an
    action sheet over a dim backdrop (`role="dialog"`).
  - `TaskForm({ ds, open, lists, heading, saveLabel, onSaved, onCancel })`:
    `<form class="sheet full" aria-label={heading}>`, header `Cancel` / heading / saveLabel.
    Renders the fields in `answer.form.fields` order: Title input; List `<select>` of the
    `list` entries of `views.list` (`{view:"list", list}` items: value = collection id, label =
    name); Type segmented control (`Task`, `Project`, `Checklist`, `role="radiogroup"`); Due,
    Start, Repeat, Alert inputs (`aria-label` = field name capitalised) each followed by its
    preview text from `answer.form.previews`; Due and Start get a calendar button over a
    hidden `<input type="date">` writing `YYYY-MM-DD` into the field; Alert gets chips `15m`,
    `1h`, `1d`, `none` and is `disabled` when `!alert_editable`; Notes a `<textarea>` growing
    with content. While `answer.error` is set and the user has typed, it shows under the header
    (not for `no changes`). On `changed_since`: a notice `Changed since you opened it` with a
    `Review` button; after review, a line naming overwritten (`Your typing replaces their
    change to: title, notes`) and dropped fields (`Can no longer be set: alert`). Save calls
    `ds.save(answer.op, basis, {uid, title, form})`; success → `onSaved(title)`; refusal → the
    message stays in the sheet; a `stale` refusal re-checks (which then shows the notice).
    Cancel with `dirty` opens `ConfirmSheet` `Discard changes?` (`Discard` destructive /
    `Keep editing`).

- [ ] **Step 1: Write the failing tests:**
  - `formState.test.ts` (fake ds, fake timers): `checks once on open and adopts starting
    values for a new task`; `debounces typing to one check after 250 ms`; `ignores an
    out-of-date check` (two checks, the first answered last → state shows the second);
    `canSave only with an op`; `changed elsewhere asks for review before saving`
    (`changed_since` → `canSave` false; `review()` → values/initial replaced, re-check sent
    with the new `initial`); `dirty after typing, clean after typing back`.
  - Component tests (a `TaskForm.test.tsx` beside it): `renders the fields in the answer's
    order`; `shows previews under the fields`; `alert input is disabled when not editable`;
    `date picker writes the date as text`; `cancel with changes asks to discard`;
    `save submits the op and closes`; `a refusal keeps the sheet open with the message`.
- [ ] **Step 2: Run** `npx vitest run src/formState.test.ts src/components/TaskForm.test.tsx`
  — FAIL.
- [ ] **Step 3: Implement**; styles in the existing sheet/card idiom, the slide-up animation
  already used by the capture sheet (none under reduced motion), inputs at least 16 px font
  (no iOS zoom).
- [ ] **Step 4: Run** `npm run check` — PASS.
- [ ] **Step 5: Commit** `feat(web): add the task form sheet`.

### Task 7: Edit, add item, new task and delete in the app

**Files:**
- Modify: `web/src/App.tsx`, `web/src/components/TaskScreen.tsx`,
  `web/src/components/App.test.tsx`, `web/src/styles.css`
- Delete: `web/src/components/CaptureSheet.tsx`

**Interfaces:**
- Consumes: Tasks 5–6.
- Produces: `TaskScreen` gains `onEdit()`, `onAddItem()` (shown only when `task.kind` is
  `Project` or `Checklist`, as `+ Add item` at the end of the Items card, which then shows even
  with no children), `onDelete()` (a red `Delete task` button at the bottom); an `Edit` button
  top-right in the header row. All disabled offline. App state `editing?: { open: FormOpen;
  heading: string; saveLabel: string }`:
  - Edit: `open = { uid, initial: result.form.values, values: result.form.values }`, heading
    `Edit`, label `Save`; saved → banner `Saved "<title>"` with Undo; the task screen refetches.
  - Add item: `open = { new: { collection: task.list.id, parent: uid } }`, heading `New item`,
    label `Add`; saved → `Added "<title>"`.
  - FAB new task: `open = { new: { collection: capture.collection, due: capture.due } }`,
    heading `New task`, label `Add`; saved → `Added "<title>" to <capture.name>` when the list
    was not changed, else `Added "<title>"`.
  - Delete: `ConfirmSheet` `Delete "<title>"?` (parent: `Delete "<title>" and its N items?`,
    N = `children.length`), `Delete` destructive; confirmed → pop the screen first (so the
    refetch does not raise "That task is gone"), `ds.remove`, banner `Deleted "<title>"` with
    Undo; a refusal shows the message.
  - Refusals: `add`/`save` → banner `Could not add "<t>": <msg>` / `Could not save "<t>":
    <msg>` with `Edit` reopening the form from `form`; `delete` → `Could not delete "<t>":
    <msg>`.

- [ ] **Step 1: Write the failing tests** in `App.test.tsx` (fake ds gains `checkForm`,
  `save`, `remove`): `edit opens the form with the task's values`; `saving shows the banner
  and refetches`; `add item opens the form for a new item of this task`; `no add item on a
  plain task`; `new task from the FAB uses the view's capture`; `delete asks, pops and shows
  the banner`; `delete pops without the gone banner` (fire `onTaskGone` after the pop: no gone
  banner); `delete of a parent names its items`; `a refused queued save reopens the form`;
  `offline disables edit, add item and delete`. Replace the CaptureSheet-based tests.
- [ ] **Step 2: Run** `npx vitest run src/components/App.test.tsx` — FAIL.
- [ ] **Step 3: Implement**; delete `CaptureSheet.tsx`, `DataSource.create` and the old
  `create` refusal kind (and their tests in `data.test.ts`).
- [ ] **Step 4: Run** `npm run check` — PASS.
- [ ] **Step 5: Commit** `feat(web): edit, add items to and delete tasks on the phone`.

### Task 8: End-to-end checks and documentation

**Files:**
- Modify: `web/e2e/app.spec.ts`, `web/e2e/setup.ts` (only if a new seed is needed),
  `docs/gateway.md`, `docs/protocol.md` (check), `docs/README.md` (only if it lists phone
  features)

**Interfaces:**
- Consumes: the seed from Plan A (`chk1` "Synthetic packing" with `chk1a`, `chk1b`; inbox and
  work tasks).

- [ ] **Step 1: Write the e2e tests** (phone size, light and dark projects, isolated daemon):
  - `edit a due date and a repeat`: open a synthetic inbox task → Edit → Due `tomorrow`
    (preview shows `→`), Repeat `weekly` (preview shows `→ every week`) → Save → banner
    `Saved "<t>"`; the task screen shows a Due and a Repeat line.
  - `add an item`: open "Synthetic packing" → `+ Add item` → title `Synthetic towel` → Add →
    the item appears in Items.
  - `delete with undo`: open a synthetic task → Delete task → confirm → back on the list,
    banner `Deleted "<t>"`, row gone; Undo → row back.
  - `create a task with a list and a due date`: FAB → title, List `work`'s name, Due `today` →
    Add → it shows in Today.
  Keep counts other tests assert stable (use tasks no other test asserts on, or adjust).
- [ ] **Step 2: Run** `mise run web-e2e` — PASS.
- [ ] **Step 3: Update** `docs/gateway.md` "what the phone can do": edit, new task with fields,
  add item, delete; what stays in the TUI (spec §2 non-goals). Run
  `python3 tools/check_links.py`.
- [ ] **Step 4: Final verification:** `mise run test && mise run lint && mise run web-check &&
  mise run web-e2e` — all PASS.
- [ ] **Step 5: Commit** `test(web): cover phone editing end to end`.
