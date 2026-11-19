# Phone All Views Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A third phone tab, **Views**, lists the TUI sidebar's views; tapping one opens it
inside the tab with a back button, served by a new `views.list` and `views.get` for every view.

**Architecture:** The TUI's sidebar builder moves into `twydo_core::views` so the TUI and the
daemon share it. The daemon gains `views.list` and serves `views.get` for `all`, `scheduled`,
`done`, `next_3_days` and `list` + id; the gateway allowlists `views.list` for both roles. The
web app's data layer caches views by a key derived from the view reference and adds the views
list; the UI adds the tab, the list screen and push/back navigation.

**Tech Stack:** Rust (serde, schemars, the daemon's JSON-RPC), TypeScript + Preact + signals,
Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-19-phone-all-views-design.md`

## Global Constraints

- Tests isolate `XDG_CONFIG_HOME` and `TWYDO_SOCKET`; stop only an isolated test daemon.
- No personal data: fixtures and docs use synthetic content only.
- Protocol changes are additions: `schema/tdo.openrpc.json` regenerated with
  `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema`, and the released-schema test passes.
- Enums a client receives are open (`x-open`, an `Unknown` `#[serde(other)]` variant).
- `views.get` with `inbox` and `today` keeps its exact result.
- The TUI's behaviour does not change; it does not call `views.list`.
- Commits pass commitlint: `type(scope): lowercase subject`, no trailing full stop, body lines
  ≤ 100 chars. No agent attribution lines.
- Cargo target never in `/tmp` (use the default `target/` or `~/.cache`).
- Web checks: `cd web && npm run check` (regenerates `src/gen/protocol.d.ts` from the schema).

## Review Focus

1. **The Inbox list is hidden or archived** → the views list has no Inbox entry and starts with
   Today, exactly like the TUI; the Inbox tab still works. (Task 3 test
   `views_list_omits_hidden_inbox`.)
2. **A list is deleted while open on the phone** → the next refetch is refused `invalid`; the
   app returns to the views list with "That list is gone", not a spinner forever. (Task 5 test
   `gone_list_reports_once`; Task 6 test `deleted_list_pops_back_with_banner`.)
3. **The remembered open view no longer exists after a reload** → same pop-back as 2, no
   crash on start. (Task 6 test `remembered_missing_list_pops_back`.)
4. **Completing in an opened view while another view is cached** → `based_on` is the opened
   view's revision, and the views-list counts refresh afterwards on `data.changed`. (Task 5 test
   `complete_uses_opened_view_basis`.)
5. **`list` param misuse** (`{"view":"list"}` without id, `{"view":"today","list":"x"}`) →
   refused `invalid`, never served as some other view. (Task 2 test
   `list_param_must_match_view`.)

---

### Task 1: Sidebar in core

**Files:**
- Modify: `crates/twydo-core/src/views.rs` (add `SidebarEntry`, `sidebar`, `view_name`,
  `list_name`)
- Modify: `crates/tdo/src/app.rs:62-67,306-361` (`SidebarEntry` becomes
  `pub use twydo_core::views::SidebarEntry`; `sidebar_entries` delegates)
- Modify: `crates/tdo/src/ui/mod.rs:188-206` (`view_name`, `list_name` delegate or are
  re-exported)
- Test: `crates/twydo-core/tests/sidebar.rs` (create)

**Interfaces:**
- Produces, in `twydo_core::views`:
  - `pub enum SidebarEntry { Heading(String), Item(View) }` (`Debug, Clone, PartialEq, Eq`)
  - `pub fn sidebar(snap: &Snapshot) -> Vec<SidebarEntry>`: the body of today's
    `App::sidebar_entries`, unchanged in behaviour.
  - `pub fn view_name(snap: &Snapshot, view: &View) -> String`: `Today`, `All`, `Scheduled`,
    `Done`, `Next 3 Days`, or the list's display name (id when none).
  - `pub fn list_name(snap: &Snapshot, id: &str) -> String`.
- `tdo::app::SidebarEntry` stays importable (re-export), so `crates/tdo/tests/app_keys.rs`
  compiles unchanged.

- [ ] **Step 1: Write the failing core tests** in `crates/twydo-core/tests/sidebar.rs`, on a
  synthetic snapshot with lists `inbox` (display name `Inbox`), `work`, `home`, and one
  config group `G` holding `home`:
  - `sidebar_order`: `sidebar(&s)` equals `[Item(List("inbox")), Item(Today), Item(All),
    Item(Scheduled), Item(Done), Heading("G"), Item(List("home")), Heading("Lists"),
    Item(List("work")), Heading("Smart lists"), Item(Next3Days)]`.
  - `hidden_inbox_left_out`: with `inbox` hidden, the first entry is `Item(Today)` and no entry
    is `Item(List("inbox"))`.
  - `view_names`: `view_name` of each built-in view gives the strings above; of
    `List("work")` with no display name gives `"work"`.
  Reuse whatever snapshot builder the existing core view tests use.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test sidebar` → fails to compile
  (`sidebar` not found).
- [ ] **Step 3: Move the code.** Cut `App::sidebar_entries`'s body into
  `twydo_core::views::sidebar`; `App::sidebar_entries` becomes `sidebar(&self.snapshot)`.
  Move `view_name`/`list_name` likewise; the `tdo` ui keeps thin `pub(crate)` wrappers or
  imports, whichever touches fewer call sites.
- [ ] **Step 4: Run** `cargo test -p twydo-core --test sidebar && cargo test -p tdo` → all
  pass, including `inbox_heads_the_focus_lists`.
- [ ] **Step 5: Commit** `refactor(core): move the sidebar entries into twydo-core`.

### Task 2: `views.get` for every view

**Files:**
- Modify: `crates/twydo-daemon/src/protocol/views.rs` (`ViewName`, `ViewsGetParams`)
- Modify: `crates/twydo-daemon/src/server/views.rs` (`get`)
- Test: `crates/twydo-daemon/tests/views.rs`
- Modify: `schema/tdo.openrpc.json` (regenerated)

**Interfaces:**
- Consumes: `twydo_core::views::{view_name, list_name}` (Task 1).
- Produces:
  - `ViewName` gains `All`, `Scheduled`, `Done`, `#[serde(rename = "next_3_days")] Next3Days`,
    `List`, and becomes open: `#[schemars(transform = super::open_enum)]` plus
    `#[serde(other)] #[schemars(skip)] Unknown` (clients now receive it in `views.list`).
  - `ViewsGetParams { pub view: ViewName, #[serde(default, skip_serializing_if =
    "Option::is_none")] pub list: Option<String> }`. In TypeScript this type is the view
    reference (`ViewRef`).
  - `pub(crate) fn resolve(snap: &Snapshot, p: &ViewsGetParams) -> Result<View, RpcError>` in
    `server/views.rs`: the params check and mapping to a core `View`, used by `get`.

- [ ] **Step 1: Write the failing tests** in `crates/twydo-daemon/tests/views.rs`. Replace
  `unknown_view_is_invalid_params` (it uses `"all"`, now valid) with one using `"someday"`.
  Add a helper `get_ref(td, json!({...}))`, then:
  - `every_view_matches_build_view`: for `("all", View::All)`, `("scheduled", Scheduled)`,
    `("done", Done)`, `("next_3_days", Next3Days)` and `({"view":"list","list":"work"},
    List("work"))`, the uids, section keys and labels equal `build_view` at the fixture clock,
    as `today_matches_build_view` does; `title` equals `view_name`; `open_count` equals
    `counts(..).total`.
  - `capture_for_new_views`: `list`/`work` → collection `work`, due `None`; `all`,
    `scheduled`, `done`, `next_3_days` → `default_list` (`inbox` in the fixture), due `None`.
  - `list_param_must_match_view`: `{"view":"list"}`, `{"view":"today","list":"work"}` and
    `{"view":"list","list":"nope"}` are each refused with `ErrorKind::Invalid`.
  - `hidden_list_is_served_by_id`: with `work` hidden in config, `{"view":"list","list":
    "work"}` still returns its rows.
  - Existing `inbox_*`, `today_*`, `capture_targets` tests stay unchanged and must pass.
- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test views` → new tests fail.
- [ ] **Step 3: Implement** `resolve` and the new arms in `get`. `list` without id, id with a
  non-list view, an unknown collection id, and `Unknown` → `ErrorKind::Invalid` (message
  `"no such list"` for the unknown id). Capture per the spec §6.2: `List(id)` → that list;
  `Today` unchanged; the other four → `default_list` with `due: None`, `None` when it is empty.
  The `inbox` arm is untouched.
- [ ] **Step 4: Run** `cargo test -p twydo-daemon --test views` → pass. Then
  `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema` and
  `cargo test -p twydo-daemon --test schema` → pass, including the released-schema
  compatibility test. If that test rejects making `ViewName` open, stop and report rather than
  weakening the test.
- [ ] **Step 5: Commit** `feat(daemon): serve every sidebar view from views.get`.

### Task 3: `views.list`

**Files:**
- Modify: `crates/twydo-daemon/src/protocol/views.rs` (result types)
- Modify: `crates/twydo-daemon/src/server/views.rs` (`list`)
- Modify: `crates/twydo-daemon/src/server/dispatch.rs:165`, `crates/twydo-daemon/src/schema.rs:82`
- Test: `crates/twydo-daemon/tests/views.rs`
- Modify: `schema/tdo.openrpc.json` (regenerated)

**Interfaces:**
- Consumes: `twydo_core::views::{sidebar, SidebarEntry, view_name, counts}` (Task 1);
  `ViewsGetParams`, `ViewName` (Task 2).
- Produces:
  - `ViewsListResult { epoch: String, revision: u64, now: DateTime<FixedOffset>, today:
    NaiveDate, entries: Vec<ViewsEntry> }`
  - `#[serde(untagged)] enum ViewsEntry { Heading { heading: String }, View(ViewsListItem) }`
  - `ViewsListItem { view: ViewsGetParams, name: String, color: Option<String>, open_count:
    usize, due_count: usize }` (`color` is the list's `#rrggbb`, as `RowList.color` derives it;
    `None` for built-in views).
  - Method `views.list`, params `{}` (`Empty`), any session; `pub fn list(shared:
    &Arc<Shared>) -> Result<Value, RpcError>`.
  - Mapping: `Item(List(id))` where `Some(id) == inbox_list(snap)` → `{view:"inbox"}`; other
    `List(id)` → `{view:"list", list:id}`; built-ins → their `ViewName`.

- [ ] **Step 1: Write the failing tests:**
  - `views_list_matches_sidebar`: with config group `G` holding `work`, entries map 1:1 onto
    `sidebar(&fx.snapshot())`; headings equal; each item's `name` equals `view_name`, its
    counts equal `counts(..)` `total`/`due_or_overdue` at the fixture clock; the first item's
    `view` is `{"view":"inbox"}`.
  - `views_list_omits_hidden_inbox`: with `inbox` hidden, the first item is `today` and no
    item has `view: inbox`.
  - `views_list_item_round_trips`: every item's `view`, sent back to `views.get`, succeeds and
    its `title` equals the item's `name`.
  - `views_list_colors`: a list with a colour in the fixture gives `#rrggbb`; `today` has
    `color: None`.
- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test views` → fail (unknown method).
- [ ] **Step 3: Implement** `list` on the published snapshot at `shared.clock` (as `get`
  does), register it in `dispatch.rs` and in `schema.rs` (`of::<Empty>`, `of::<ViewsListResult>`).
- [ ] **Step 4: Run** the views tests, re-bless and re-run the schema tests → pass.
- [ ] **Step 5: Commit** `feat(daemon): add views.list for thin clients`.

### Task 4: Gateway allowlist

**Files:**
- Modify: `crates/twydo-gateway/src/bridge.rs:14-21,95-103`
- Test: `crates/twydo-gateway/tests/bridge.rs`

**Interfaces:**
- Consumes: method name `views.list` (Task 3); `ViewsGetParams` with `list` (Task 2).

- [ ] **Step 1: Write the failing tests:** `views_list_allowed_for_both_roles`: `inbound`
  with `views.list` and any params returns `("views.list", json!({}))` for `Role::Phone` and
  `Role::Display`. `views_get_list_param_forwarded`: `{"view":"list","list":"work"}` is
  forwarded unchanged; `{"view":"list","list":"work","extra":1}` is forwarded without `extra`.
- [ ] **Step 2: Run** `cargo test -p twydo-gateway --test bridge` → fail.
- [ ] **Step 3: Implement:** add `"views.list"` to `PHONE` and `DISPLAY`; forward it as
  `json!({})` beside `events.subscribe`.
- [ ] **Step 4: Run** `cargo test -p twydo-gateway` → pass.
- [ ] **Step 5: Commit** `feat(gateway): allow views.list for phones and displays`.

### Task 5: Web data layer

**Files:**
- Modify: `web/src/data.ts`
- Test: `web/src/data.test.ts`

**Interfaces:**
- Consumes: generated `ViewsGetParams`, `ViewsListResult`, `ViewsEntry` from
  `./gen/protocol` (Tasks 2–3).
- Produces (replacing `ViewName = "inbox" | "today"`):
  - `export type ViewRef = ViewsGetParams;`
  - `export type Visible = ViewRef | "views";`
  - `export function viewKey(ref: ViewRef): string` → `"inbox"`, `"today"`, `"all"`, …,
    `"list:<id>"`.
  - `DataSource.view(ref: ViewRef): ReadonlySignal<ViewsGetResult | undefined>` (cache keyed
    by `viewKey`, entries created on first use; in memory only, as now).
  - `DataSource.viewsList: ReadonlySignal<ViewsListResult | undefined>`.
  - `DataSource.refresh(v: Visible): Promise<void>`: `"views"` fetches `views.list`, a ref
    fetches `views.get` with exactly `{view, list?}`. Coalescing, `fresh`, the timer, the
    visibility refetch and the `data.changed`/`status.changed`/`missed` refetch all act on the
    current `Visible`.
  - `DataSource.onGone(cb: (ref: ViewRef) => void): () => void`: called once when a `views.get`
    for a `list` ref is refused with `data.kind === "invalid"`; the cached entry is dropped.
  - `submit`'s `based_on` comes from the visible ref's cached view (never from the views list).

- [ ] **Step 1: Write the failing tests** in `data.test.ts`, following the existing
  FakeWebSocket patterns:
  - `view_key_forms`: `viewKey({view:"list",list:"w"})` is `"list:w"`; `viewKey({view:"done"})`
    is `"done"`.
  - `refresh_views_fetches_views_list`: `refresh("views")` sends `views.list` with `{}` and
    sets `viewsList`.
  - `refresh_list_sends_list_param`: `refresh({view:"list",list:"w"})` sends exactly
    `{view:"list",list:"w"}`; `view({view:"list",list:"w"})` holds the result and
    `view({view:"inbox"})` is untouched.
  - `data_changed_refetches_visible`: with `"views"` visible, a `data.changed` event sends
    `views.list`, not `views.get`.
  - `gone_list_reports_once`: `views.get` refused with kind `invalid` for a list ref calls the
    `onGone` callback once with that ref and clears its cached signal.
  - `complete_uses_opened_view_basis`: with `inbox` cached at revision 3 and the opened list at
    revision 7, `complete` sends `based_on.revision === 7`.
- [ ] **Step 2: Run** `cd web && npm run check` → new tests fail.
- [ ] **Step 3: Implement** the interface above in `data.ts`; update `App.tsx` and `TabBar.tsx`
  only as far as needed to compile (`{view:"inbox"}` / `{view:"today"}` refs).
- [ ] **Step 4: Run** `cd web && npm run check` → pass.
- [ ] **Step 5: Commit** `feat(web): fetch any view and the views list`.

### Task 6: Views tab and navigation

**Files:**
- Create: `web/src/components/ViewsScreen.tsx`
- Modify: `web/src/components/TabBar.tsx`, `web/src/components/Icons.tsx`, `web/src/App.tsx`,
  `web/src/styles.css`
- Test: `web/src/components/App.test.tsx`

**Interfaces:**
- Consumes: Task 5's `DataSource`, `ViewRef`, `viewKey`, `onGone`.
- Produces:
  - `type Tab = "inbox" | "today" | "views"`; local storage `twydo.tab` accepts `"views"`;
    `twydo.open` holds the JSON `ViewRef` open in the Views tab (absent on the list screen).
  - `ViewsScreen({ list, onOpen }: { list: ViewsListResult; onOpen: (ref: ViewRef) => void })`.
  - `ListIcon` (tab icon and generic empty-state icon) in `Icons.tsx`.

- [ ] **Step 1: Write the failing tests** in `App.test.tsx` (fake data source as the existing
  tests use):
  - `views_tab_lists_entries`: the third tab, labelled "Views", shows headings and one button
    per item with its name; an item with `due_count` 2 and `open_count` 5 shows both numbers,
    one with `open_count` 0 shows none.
  - `tapping_entry_opens_view_with_back`: tapping "All" shows heading "All" and a "Views" back
    button; tapping back shows the list again.
  - `inbox_entry_opens_inside_views_tab`: tapping the Inbox entry keeps the Views tab
    selected and shows the Inbox rows.
  - `retapping_views_tab_pops`: with a view open, tapping the Views tab returns to the list.
  - `open_view_survives_tab_switch_and_reload`: open "All", switch to Today and back → "All";
    re-mount `App` → "All" again.
  - `deleted_list_pops_back_with_banner`: an `onGone` for the open list shows the list screen
    and the banner text `That list is gone`.
  - `remembered_missing_list_pops_back`: `twydo.open` names a list whose fetch is gone →
    same as above, no crash.
  - `no_add_button_on_views_list`: the "New task" button is absent on the list screen and
    present in an opened view with `capture`.
- [ ] **Step 2: Run** `cd web && npm run check` → fail.
- [ ] **Step 3: Implement.** `App` derives the visible target: Inbox/Today tabs → their ref;
  Views tab → `twydo.open` ref or `"views"`; calls `ds.refresh(visible)` when it changes.
  `ViewsScreen` follows the spec §8.2: large title "Views", small-caps headings, cells with a
  colour bar (lists) or an icon (Inbox/Today reuse tab icons, others `ListIcon`), counts as
  `due` in the overdue colour then `total`, a chevron. The opened view reuses the existing
  screen (title, sync warning, `TaskList`, `+` with `capture`, banner, sheets) with a back
  button labelled "Views" at top left; `EmptyList` uses `ListIcon` for views other than
  Inbox/Today.
- [ ] **Step 4: Run** `cd web && npm run check` → pass.
- [ ] **Step 5: Commit** `feat(web): add a views tab that opens any view`.

### Task 7: End-to-end, docs, Done size

**Files:**
- Modify: `web/e2e/app.spec.ts`
- Modify: `docs/protocol.md`, `docs/gateway.md`, `docs/architecture.md` (only if it names where
  the sidebar is built)

- [ ] **Step 1: Write the e2e test** `views tab opens a list and completes in it`: tap the
  Views tab, see "Inbox", "Today", "All" and a "work" entry; tap "work"; complete
  "Synthetic due today"; the banner says `Completed "Synthetic due today"`; tap back; the
  "work" entry's count drops by one.
- [ ] **Step 2: Run** `cargo build -p tdo && cd web && npm run build && npm run e2e` → pass
  (the e2e harness uses `target/debug/tdo`).
- [ ] **Step 3: Docs.** `docs/protocol.md`: `views.list` (params, entry shapes, Inbox mapping)
  and `views.get`'s new values and `list` field with its refusals. `docs/gateway.md`: the Views
  tab and its navigation, and `views.list` in both roles' allowlists. Measure the `done` view's
  `views.get` JSON size on the e2e fixture after completing all its tasks and state in
  `docs/gateway.md` that Done is not paged, with that measured size per row as guidance.
  Run `python3 tools/check_links.py` → no output.
- [ ] **Step 4: Full check:** `cargo test --workspace`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cd web && npm run check` → all pass.
- [ ] **Step 5: Commit** `test(web): cover the views tab end to end` and
  `docs: describe views.list and the phone views tab` (two commits).
