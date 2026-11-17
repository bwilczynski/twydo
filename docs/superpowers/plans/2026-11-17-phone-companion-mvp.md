# Phone companion MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A home-screen PWA (Inbox and Today, capture, complete, undo) served by a new
`tdo gateway` command that bridges an allowlisted subset of the daemon's JSON-RPC to a
WebSocket, shipped in a container image.

**Architecture:** The daemon gains one read method, `views.get`, which runs the TUI's own view
code and returns finished rows. A new crate `twydo-gateway` (linked into `tdo` as
`tdo gateway`) is a client of the daemon socket: it serves the embedded PWA, pairs devices,
authenticates every request, and forwards allowlisted JSON-RPC messages per device role, one
daemon connection per WebSocket. The PWA is TypeScript + Preact + Vite with protocol types
generated from `schema/tdo.openrpc.json`.

**Tech Stack:** Rust 1.99 (std threads, `httparse`, `tungstenite` with `handshake`, `sha2`,
`getrandom`), TypeScript, Preact, `@preact/signals`, Vite, Vitest, Playwright (WebKit), Docker
(buildx, `tini`), GitHub Actions, cargo-dist 0.33.0.

**Spec:** [docs/superpowers/specs/2026-11-17-phone-companion-mvp-design.md](../specs/2026-11-17-phone-companion-mvp-design.md).
Read it first; section numbers below (§n) refer to it.

## Global Constraints

- The daemon never opens a TCP port. Only `tdo gateway serve` listens on the network.
- Clients (the gateway included) never read `config.toml`, the vdir or Twydo's state directory.
- Tests isolate `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `TWYDO_SOCKET`; no test reaches the
  user's default socket. Stop only an isolated test daemon. Cargo target dirs never under
  `/tmp` (use the default `target/` or `~/.cache`).
- No personal data anywhere in the repo: synthetic task titles and lists only; docs never
  name the user's hosts, domains or network.
- Protocol changes regenerate `schema/tdo.openrpc.json` with
  `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema`, and must pass the
  released-schema test (additions only). Enums the client *receives* are open
  (`#[schemars(transform = super::open_enum)]` plus a `#[serde(other)] Unknown` variant).
- Logs never contain task titles; gateway logs name the device and method.
- Pairing: code valid 10 minutes, single use, 5 wrong attempts void it. Device keys and codes
  are stored only as SHA-256 hashes. State files `0600` in a `0700` directory, written
  atomically.
- Cookie `twydo_device`: `HttpOnly`, `SameSite=Strict`, `Secure` unless `--insecure`,
  `Path=/`, `Max-Age=34560000`.
- Default listen address `127.0.0.1:8787`. `--origin` required unless `--insecure`.
- Revocation takes effect within 5 seconds, including closing open WebSockets.
- Tabs: Inbox first, Today second, icon-only (mailbox, calendar day with today's day number),
  `aria-label`s `Inbox` and `Today`, Lucide-style inline SVGs (ISC licence), never SF Symbols.
- vdirsyncer in the image is pinned to `0.21.0`.
- Commits pass commitlint (`type(scope): lowercase subject`, header ≤ 100, body lines ≤ 100) and
  carry no agent attribution.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` pass
  after every task (`mise run lint`); `mise run test` passes after every Rust task.

## Review Focus

1. **iOS drops the socket when the app is backgrounded.** Returning to the app must reconnect
   and refetch before rows are shown as current, never act on a stale `based_on`. Test: Task 8
   `data.test.ts` "visibility change reconnects and refetches".
2. **Daemon restarted while the phone is open** (new epoch, session gone). A write with the old
   `based_on` is refused as `stale`; the PWA refetches and shows the message; undo after a
   restart shows the daemon's refusal, not a crash. Test: Task 6 `daemon_restart_closes_socket`
   and Task 8 "stale refusal refetches".
3. **Double tap or retry after a lost answer** must write once. Test: Task 5
   `same_request_id_writes_once`.
4. **The installed home-screen app has its own cookie jar on iOS**, separate from Safari:
   pairing in Safari then installing leaves the app unpaired. Docs (Task 11) say "add to home
   screen first, then pair inside the app"; the pairing screen says so too (Task 9).
5. **Midnight while the app is open, or phone and NAS in different timezones.** Today must
   follow the daemon's `today`, refreshed by the periodic refetch. Test: Task 2
   `today_follows_daemon_timezone` and Task 8 "periodic refetch".

---

## File map

| Path | Responsibility |
|---|---|
| `crates/twydo-core/src/views.rs` | gains `inbox_list`, `default_list`, `group_label` (moved from `tdo`) |
| `crates/twydo-daemon/src/protocol/views.rs` | `views.get` params/result types |
| `crates/twydo-daemon/src/server/views.rs` | `views.get` implementation |
| `crates/twydo-gateway/` | new crate: `state.rs` (devices, codes), `bridge.rs` (roles, rewrites, event filter), `http.rs` (parsing, routes, auth), `ws.rs` (WebSocket session), `assets.rs` + `build.rs` (embedded PWA), `lib.rs` (`serve`, `Connector`) |
| `crates/tdo/src/cli/gateway.rs` | `tdo gateway serve/pair/devices/revoke` |
| `web/` | the PWA (Vite project) |
| `contrib/docker/` | `Dockerfile`, `entrypoint.sh`, `docker-compose.yaml`, `env.sample` |
| `.github/workflows/web.yml`, `image.yml`, `.github/build-setup.yml` | web CI, image build/push, dist pre-build step |
| `docs/gateway.md` | user docs |

---

### Task 1: Move Inbox resolution and group labels into twydo-core

**Files:**
- Modify: `crates/twydo-core/src/views.rs`
- Modify: `crates/tdo/src/app/newrow.rs:41-70` (remove `inbox_list`, `default_list`; re-export from core)
- Modify: `crates/tdo/src/app.rs:17`, `crates/tdo/src/ui/tasks.rs:30-50` (use core's `group_label`)
- Test: `crates/twydo-core/tests/views_labels.rs`

**Interfaces:**
- Produces (in `twydo_core::views`):
  - `pub fn inbox_list(snap: &Snapshot) -> Option<String>` (same behaviour as today's `tdo` one)
  - `pub fn default_list(snap: &Snapshot) -> String` (same)
  - `pub fn group_label(g: DateGroup, today: NaiveDate) -> Option<String>` (exact strings of
    today's `group_title`: `"OVERDUE"`, `"IN PROGRESS"`, …, `CompletedOn` → `"TODAY"`,
    `"YESTERDAY"` or `"%a %-d %b"`, `Ungrouped` → `None`)

- [ ] **Step 1: Write failing tests** `group_labels_match_tui_wording` (every `DateGroup`
  variant, including `CompletedOn` for today, yesterday and `2026-11-10` → `"Tue 10 Nov"`),
  `inbox_prefers_configured_default_list`, `inbox_falls_back_to_list_named_inbox_case_insensitive`,
  `default_list_falls_back_to_first_list`, `default_list_empty_without_lists`.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test views_labels`. Expected: FAIL (unresolved imports).
- [ ] **Step 3: Move the three functions** into `views.rs`; `tdo` uses
  `twydo_core::views::{inbox_list, default_list, group_label}` (keep `pub use` in `app.rs` so
  existing callers compile). No behaviour change.
- [ ] **Step 4: Run** `mise run test && mise run lint`. Expected: all pass (TUI render tests unchanged).
- [ ] **Step 5: Commit** `refactor(core): move inbox resolution and group labels into views`

---

### Task 2: `views.get` in the daemon

**Files:**
- Create: `crates/twydo-daemon/src/protocol/views.rs`, `crates/twydo-daemon/src/server/views.rs`
- Modify: `crates/twydo-daemon/src/protocol/mod.rs`, `src/server/mod.rs` (store the clock in `Shared`), `src/server/dispatch.rs` (arm `"views.get"`), `src/schema.rs` (`METHODS` entry), `src/server/admin.rs` (factor out unresolved conflict uids)
- Modify: `schema/tdo.openrpc.json` (blessed), `docs/protocol.md` (a `views.get` bullet)
- Test: `crates/twydo-daemon/tests/views.rs`

**Interfaces:**
- Consumes: Task 1's `inbox_list`, `default_list`, `group_label`; `views::{build_view, counts, Expansion, View, DateGroup}`; `format::relative_date`.
- Produces (`twydo_daemon::protocol::views`, all `Serialize, Deserialize, JsonSchema`):

```rust
#[serde(rename_all = "snake_case")]
pub enum ViewName { Inbox, Today }            // params: closed; unknown → invalid params
pub struct ViewsGetParams { pub view: ViewName }
pub struct ViewsGetResult {
    pub epoch: String, pub revision: u64,
    pub now: DateTime<FixedOffset>,           // daemon clock, configured timezone
    pub today: NaiveDate,
    pub title: String,                        // "Today", or the Inbox list's display name
    pub open_count: usize,                    // views::counts(..).total
    pub capture: Option<CaptureTarget>,       // None when there is no list at all
    pub sections: Vec<ViewSection>,
}
pub struct CaptureTarget { pub collection: String, pub name: String, pub due: Option<NaiveDate> }
pub struct ViewSection { pub group: SectionKey, pub label: Option<String>, pub rows: Vec<ViewRow> }
#[serde(rename_all = "snake_case")]           // open enum (Unknown variant), one per DateGroup;
pub enum SectionKey { Overdue, InProgress, LaterToday, Today, Tomorrow, DayAfterTomorrow,
    Next7Days, Next2Weeks, Next30Days, Future, Done, NoDueDate, Scheduled, Completed,
    Ungrouped, Unknown }                     // CompletedOn(_) → Completed
pub struct ViewRow {
    pub uid: String, pub title: String, pub depth: u8,
    pub parent: Option<RowParent>,            // { uid, title }, from Row::parent_hint
    pub due: Option<String>,                  // relative_date(due, now)
    pub start: Option<String>,                // relative_date(start, now), only if start > today
    pub overdue: bool, pub due_today: bool,
    pub note: Option<String>,                 // DESCRIPTION
    pub kind: TaskKind, pub repeats: bool,
    pub list: RowList,                        // { id, name, color: Option<String> }
    pub pending: bool, pub conflict: bool,
}
```

  - Inbox view = `View::List(inbox_list(snap).unwrap_or(default_list(snap)))`; with no lists:
    empty `sections`, `capture: None`, title `"Inbox"`.
  - Today's `capture` = default list with `due: Some(today)`; Inbox's = the Inbox list, `due: None`.
  - `overdue`: open and `date_group(.., false) == Overdue` (as `ui/tasks.rs::is_overdue`).
    `due_today`: due date == `today`.
  - Rows from `build_view(snap, &view, now, &Expansion::default())` on
    `shared.publisher.current().1.snap`, in order; `epoch`/`revision` from that same snapshot.
  - `Shared` gains `pub clock: Arc<dyn schedule::Clock>` set from `DaemonOptions::clock`.

- [ ] **Step 1: Write failing tests** in `tests/views.rs`, with
  `TestDaemon::start_with(&fx, |o| o.clock = FakeClock::at(<2026-11-17 09:00 UTC>))` and tasks
  written with due dates (yesterday, today 17:00, tomorrow, none) in `inbox` and `work`:
  - `today_matches_build_view`: uids and section keys equal what `build_view(&fx.snapshot(), &View::Today, now, &Expansion::default())` yields; labels equal `group_label`.
  - `inbox_lists_the_inbox_collection` and `inbox_uses_configured_default_list`.
  - `capture_targets`: Today → `{collection:"inbox", due: Some(2026-11-17)}`; Inbox → `due: None`.
  - `row_fields`: overdue row has `overdue: true` and `due: Some("Yesterday")` (whatever `relative_date` returns); note text present; `kind`/`repeats` for a repeating task.
  - `queued_write_shows_pending` (queue an edit while an outside lock is held, as `tests/read.rs::queue_edit`).
  - `conflict_flag_set_for_unresolved_conflict`.
  - `today_follows_daemon_timezone`: config `timezone = "Pacific/Auckland"`, clock at 2026-11-17 13:00 UTC → `today == 2026-11-18`.
  - `no_lists_gives_empty_inbox_without_capture`.
  - `unknown_view_is_invalid_params` (`{"view":"all"}` → kind `invalid`).
  - `revision_and_epoch_match_snapshot_get`.
- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test views`. Expected: FAIL (`method not found`).
- [ ] **Step 3: Implement** types, `server::views::get(shared: &Arc<Shared>, p: ViewsGetParams) -> Result<Value, RpcError>`, the dispatch arm, the `METHODS` entry `m("views.get", false, of::<ViewsGetParams>, of::<ViewsGetResult>)`, the clock in `Shared`, and `admin::unresolved_conflict_uids(shared) -> HashSet<String>` reused by `conflicts_list`.
- [ ] **Step 4: Bless and run** `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema`, then `mise run test && mise run lint`. Expected: all pass, `no_breaking_change_against_the_released_schema` passes.
- [ ] **Step 5: Document** `views.get` in `docs/protocol.md` ("Data, writes and events"): params, the result's role for thin clients, `based_on` from its `epoch`/`revision`, daemon time.
- [ ] **Step 6: Commit** `feat(daemon): add views.get for thin clients`

---

### Task 3: Gateway crate and device state

**Files:**
- Create: `crates/twydo-gateway/Cargo.toml` (`publish = false`; deps: `twydo-daemon`, `twydo-core`, `serde`, `serde_json`, `sha2`, `getrandom`, `chrono`, `libc`, `thiserror`), `src/lib.rs`, `src/state.rs`
- Modify: root `Cargo.toml` (member), `release-plz.toml` (a `twydo-gateway` package block like `twydo-daemon`'s, plus `git_tag_name = "twydo-gateway-v{{ version }}"` per the file's own comment; add to `tdo`'s `changelog_include`)
- Test: `crates/twydo-gateway/tests/state.rs`

**Interfaces:**
- Produces (`twydo_gateway::state`):

```rust
#[serde(rename_all = "snake_case")] pub enum Role { Phone, Display }
pub struct Device { pub id: String /* 8 hex */, pub name: String, pub role: Role,
    pub paired_at: DateTime<Utc>, pub last_seen: Option<DateTime<Utc>>, pub session: Option<String> }
pub struct Store { /* dir */ }
impl Store {
    pub fn open(dir: &Path) -> Result<Store, StateError>;   // creates dir 0700
    pub fn default_dir() -> PathBuf;  // $TWYDO_GATEWAY_STATE, else $XDG_STATE_HOME/twydo/gateway, else ~/.local/state/twydo/gateway
    pub fn new_code(&self, name: &str, role: Role, now: DateTime<Utc>) -> Result<String, StateError>; // "ABCD-EFGH"
    pub fn redeem(&self, code: &str, now: DateTime<Utc>) -> Result<(Device, String /* key */), StateError>;
    pub fn authenticate(&self, key: &str) -> Option<Device>;
    pub fn devices(&self) -> Result<Vec<Device>, StateError>;
    pub fn revoke(&self, name: &str) -> Result<(), StateError>;
    pub fn touch(&self, id: &str, now: DateTime<Utc>, session: Option<&str>) -> Result<(), StateError>;
    pub fn changed_since(&self, mark: &mut Option<SystemTime>) -> bool;   // mtime poll
}
pub enum StateError { NameTaken, NameInvalid, NoSuchDevice, CodeInvalid, Io(..), Corrupt(..) }
```

  - One file `devices.json`; every read-modify-write under an exclusive `flock` on
    `devices.lock`; write to a temp file + rename; modes `0600`/`0700`.
  - Codes: 8 Crockford base32 characters, shown `XXXX-XXXX`; matching ignores case and `-`.
    Stored `{code_sha256, name, role, expires_at, attempts_left: 5}`.
  - A wrong `redeem` decrements `attempts_left` of every pending code (attempts can't be
    attributed) and drops codes at 0 or past `expires_at`.
  - Key: 32 bytes from `getrandom`, base64url without padding; stored as `key_sha256` hex.
  - Names: 1–40 characters, no control characters, unique.

- [ ] **Step 1: Write failing tests:** `code_redeems_once`, `code_expires_after_10_minutes`
  (`now + 10m + 1s` → `CodeInvalid`), `five_wrong_attempts_void_the_code`,
  `code_matching_ignores_case_and_dash`, `authenticate_accepts_key_and_rejects_others`,
  `revoked_device_no_longer_authenticates`, `files_hold_hashes_only` (file text contains neither
  key nor code), `state_files_are_private` (modes 0600/0700), `duplicate_name_refused`,
  `changed_since_sees_admin_change`.
- [ ] **Step 2: Run** `cargo test -p twydo-gateway --test state`. Expected: FAIL.
- [ ] **Step 3: Implement** `state.rs`; `lib.rs` declares the modules.
- [ ] **Step 4: Run** `cargo test -p twydo-gateway && mise run lint`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(gateway): add the gateway crate with device pairing state`

---

### Task 4: The bridge: roles, rewrites, event filter

**Files:**
- Create: `crates/twydo-gateway/src/bridge.rs`
- Test: `crates/twydo-gateway/tests/bridge.rs`

**Interfaces:**
- Consumes: `state::{Device, Role}`; `twydo_daemon::protocol::{methods::{SubmitParams, UndoParams}, events::{Event, EventEnvelope}, rpc::{RpcError, ErrorData, ErrorKind, REFUSED}}`; `twydo_daemon::client::Notification`; `twydo_core::edit::{Op, Due}`.
- Produces:

```rust
pub struct Ctx<'a> { pub device: &'a Device, pub session: &'a str }
/// A client request checked and rewritten for the daemon, or the error to answer with.
pub fn inbound(ctx: &Ctx, method: &str, params: Option<Value>) -> Result<(String, Value), RpcError>;
/// A daemon notification as the device may see it, or None to drop it.
pub fn outbound_note(ctx: &Ctx, n: &Notification) -> Option<Value>; // full JSON-RPC notification
pub fn request_prefix(device: &Device) -> String;                     // "g<id>-"
```

  - Role table (data, one `const` per role): `phone` = `views.get`, `events.subscribe`,
    `events.unsubscribe`, `op.submit`, `op.undo`; `display` = `views.get`, `events.subscribe`,
    `events.unsubscribe`.
  - `op.submit` for `phone`: only `Op::Create(t)` with `t.parent`, `t.repeat`, `t.start` all
    `None`, `t.kind == TaskKind::Task`, `t.due` `None` or `Some(Due::Date(_))`; or
    `Op::Complete`. Anything else refused.
  - Rewrites: `session` ← `ctx.session` for `op.submit`/`op.undo`; `request_id` ← prefix +
    client id; the client's `request_id` must be 1–100 chars of `[A-Za-z0-9_-]`.
  - Refusal: `RpcError { code: REFUSED, message: "not allowed for this device", data: kind Invalid }`.
  - Notifications: forward `missed`, `heartbeat`, and events `data.changed`, `status.changed`;
    `op.finished` only when its `request_id` starts with this device's prefix (strip it), and
    only for `phone`; drop everything else, including `Event::Unknown`.

- [ ] **Step 1: Write failing tests:** `phone_allowlist`, `display_is_read_only`,
  `methods_outside_role_refused` (`lists.delete`, `daemon.shutdown`, `journal.recover`,
  `sync.run`, `snapshot.get`, `hello`), `create_with_repeat_parent_start_kind_or_timed_due_refused`,
  `delete_move_edit_refused`, `forged_session_replaced`, `request_id_prefixed_and_validated`,
  `own_op_finished_forwarded_with_prefix_stripped`, `other_devices_op_finished_dropped`,
  `job_and_sync_events_dropped`.
- [ ] **Step 2: Run** `cargo test -p twydo-gateway --test bridge`. Expected: FAIL.
- [ ] **Step 3: Implement** `bridge.rs` as pure functions (no I/O), so a later `POST /rpc` reuses them.
- [ ] **Step 4: Run** `cargo test -p twydo-gateway && mise run lint`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(gateway): allowlist and rewrite device requests per role`

---

### Task 5: HTTP server, auth and embedded assets

**Files:**
- Create: `crates/twydo-gateway/build.rs`, `src/assets.rs`, `src/http.rs`
- Modify: `crates/twydo-gateway/Cargo.toml` (add `httparse`; dev-dep `tungstenite`)
- Test: `crates/twydo-gateway/tests/http.rs`

**Interfaces:**
- Consumes: `state::Store`.
- Produces:

```rust
pub struct Asset { pub path: &'static str, pub mime: &'static str, pub bytes: &'static [u8] }
pub const ASSETS: &[Asset];            // assets.rs, generated by build.rs
pub const PLACEHOLDER: bool;           // true when web/dist was missing at build time
pub struct ServeOptions { pub listen: SocketAddr, pub origin: Option<String>, pub insecure: bool,
    pub assets_dir: Option<PathBuf>, pub state_dir: PathBuf }
pub enum Auth { Device(Device), None }
pub fn authenticate(headers: &[httparse::Header], store: &Store) -> Auth; // cookie, else Bearer
```

  - `build.rs`: walk `../../web/dist` (`cargo:rerun-if-changed`); emit `include_bytes!` entries
    with MIME by extension. Missing dir → one placeholder `index.html` ("The Twydo web app was
    not built; run `npm run build` in web/") and `PLACEHOLDER = true`; with env
    `TWYDO_REQUIRE_WEB=1` a missing dir is a build error instead.
  - One thread per TCP connection, `Connection: close`, request head ≤ 16 KiB, read timeout 10 s,
    body ≤ 4 KiB.
  - Routes: `GET /` and any static path → asset (from `--assets` dir when set, else embedded),
    SPA fallback to `index.html` for unknown paths without an extension; `POST /pair` body
    `{"code": "..."}` → `200 {"device": name, "key": key}` + `Set-Cookie` (see Global
    Constraints), wrong code → `401 {"error": "..."}`; `GET /session` → `200 {"device", "role"}`
    or `401`; `GET /ws` → Task 6 (until then `501`).
  - `GET /ws` and `GET /session` with a cookie require `Origin == --origin` (`403` otherwise);
    bearer requests skip the Origin check.
  - Static responses carry `Content-Security-Policy: default-src 'self'; connect-src 'self' <ws(s) origin>; img-src 'self' data:`
    and `X-Content-Type-Options: nosniff`. The public bundle holds no data; every data path needs a key.

- [ ] **Step 1: Write failing tests** (server on `127.0.0.1:0`, raw `TcpStream` requests):
  `serves_index_and_manifest`, `unknown_route_falls_back_to_index`, `pair_sets_cookie_with_flags`
  (assert `HttpOnly`, `SameSite=Strict`, `Secure`, `Max-Age=34560000`), `insecure_drops_secure_flag`,
  `wrong_code_is_401`, `session_requires_key`, `session_rejects_foreign_origin`,
  `bearer_key_accepted_without_origin`, `revoked_key_refused`, `oversized_head_refused`,
  `assets_dir_overrides_embedded`.
- [ ] **Step 2: Run** `cargo test -p twydo-gateway --test http`. Expected: FAIL.
- [ ] **Step 3: Implement** `build.rs`, `assets.rs`, `http.rs`.
- [ ] **Step 4: Run** `cargo test -p twydo-gateway && mise run lint`. Expected: PASS (with no `web/dist`, placeholder served).
- [ ] **Step 5: Commit** `feat(gateway): serve the web app, pairing and device auth over http`

---

### Task 6: WebSocket session over the daemon

**Files:**
- Create: `crates/twydo-gateway/src/ws.rs`
- Modify: `src/http.rs` (`/ws` upgrade), `src/lib.rs` (`serve`, `Connector`), `Cargo.toml` (`tungstenite`, `default-features = false, features = ["handshake"]`)
- Test: `crates/twydo-gateway/tests/ws.rs` (+ `tests/common/mod.rs`: an in-process daemon via `twydo_daemon::server::Daemon` and `DaemonOptions` in a temp vdir, `short_socket_dir()` as in `crates/tdo/tests/common/mod.rs`)

**Interfaces:**
- Consumes: Tasks 3–5; `twydo_daemon::client::{Client, Notification}`, `protocol::methods::HelloResult`, `protocol::rpc::MAX_LINE`.
- Produces:

```rust
pub trait Connector: Send + Sync + 'static {
    /// Connects and says hello as `tdo-gateway`, resuming `session` when given.
    fn connect(&self, session: Option<String>) -> Result<(Client, HelloResult), String>;
}
pub fn serve(opts: ServeOptions, connector: Arc<dyn Connector>) -> std::io::Result<()>;
pub fn bind(opts: &ServeOptions) -> std::io::Result<TcpListener>;                  // tests read local_addr
pub fn serve_on(listener: TcpListener, opts: ServeOptions, connector: Arc<dyn Connector>) -> std::io::Result<()>;
```

  - Upgrade: authenticate (Task 5 rules) → compute `Sec-WebSocket-Accept` with
    `tungstenite::handshake::derive_accept_key`, write `101`, `WebSocket::from_raw_socket(stream, Role::Server, None)`.
  - Per connection: `connector.connect(device.session)`; store the returned session with
    `Store::touch`. One thread owns the socket (read timeout 100 ms); requests go through
    `bridge::inbound` and are called on a worker thread via `Client::caller()`
    (`Value` → `Value`); replies, errors and `bridge::outbound_note` output go back through one
    `mpsc` channel. Text frames only, each one JSON-RPC message, ≤ `MAX_LINE` bytes; non-JSON
    → JSON-RPC parse error `-32700`.
  - Ping every 20 s. Each loop checks the device still authenticates (the store is reloaded
    when `changed_since` says so, polled every 2 s); revoked → close `1008` reason
    `device revoked`. Daemon connection closed → close `1011` reason `daemon not running`.
  - Writes logged as `gateway: device=<name> <method> <op variant> -> <outcome|error kind>`.

- [ ] **Step 1: Write failing tests** (tungstenite client with the cookie and Origin):
  `views_get_through_socket`, `complete_and_undo_through_socket` (task file shows `STATUS:COMPLETED`, then open again),
  `capture_creates_task_in_inbox`, `same_request_id_writes_once`, `two_devices_have_separate_undo`,
  `events_reach_subscribed_socket` (outside file change → `data.changed`),
  `disallowed_method_never_reaches_daemon`, `revoke_closes_socket_within_5s`,
  `daemon_restart_closes_socket` (stop the in-process daemon → close code 1011),
  `upgrade_without_key_refused`, `upgrade_with_foreign_origin_refused`, `non_json_frame_gets_parse_error`.
- [ ] **Step 2: Run** `cargo test -p twydo-gateway --test ws`. Expected: FAIL.
- [ ] **Step 3: Implement** `ws.rs`, the upgrade route, `serve`/`serve_on`/`bind`.
- [ ] **Step 4: Run** `cargo test -p twydo-gateway && mise run lint`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(gateway): bridge device websockets to the daemon`

---

### Task 7: `tdo gateway` command

**Files:**
- Create: `crates/tdo/src/cli/gateway.rs`
- Modify: `crates/tdo/src/main.rs` (subcommand), `crates/tdo/src/cli/mod.rs`, `crates/tdo/Cargo.toml` (`twydo-gateway`)
- Test: `crates/tdo/tests/gateway.rs` (uses `tests/common::tdo_bin`)

**Interfaces:**
- Consumes: `twydo_gateway::{serve, ServeOptions, Connector, state::{Store, Role}}`; `tdo::client::{Target, connect}`.
- Produces: CLI

```
tdo gateway [--state-dir PATH] serve [--listen ADDR] [--origin URL] [--insecure] [--assets DIR]
tdo gateway [--state-dir PATH] pair <NAME> [--role phone|display]
tdo gateway [--state-dir PATH] devices
tdo gateway [--state-dir PATH] revoke <NAME>
```

  - `--state-dir` default `Store::default_dir()`; env `TWYDO_GATEWAY_LISTEN`,
    `TWYDO_GATEWAY_ORIGIN` back `--listen`/`--origin` (clap `env`).
  - `serve` without `--origin` and without `--insecure` → exit 2 with
    `tdo gateway: --origin is required (or --insecure for plain HTTP on a trusted network)`.
  - `pair` prints `Pairing code for <name>: XXXX-XXXX (valid 10 minutes)`. `devices` prints one
    line per device: name, role, paired at, last seen. `revoke` of an unknown name → exit 1.
  - The `Connector` impl calls `tdo::client::connect(&target, "tdo-gateway", session)`.

- [ ] **Step 1: Write failing tests:** `serve_requires_origin`, `pair_prints_code_and_devices_lists_it`,
  `revoke_unknown_device_fails`, `end_to_end_pair_and_views_get` (spawn an isolated
  `tdo daemon`, `tdo gateway serve --insecure --origin http://127.0.0.1:<port> --listen 127.0.0.1:<port>`,
  pair, `POST /pair`, WebSocket `views.get` returns the fixture's Inbox; stop the daemon with
  `tdo daemon stop`).
- [ ] **Step 2: Run** `cargo test -p tdo --test gateway`. Expected: FAIL.
- [ ] **Step 3: Implement** `cli/gateway.rs` and the subcommand.
- [ ] **Step 4: Run** `mise run test && mise run lint`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(cli): add tdo gateway to serve the phone app`

---

### Task 8: Web project, generated types and the data layer

**Files:**
- Create: `web/package.json`, `web/package-lock.json`, `web/tsconfig.json`, `web/vite.config.ts`, `web/index.html`, `web/.gitignore` (`dist/`, `node_modules/`, `src/gen/`), `web/scripts/gen-types.mjs`, `web/src/rpc.ts`, `web/src/data.ts`, `web/src/rpc.test.ts`, `web/src/data.test.ts`
- Modify: `mise.toml` (tool `node = "24"`; tasks `web-build` = `cd web && npm ci && npm run build`, `web-check` = `cd web && npm ci && npm run check`), `.github/workflows/web.yml` (on PRs: `mise run web-check`, `mise run web-build`), `.github/workflows/pr-validation.yml` (call `web.yml`)

**Interfaces:**
- Produces:
  - `npm run gen` → `src/gen/protocol.d.ts` from `../schema/tdo.openrpc.json`
    (`components.schemas` wrapped as `definitions`, `$ref`s rewritten, compiled with
    `json-schema-to-typescript`); `prebuild`/`precheck` run it. `npm run check` = gen +
    `tsc --noEmit` + `vitest run`. Never committed.
  - `rpc.ts`: `class RpcSocket { constructor(url: string, ws?: typeof WebSocket); call<R>(method: string, params: unknown): Promise<R>; onNotification(cb: (method: string, params: unknown) => void): void; onState(cb: (s: "connecting" | "open" | "closed", reason?: string) => void): void; close(): void }` with reconnect backoff 1 s doubling to 30 s; a call on a closed socket rejects at once.
  - `data.ts`: the only module screens use (§9.4):

```ts
export type ViewName = "inbox" | "today";
export interface DataSource {
  view(name: ViewName): ReadonlySignal<ViewsGetResult | undefined>;
  connection: ReadonlySignal<"connecting" | "open" | "closed" | "unpaired">;
  refresh(name: ViewName): Promise<void>;
  complete(uid: string): Promise<WriteResult>;            // based_on from the last view fetched
  create(title: string, note: string, capture: CaptureTarget): Promise<WriteResult>;
  undo(): Promise<WriteResult>;
}
export function createDataSource(opts?: { url?: string; fetch?: typeof fetch; ws?: typeof WebSocket; now?: () => number }): DataSource;
```

  - On open: `GET /session` (401 → `unpaired`), `events.subscribe`, fetch the visible view.
    Refetch on `data.changed`, `status.changed`, `missed`; on `visibilitychange` to visible
    (reconnect first if closed); every 5 minutes while visible. A `stale` refusal refetches.
    `request_id` = `crypto.randomUUID()` per action, reused on retry. Ops are built as the
    schema says (`{"Create": NewTask}`, `{"Complete": {"uid"}}`, `Due` as `{"Date": "YYYY-MM-DD"}`).

- [ ] **Step 1: Scaffold** with `npm create vite@latest web -- --template preact-ts`, trim to the files above, add `@preact/signals`, `json-schema-to-typescript`, `vitest`, `jsdom`. Commit nothing yet.
- [ ] **Step 2: Write failing tests** with a fake WebSocket class: `rpc.test.ts`:
  "matches replies by id", "rejects calls while closed", "reconnects with backoff";
  `data.test.ts`: "unpaired when /session is 401", "subscribes and fetches on open",
  "refetches on data.changed", "visibility change reconnects and refetches",
  "periodic refetch", "complete sends based_on from the last view", "retry reuses request_id",
  "stale refusal refetches", "today capture sends due date".
- [ ] **Step 3: Run** `cd web && npm run check`. Expected: FAIL.
- [ ] **Step 4: Implement** `gen-types.mjs`, `rpc.ts`, `data.ts`.
- [ ] **Step 5: Run** `cd web && npm run check && npm run build`; `cargo build -p tdo` then
  `curl` the gateway's `/` shows the Vite `index.html` (not the placeholder). Expected: PASS.
- [ ] **Step 6: Commit** `feat(web): add the phone app project with typed json-rpc data layer`

---

### Task 9: Screens

**Files:**
- Create: `web/src/main.tsx`, `web/src/App.tsx`, `web/src/components/{TabBar,Icons,TaskList,TaskRow,DetailSheet,CaptureSheet,Banner,Pairing,Offline}.tsx`, `web/src/styles.css`, `web/public/manifest.webmanifest`, `web/public/icons/` (192, 512, `apple-touch-icon.png` 180; a simple synthetic glyph), `web/public/sw.js`, component tests `web/src/components/*.test.tsx` (`@testing-library/preact`)

**Interfaces:**
- Consumes: Task 8's `DataSource` only (screens never touch `RpcSocket`).
- Behaviour fixed by spec §9.1–§9.4 and Global Constraints. Specifics:
  - `TabBar`: two buttons, Inbox then Today, icon-only; `MailboxIcon` and
    `CalendarDayIcon({ day })` (day from the last `views.get` `today`, any view); selected tab
    uses `--accent`, other `--secondary`; last tab in `localStorage` key `twydo.tab`
    (try/catch), default `inbox`.
  - `TaskRow`: round checkbox, title, sub-line (`due` red when `overdue || due_today`, else
    `Starts <start>`), note marker, kind marker for `repeats`/project/checklist, pending dot,
    conflict marker; indent by `depth`.
  - `Banner`: one at a time, 4 s, `Completed "<title>"` / `Added "<title>" to <name>` with
    Undo; a refusal shows the daemon's message as is.
  - `CaptureSheet`: autofocused title, multi-line note, Add disabled for blank title.
  - `Pairing`: code input (`XXXX-XXXX`) and the line "Add Twydo to your Home Screen first,
    then pair inside the app."
  - `Offline`: "Can't reach Twydo" with Retry; rows shown dimmed with "last updated <time>".
  - `styles.css`: colour tokens on `:root`, dark values under `prefers-color-scheme: dark`;
    safe-area insets; iOS system font stack.
  - Manifest: `display: standalone`, `start_url: "/"`, name `Twydo`. `sw.js`: cache-first for
    `/assets/*`, network-first for navigations, never touches `/ws`, `/pair`, `/session`.

- [ ] **Step 1: Write failing component tests:** "tabs are Inbox then Today with aria-labels",
  "calendar icon shows today's day", "remembers last tab", "tapping checkbox completes and shows
  undo banner", "new banner replaces old", "refusal unticks and shows message",
  "capture from Today sends due today", "blank title cannot be added", "unpaired shows pairing",
  "closed connection shows offline state".
- [ ] **Step 2: Run** `cd web && npm run check`. Expected: FAIL.
- [ ] **Step 3: Implement** the components, CSS, manifest, icons and service worker.
- [ ] **Step 4: Run** `cd web && npm run check && npm run build`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(web): add inbox and today screens with capture and undo`

---

### Task 10: Browser tests against a real gateway

**Files:**
- Create: `web/playwright.config.ts` (WebKit, `devices["iPhone 15"]`, light and dark projects), `web/e2e/setup.ts`, `web/e2e/app.spec.ts`
- Modify: `web/package.json` (`e2e` script), `mise.toml` (task `web-e2e`), `.github/workflows/web.yml` (build `tdo`, `npx playwright install --with-deps webkit`, run e2e)

**Interfaces:**
- Consumes: the built `target/debug/tdo`, `web/dist`.
- `setup.ts`: temp dirs for `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `TWYDO_SOCKET` (short path
  under `/tmp`, mode 0700), `TWYDO_NOTIFIER=off`; writes a synthetic vdir (`inbox/`, `work/`;
  titles like `Synthetic task 1`) and a `config.toml` with `writes.mode` for tests; starts
  `tdo daemon` and `tdo gateway serve --insecure --origin http://127.0.0.1:<port> --listen 127.0.0.1:<port> --assets web/dist`;
  pairs through `tdo gateway pair e2e`; teardown runs `tdo daemon stop` and kills the gateway.

- [ ] **Step 1: Write the specs:** "pairing then Inbox renders", "Today lists the due tasks",
  "complete and undo", "capture from Inbox and from Today", "daemon stopped shows offline
  state", each in light and dark at phone size.
- [ ] **Step 2: Run** `mise run web-e2e`. Expected: PASS.
- [ ] **Step 3: Commit** `test(web): run the phone app against a real gateway in webkit`

---

### Task 11: Container image, release wiring and docs

**Files:**
- Create: `contrib/docker/Dockerfile`, `contrib/docker/entrypoint.sh`, `contrib/docker/docker-compose.yaml`, `contrib/docker/env.sample`, `contrib/systemd/tdo-gateway.service`, `.github/workflows/image.yml`, `.github/build-setup.yml`, `docs/gateway.md`
- Modify: `dist-workspace.toml` (`github-build-setup = "../build-setup.yml"`), `.github/workflows/release.yml` (regenerated by `dist generate`), `docs/architecture.md` ("The pieces" diagram and bullets), `docs/README.md` (index), `docs/install.md` (pointer), `AGENTS.md` (roadmap line), `docs/superpowers/specs/2026-11-05-phone-web-app-design.md` (status line: superseded in part by the MVP design)

**Interfaces / fixed values:**
- `Dockerfile`, three stages: `node:24` runs `npm ci && npm run build` in `web/`;
  `rust:1.99` builds `tdo --release` with `TWYDO_REQUIRE_WEB=1`; runtime `debian:bookworm-slim`
  with `tini`, `ca-certificates`, Python venv with `vdirsyncer==0.21.0`, user `twydo` (uid
  1000), `VOLUME /data`, env `XDG_CONFIG_HOME=/data/config`, `XDG_STATE_HOME=/data/state`,
  `XDG_RUNTIME_DIR=/run/twydo` (0700, owned by `twydo`), `TWYDO_SOCKET=/run/twydo/tdo.sock`,
  `TWYDO_NOTIFIER=off`, `TWYDO_GATEWAY_LISTEN=0.0.0.0:8787`, `EXPOSE 8787`,
  `ENTRYPOINT ["tini", "--", "/usr/local/bin/entrypoint.sh"]`.
- `entrypoint.sh`: start `tdo daemon` in the background, wait until `tdo daemon ping` succeeds
  (30 s), run `tdo gateway serve` in the background, `wait -n`, exit with that status (so the
  restart policy restarts both). Any other arguments are run as a command
  (`docker exec`/`run … tdo gateway pair phone` works either way).
- Compose: `ports: ["127.0.0.1:8787:8787"]`, `restart: unless-stopped`, volume `./data:/data`,
  secret file for the CalDAV password, `TWYDO_GATEWAY_ORIGIN` from `.env`. `env.sample` has
  placeholders only (`https://tasks.example.test`).
- `image.yml`: on pull requests touching `contrib/docker/**`, `web/**`, `crates/**`,
  `Cargo.lock`: build `linux/amd64` and smoke-test (start the container with a synthetic
  config, `curl /` returns the app, a WebSocket upgrade without a key gets 401). On tags
  `v*.*.*`: build natively on `ubuntu-24.04` and `ubuntu-24.04-arm`, push by digest to
  `ghcr.io/${{ github.repository }}` with `GITHUB_TOKEN` (`packages: write`), merge into a
  manifest tagged with the version and `latest`.
- `build-setup.yml`: `actions/setup-node` (24), `npm ci && npm run build` in `web/`, and
  `echo TWYDO_REQUIRE_WEB=1 >> "$GITHUB_ENV"`, so every dist binary embeds the real app.
- `docs/gateway.md`: what it is, the security model (TLS proxy that passes WebSocket upgrades,
  `127.0.0.1` binding, `--origin`, `--insecure`), Docker and plain-binary setup, the server's
  `config.toml` (same `[writes]` mode and matrix as the desktop; `[sync]` with
  `password_command = ["cat", "/run/secrets/caldav_password"]`), pairing **inside the
  installed home-screen app**, `devices`/`revoke`, roles, logs.

- [ ] **Step 1: Write** the Docker files; build and smoke-test locally:
  `docker build -f contrib/docker/Dockerfile -t twydo-test .` then the same smoke script
  `image.yml` runs. Expected: page served, unkeyed upgrade 401.
- [ ] **Step 2: Wire dist:** add `github-build-setup`, run `dist generate` and `dist plan`.
  Expected: `release.yml` gains the setup steps; `dist plan` succeeds.
- [ ] **Step 3: Write** `image.yml` and verify with `actionlint` if installed (else review by eye).
- [ ] **Step 4: Write the docs**; run `python3 tools/check_links.py`. Expected: PASS.
- [ ] **Step 5: Run** `mise run test && mise run lint && mise run web-check`. Expected: PASS.
- [ ] **Step 6: Commit** `feat(gateway): ship a container image with the daemon and the gateway`
  (docs may be a separate `docs(gateway): …` commit).

---

## After the plan

- Device interop round (phone capture/complete read by 2Do and `tdo`, spec §13) runs only
  with the user's explicit approval, on the test setup.
- After the first release that contains `twydo-gateway`, remove its `git_tag_name` line from
  `release-plz.toml` (as was done for `twydo-daemon`).
