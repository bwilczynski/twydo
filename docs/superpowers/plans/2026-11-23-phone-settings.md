# Phone Settings Screen Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Settings screen in the phone app, opened by a gear on the Views screen, holding the
Reminders switch, sync status, the CalDAV server host and versions.

**Architecture:** A new daemon method `settings.get` (no params) returns the daemon version,
protocol generation, server host and the same sync/reminders summaries `views.get` sends. The
gateway allows it for the phone role only. The web app gains a `"settings"` visible target that
`DataSource` fetches like `"views"`, and a `SettingsScreen` pushed inside the Views tab.

**Tech Stack:** Rust (daemon, gateway; schemars/serde), TypeScript + Preact + signals (web),
vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-23-phone-settings-design.md`

## Global Constraints

- The phone never receives credentials, usernames or URL paths: `server` is host, plus `:port`
  only when it is not the scheme's default (443 for https, 80 for http).
- No new protocol generation (`PROTOCOL` stays 4); `settings.get` is additive.
- The display role may not call `settings.get`.
- Warnings stay on the task views; Settings repeats them.
- Commits pass commitlint (`type(scope): subject`, lowercase subject, ≤100 chars, no agent
  attribution).
- Docs change with behaviour; `python3 tools/check_links.py` passes.

## Review Focus

- A sync URL with userinfo (`https://user:pw@host/dav/`) shows only `host` — test in Task 1.
- A URL with a non-default port (`https://host:8443/`) keeps the port; `https://host:443` drops
  it; an IPv6 literal `http://[::1]:5232/` keeps `[::1]:5232` — tests in Task 1.
- Sync not configured: `server` and `sync` absent; Settings shows no Server row — Tasks 1 and 3.
- An older daemon answers "method not found": Settings still shows the Reminders switch and
  `Daemon info unavailable` — test in Task 3.
- Reload with Settings open returns to Settings; re-tapping the Views tab pops to the list —
  tests in Task 3.

---

### Task 1: Daemon `settings.get`

**Files:**
- Modify: `crates/twydo-daemon/src/protocol/views.rs` (add `SettingsGetResult`)
- Create: `crates/twydo-daemon/src/server/settings.rs` (handler and `server_host`)
- Modify: `crates/twydo-daemon/src/server/mod.rs` (`pub mod settings;`)
- Modify: `crates/twydo-daemon/src/server/views.rs` (extract the summaries helper)
- Modify: `crates/twydo-daemon/src/schema.rs` (method row after `views.get`)
- Modify: `crates/twydo-daemon/src/server/dispatch.rs` (arm after `views.get`)
- Regenerate: `schema/tdo.openrpc.json`
- Modify: `docs/protocol.md` (a `settings.get` bullet beside `views.list`)
- Test: `crates/twydo-daemon/tests/views.rs`, unit tests in `server/settings.rs`

**Interfaces:**
- Produces (wire): `settings.get`, params `{}`, result
  `SettingsGetResult { version: String, protocol: u32, server: Option<String>,
  sync: Option<SyncSummary>, reminders: Option<RemindersSummary> }`; `server`, `sync`,
  `reminders` use `skip_serializing_if = "Option::is_none"` and `default`.
- Produces (Rust): `pub(crate) fn status_summaries(shared: &Arc<Shared>, snap: &Snapshot,
  now: DateTime<FixedOffset>) -> (Option<SyncSummary>, Option<RemindersSummary>)` in
  `server/views.rs`, used by both `views::get` and `settings::get`.
- Produces (Rust): `pub fn server_host(url: &str) -> Option<String>` in `server/settings.rs`.

- [ ] **Step 1: Write failing unit tests for `server_host`**

```rust
assert_eq!(server_host("https://dav.example.net/dav/"), Some("dav.example.net".into()));
assert_eq!(server_host("https://user:pw@dav.example.net/x"), Some("dav.example.net".into()));
assert_eq!(server_host("https://dav.example.net:443/"), Some("dav.example.net".into()));
assert_eq!(server_host("http://dav.example.net:80"), Some("dav.example.net".into()));
assert_eq!(server_host("https://dav.example.net:8443/"), Some("dav.example.net:8443".into()));
assert_eq!(server_host("http://[::1]:5232/"), Some("[::1]:5232".into()));
assert_eq!(server_host(""), None);
assert_eq!(server_host("not a url"), None);
```

Hand-rolled parse (no new crate): require `http://` or `https://` (case-insensitive), cut at the
first `/`, `?` or `#`, drop everything up to the last `@`, drop a default port, `None` if empty
or containing whitespace.

- [ ] **Step 2: Write failing integration tests in `tests/views.rs`**

- `settings_without_sync`: `start(&fx, |_| {})`; call `settings.get` with `{}`;
  `version` equals `env!("CARGO_PKG_VERSION")` of the daemon as `daemon.info` reports it (compare
  to a `daemon.info` call), `protocol == 4`, `server == None`, `sync == None`.
- `settings_with_sync`: config from `fx.with_sync("exit 0")` with `sync.url` set to
  `"https://user@dav.example.net/dav/"`; `server == Some("dav.example.net")`, `sync.is_some()`,
  and `sync` equals `get(&td, "inbox").sync`.
- `settings_carry_the_notifier_error`: same setup as `reminders_carry_the_notifier_error`; poll
  until `settings.get`'s `reminders` is `Some`, non-empty `error`.

- [ ] **Step 3: Run and see them fail**

Run: `cargo test -p twydo-daemon --test views settings_ && cargo test -p twydo-daemon --lib settings`
Expected: FAIL (method not found / function missing).

- [ ] **Step 4: Implement** the type, `status_summaries` (moved out of `views::get`, which now
  calls it), `settings::get(shared) -> Result<Value, RpcError>` (version from `shared.version`,
  `PROTOCOL`, `shared.cfg.sync.as_ref().and_then(|s| server_host(&s.url))`), the schema row
  `m("settings.get", false, of::<Empty>, of::<SettingsGetResult>)` and the dispatch arm.

- [ ] **Step 5: Bless the schema and run the daemon tests**

Run: `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema && cargo test -p twydo-daemon`
Expected: PASS, including `no_breaking_change_against_the_released_schema`.

- [ ] **Step 6: Document** `settings.get` in `docs/protocol.md` next to `views.list` (fields,
  host-only server, absent when sync is off).

- [ ] **Step 7: Commit**

```bash
git add crates/twydo-daemon schema/tdo.openrpc.json docs/protocol.md
git commit -m "feat(daemon): add settings.get for the phone's settings screen"
```

### Task 2: Gateway allows `settings.get` for phones

**Files:**
- Modify: `crates/twydo-gateway/src/bridge.rs` (`PHONE` list; the no-params arm with `views.list`)
- Modify: `docs/gateway.md` (role table at line ~149 and the sentence at ~141)
- Test: `crates/twydo-gateway/tests/bridge.rs`

**Interfaces:**
- Consumes: `settings.get` from Task 1.
- Produces: phones may call `settings.get`; params are replaced by `{}` like `views.list`.

- [ ] **Step 1: Write failing tests**, modelled on `views_list_allowed_for_both_roles`:
  `settings_get_allowed_for_phone` (`run(Role::Phone, "settings.get", Some(json!({"x":1})))` →
  `("settings.get", json!({}))`; same with `None`) and `settings_get_refused_for_display`
  (`run(Role::Display, …)` is `Err` with message `not allowed for this device`).

- [ ] **Step 2: Run** `cargo test -p twydo-gateway --test bridge settings_get` — FAIL.

- [ ] **Step 3: Implement**: add `"settings.get"` to `PHONE` and to the
  `"views.list" | "events.subscribe" | …` arm.

- [ ] **Step 4: Run** `cargo test -p twydo-gateway` — PASS.

- [ ] **Step 5: Update `docs/gateway.md`** role table (phone row gains `settings.get`).

- [ ] **Step 6: Commit** `feat(gateway): let phones read settings.get`

### Task 3: Web Settings screen

**Files:**
- Modify: `web/src/data.ts` (`Visible`, `sameTarget`, `fetchVisible`, `DataSource.settings`)
- Create: `web/src/components/SettingsScreen.tsx`
- Modify: `web/src/components/ViewsScreen.tsx` (remove `RemindersRow` and the `push` prop; the
  row moves to `SettingsScreen` unchanged)
- Modify: `web/src/components/Icons.tsx` (`GearIcon`, Lucide `settings` path)
- Modify: `web/src/App.tsx` (gear button, open state, back button, title)
- Modify: `web/src/styles.css` (gear button in `.large-title`; Settings rows reuse `.group`/`.card`)
- Modify: `docs/gateway.md` (phone app section: the gear and what Settings shows)
- Test: `web/src/data.test.ts`, `web/src/components/App.test.tsx`, `web/e2e/app.spec.ts`

**Interfaces:**
- Consumes: `SettingsGetResult` from the generated `web/src/gen/protocol.d.ts` (`npm run gen`).
- Produces: `Visible = ViewRef | "views" | "settings" | TaskRef`;
  `DataSource.settings: ReadonlySignal<SettingsGetResult | null | undefined>` — `undefined`
  before the first answer, `null` when the daemon refused the method (older daemon).
- Produces: `SettingsScreen({ settings, push }: { settings: SettingsGetResult | null | undefined;
  push?: Push })`.
- App: the Views tab's opened target widens to `ViewRef | "settings"`; `twydo.open` stores
  `"settings"` as the JSON string `"settings"`, and `loadOpen` accepts it.

- [ ] **Step 1: Write failing `data.test.ts` tests**, in the style of the existing `views.list`
  ones: `refresh("settings")` calls `settings.get` with `{}` and fills `ds.settings`; a refusal
  with code `-32601` (method not found) sets `ds.settings` to `null` and does not reject;
  `refresh("settings")` then `refresh("views")` makes `fresh` false until the list arrives.

- [ ] **Step 2: Write failing `App.test.tsx` tests** (the `fake()` ds gains
  `settings: signal<SettingsGetResult | null | undefined>(…)`):
  - `views_screen_has_a_gear_not_the_reminders_switch`: Views tab shows a button named
    `Settings`; no `switch` named `Reminders`.
  - `gear_opens_settings`: click `Settings` → heading `Settings`, a `‹ Views` back button,
    the Reminders switch, text `dav.example.net`, the version string, the sync headline.
  - Move the three existing `reminders_row_*` tests to open Settings first; their assertions
    stay.
  - `settings_without_sync_has_no_server_row`: `server` and `sync` absent → no `Server` text.
  - `settings_shows_the_reminders_failure`: `reminders: { error: "The push service rejected us" }`
    → that text appears.
  - `settings_on_an_older_daemon`: `settings` is `null` → Reminders switch and
    `Daemon info unavailable`, no About section.
  - `settings_survive_a_reload`: open Settings, unmount, render again → `Settings` heading.
  - `views_tab_retap_leaves_settings`: open Settings, click the Views tab → the views list.
  - `settings_shows_no_fab`: no `New task` button on Settings.

- [ ] **Step 3: Run** `cd web && npm run check` — the new tests FAIL.

- [ ] **Step 4: Implement `data.ts`**: `"settings"` in `Visible`; `sameTarget` treats it like
  `"views"` (string equality); `fetchVisible` calls `settings.get` into the `settings` signal,
  mapping an RPC error with `code === -32601` to `null` and rethrowing others; the
  visible-to-signal lookup at `data.ts:308` returns `undefined` for `"settings"` as for `"views"`.

- [ ] **Step 5: Implement `SettingsScreen`** with three `.group` sections:
  1. the `RemindersRow` moved from `ViewsScreen` (unchanged copy and behaviour);
  2. heading `Sync`: rows for the headline (with `detail` beneath when present), `Last synced`
     with `last_success` as phone-local `HH:MM` on today's date or `D MMM HH:MM` otherwise
     (`Never` when null), `Server` when present, and `Reminders failing` with the error when
     `reminders` is present; the section is omitted when `sync`, `server` and `reminders` are
     all absent, replaced by a row `Sync is not configured`;
  3. heading `About`: `Twydo` with `version`, `Protocol` with `protocol`.
  `null` replaces sections 2–3 with `<p class="reminders-hint">Daemon info unavailable</p>`;
  `undefined` shows nothing below the switch (the App's spinner covers loading).

- [ ] **Step 6: Implement `App.tsx`**: on the Views list, the `.large-title` gets a trailing
  `<button class="icon-button" aria-label="Settings"><GearIcon/></button>` that calls
  `openView("settings")`; when `open === "settings"`: `ref` is `undefined`, visible is
  `"settings"`, title `Settings`, back button `Views`, no FAB, no count, render
  `<SettingsScreen settings={ds.settings.value} push={push}/>`; spinner while
  `ds.settings.value === undefined` and online. `onGone` compares only when `open` is a ViewRef.
  Remove the `push` prop from `<ViewsScreen>`.

- [ ] **Step 7: Run** `cd web && npm run check` — PASS.

- [ ] **Step 8: Extend `e2e/app.spec.ts`** with one test: from the Views tab, tap `Settings`,
  see the `Settings` heading and the `About` section's version, tap `Views` back.
  Run: `cd web && npm run e2e` — PASS (use the chromium fallback if WebKit is unavailable here).

- [ ] **Step 9: Update `docs/gateway.md`** phone-app section (gear, Settings contents, Reminders
  switch now lives there); run `python3 tools/check_links.py`.

- [ ] **Step 10: Commit** `feat(web): move reminders into a settings screen`
