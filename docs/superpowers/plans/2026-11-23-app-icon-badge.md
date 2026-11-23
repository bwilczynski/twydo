# Phone App Icon and Today Badge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A new home-screen icon and a badge on it showing Today's open count, refreshed by the
open app and by every reminder push (issue #168).

**Architecture:** The daemon's notifier computes Today's open count at delivery and hands it to
the command sink as `TWYDO_TODAY_OPEN`; `tdo gateway push` puts it in the encrypted payload as
`badge`; the service worker sets the app badge on push. The open app sets the badge from Today's
`open_count` whenever its data source learns it. The icon is one SVG rendered to three PNGs.

**Tech Stack:** Rust (twydo-core, twydo-daemon, twydo-gateway, tdo), TypeScript/Preact + vitest
(`web/`), plain JS service worker (`web/public/sw.js`), `rsvg-convert`.

**Spec:** `docs/superpowers/specs/2026-11-23-app-icon-badge-design.md`

## Global Constraints

- Badge value = Today's open count as `twydo_core::views::counts(snap, &View::Today, now).total`
  (the Views tab's `open_count`). 0 clears the badge.
- A badge failure (missing API, rejected promise, bad value) never stops or delays the
  notification.
- No new protocol field; protocol generation stays 4.
- Env var name exactly `TWYDO_TODAY_OPEN`, decimal; payload field exactly `badge` (JSON integer).
- The payload stays within the existing padded size (`PLAINTEXT_LEN`); `badge` is never cut.
- Commits pass commitlint (`type(scope): lowercase subject`); no AI attribution trailers.
- Cargo target never under `/tmp`.

## Review Focus

1. A push with `badge: 0` (everything done) must remove the badge: use `clearAppBadge()` for 0
   rather than relying on `setAppBadge(0)` (Task 3 test `push_badge_zero_clears`).
2. `TWYDO_TODAY_OPEN` set to garbage (`"x"`, `"-1"`, `""`) must push without a `badge` field,
   not fail the push (Task 2 test `badge_env_garbage_is_omitted`).
3. The app opened on Inbox (never visiting Views or Today) must still set the badge
   (Task 4 test `today_count_known_when_opened_on_inbox`).
4. Completing a task in the open app must lower the badge without reopening the app: the count
   refreshes on `data.changed` (Task 4 test `today_count_refreshes_on_data_changed`).
5. `navigator.setAppBadge` throwing synchronously (not only rejecting) must not break the push
   handler or the app (Task 3 `push_badge_api_throwing_still_shows`, Task 4 `badge.test.ts`).

---

### Task 1: Daemon passes Today's open count to the command

**Files:**
- Modify: `crates/twydo-core/src/remind.rs:123` (`Delivery`), and every `Delivery { … }`
  literal (`grep -rn "Delivery {" crates`)
- Modify: `crates/twydo-daemon/src/server/notifier/mod.rs` (`pass`, `notify_test`)
- Modify: `crates/twydo-daemon/src/server/notifier/sink.rs:113` (`CommandSink::deliver`)
- Modify: `docs/config.md:178`, `docs/architecture.md:560`
- Test: `crates/twydo-daemon/tests/notifier.rs` (`env_probe_command`,
  `command_gets_the_reminder_key_and_uid`)

**Interfaces:**
- Produces: `remind::Delivery { key, uid, title, body, today_open: Option<usize> }`; the planner
  sets `None`; the daemon fills `Some(n)` before `sink.deliver`. The command's env gains
  `TWYDO_TODAY_OPEN=<n>` (removed when `None`).

- [ ] **Step 1: Extend the env test.** `env_probe_command` prints
  `"$TWYDO_REMINDER_KEY|${TWYDO_REMINDER_UID-unset}|${TWYDO_TODAY_OPEN-unset}"`. In
  `command_gets_the_reminder_key_and_uid` assert the output is `format!("{KEY}|u1|{n}")` where
  `n` is the `open_count` of `views.get {"view":"today"}` from the same daemon at the same fake
  clock (use the file's existing RPC helper). Fix any other assertion that used the old format.
- [ ] **Step 2: Run, expect FAIL** (`…|unset`):
  `cargo test -p twydo-daemon --test notifier command_gets_the_reminder_key_and_uid`
- [ ] **Step 3: Implement.** Add the field; in `pass`, compute the count once per pass from the
  same `snap.snap` the plan used, with `now` in `snap.snap.tz()`, and set it on each `cleaned(d)`.
  `notify_test` does the same from `self.publisher.current()`. `CommandSink::deliver` adds
  `("TWYDO_TODAY_OPEN", d.today_open.map(|n| n.to_string()).as_deref())`. Platform sinks ignore it.
- [ ] **Step 4: Run, expect PASS:** `cargo test -p twydo-daemon -p twydo-core`
- [ ] **Step 5: Docs.** `docs/config.md` `command` row and `docs/architecture.md:560`: name
  `TWYDO_TODAY_OPEN` (Today's open count at delivery, also for `tdo notify test`).
  `python3 tools/check_links.py` passes.
- [ ] **Step 6: Commit** `feat(daemon): pass today's open count to the notification command`

### Task 2: Gateway push carries the badge

**Files:**
- Modify: `crates/twydo-gateway/src/push/crypto.rs:96-111` (`Payload`, `json`), its tests
- Modify: `crates/tdo/src/cli/gateway.rs:190` (push command), `crates/twydo-gateway/tests/push.rs:18`
- Modify: `docs/gateway.md` (badge section)

**Interfaces:**
- Consumes: env `TWYDO_TODAY_OPEN` (Task 1).
- Produces: `Payload { …, badge: Option<u64> }`; JSON gains `"badge": n` when `Some`.
  `pub fn badge_from_env(v: Option<&str>) -> Option<u64>` in `crates/twydo-gateway/src/push/mod.rs`
  (re-exported like `Payload`).

- [ ] **Step 1: Tests in `crypto.rs`:** `badge_in_payload` (`badge: Some(3)` → unpadded JSON has
  `"badge":3`); `no_badge_no_field` (`None` → no `badge` key); `long_body_keeps_badge` (a body
  long enough to be shortened still yields `"badge":3` and 1024 bytes). Tests in `push/mod.rs`:
  `badge_env_parses` (`Some("7")` → `Some(7)`, `Some("0")` → `Some(0)`) and
  `badge_env_garbage_is_omitted` (`None`, `Some("")`, `Some("x")`, `Some("-1")` → `None`).
- [ ] **Step 2: Run, expect FAIL:** `cargo test -p twydo-gateway`
- [ ] **Step 3: Implement** the field, `json` insertion, `badge_from_env`; the push command sets
  `badge: badge_from_env(std::env::var("TWYDO_TODAY_OPEN").ok().as_deref())`.
- [ ] **Step 4: Run, expect PASS:** `cargo test -p twydo-gateway -p tdo`
- [ ] **Step 5: Docs.** `docs/gateway.md`: the app's icon badge shows Today's open count; it is
  set when the app loads data and on each reminder push (including the morning digest), and may
  be stale in between. `python3 tools/check_links.py` passes.
- [ ] **Step 6: Commit** `feat(gateway): send today's open count with each push`

### Task 3: Service worker sets the badge on push

**Files:**
- Modify: `web/public/sw.js` (`push` handler)
- Test: `web/src/sw.test.ts`

**Interfaces:**
- Consumes: payload `badge` (Task 2).
- The `load()` test harness gains `navigator: { setAppBadge, clearAppBadge }` on its fake
  `self` (both `vi.fn(() => Promise.resolve())`), with an option to omit or make them throw.

- [ ] **Step 1: Tests:** `push_badge_sets_count` (`badge: 4` → `setAppBadge(4)`, notification
  shown); `push_badge_zero_clears` (`badge: 0` → `clearAppBadge()`, no `setAppBadge`);
  `push_without_badge_leaves_it` (no field, `badge: -1`, `badge: 1.5`, `badge: "3"` → neither
  called); `push_badge_api_missing_still_shows` (no `navigator` API → notification shown);
  `push_badge_api_throwing_still_shows` (sync throw and rejected promise → notification shown
  with the same options as `push_shows_title_body_and_tag`).
- [ ] **Step 2: Run, expect FAIL:** `npm --prefix web test -- sw.test`
- [ ] **Step 3: Implement** a `badge(n)` helper in `sw.js`: only for non-negative integers,
  feature-detected on `self.navigator`, wrapped in try/catch with `.catch(() => {})`; called
  before `showNotification`; its promise is not part of `waitUntil`'s critical path for the
  notification (use `Promise.all` with the badge promise already made non-rejecting).
- [ ] **Step 4: Run, expect PASS:** `npm --prefix web run check`
- [ ] **Step 5: Commit** `feat(web): set the app badge from reminder pushes`

### Task 4: Open app keeps the badge current

**Files:**
- Create: `web/src/badge.ts`, `web/src/badge.test.ts`
- Modify: `web/src/data.ts` (`DataSource`, `fetchVisible`), `web/src/data.test.ts`
- Modify: `web/src/main.tsx` (one effect)

**Interfaces:**
- Produces: `export function setBadge(n: number | undefined, nav?: BadgeNav): void` in
  `badge.ts`, where `type BadgeNav = { setAppBadge?(n: number): Promise<void>; clearAppBadge?(): Promise<void> }`
  defaulting to `navigator`. `DataSource.todayCount: ReadonlySignal<number | undefined>`.

- [ ] **Step 1: `badge.test.ts`:** `undefined` → nothing called; `0` → `clearAppBadge`; `5` →
  `setAppBadge(5)`; missing methods → no throw; sync throw and rejection → no throw, no
  unhandled rejection.
- [ ] **Step 2: `data.test.ts`** (give the `views.list` handler a Today entry with
  `open_count: 5`, and `views.get` today `open_count: 2`):
  `today_count_from_views_list` (visible `"views"` → 5);
  `today_count_from_today_view` (refresh `{view:"today"}` → 2);
  `today_count_known_when_opened_on_inbox` (open on Inbox → 5 after the refetch, via one
  `views.list` call); `today_count_refreshes_on_data_changed` (change the handler to 4, notify
  `data.changed` → 4).
- [ ] **Step 3: Run, expect FAIL:** `npm --prefix web test -- badge data`
- [ ] **Step 4: Implement.** `setBadge` per the tests. In `data.ts` a `todayCount` signal set
  from every `views.list` result (the entry whose `view.view === "today"`) and every
  `views.get` result for Today. After `fetchVisible` succeeds for any other target, call
  `views.list` without awaiting it in the coalesced loop (errors ignored) to update the count.
  Adjust existing tests that count `views.list` calls only where this extra call changes them.
  In `main.tsx`: `effect(() => setBadge(ds.todayCount.value))`.
- [ ] **Step 5: Run, expect PASS:** `npm --prefix web run check`
- [ ] **Step 6: Commit** `feat(web): keep the app badge at today's open count`

### Task 5: New icon

**Files:**
- Create: `web/icons/icon.svg` (source, outside `public/` so it is not shipped)
- Modify: `web/public/icons/apple-touch-icon.png`, `icon-192.png`, `icon-512.png`

- [ ] **Step 1: Draw `icon.svg`**, 1024×1024 viewBox, full-bleed opaque square (iOS and
  Android mask the corners): a blue vertical gradient tile (around `#1e90ff` → `#0060df`) and a
  bold white check mark drawn as one stroked path with round caps and joins (stroke ≈ 110), centred
  with ≥ 20% margin so masks never clip it. In the spirit of 2Do, not a copy of its artwork. A
  comment at the top names the render command below.
- [ ] **Step 2: Render:**
  `for s in 180:apple-touch-icon 192:icon-192 512:icon-512; do rsvg-convert -w ${s%%:*} -h ${s%%:*} -b '#0060df' web/icons/icon.svg -o web/public/icons/${s#*:}.png; done`
  Check: `file web/public/icons/*.png` shows the three sizes.
- [ ] **Step 3: Show the 512 PNG to the user and wait for approval** before committing;
  iterate on the SVG on feedback.
- [ ] **Step 4: Verify the build embeds them:** `npm --prefix web run build`
- [ ] **Step 5: Commit** `feat(web): new app icon`

### Final

- [ ] `cargo test --workspace`, `npm --prefix web run check`, `python3 tools/check_links.py`.
- [ ] Commitlint: `npx --yes -p @commitlint/cli -p @commitlint/config-conventional commitlint --from=origin/main --to=HEAD`.
- [ ] Phone check is the user's: add to home screen, confirm the badge after opening the app
  and after a reminder.
