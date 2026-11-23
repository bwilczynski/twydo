# Phone settings screen (issue #167)

## Intent

The Reminders switch sits under the last card of the Views screen and reads as part of that
list. It moves to a Settings screen of its own. One switch alone would leave the screen bare,
so Settings also shows what the phone can tell about its daemon: sync state, the CalDAV server
and versions.

## Goals

- A gear button at the top right of the Views screen's large title opens Settings, pushed like
  a view, with a `‹ Views` back button. Re-tapping the Views tab pops back to the list.
- Settings shows, in this order:
  1. **Reminders**: the existing switch and its hints, moved unchanged from `ViewsScreen`.
  2. **Sync**: the sync headline, the detail line when there is one, the last successful sync
     time (phone-local time), the CalDAV server host, and the reminders failure when there is
     one. Shown whether or not anything needs attention.
  3. **About**: the daemon version and the protocol generation.
- The Views screen no longer has the Reminders row.

## Non-goals

- Warnings stay on the task views where they are now (user's decision); Settings repeats them.
- No new settings beyond Reminders: no sync controls, unpairing or configuration editing.
- No display-role access to the new method.

## Constraints

- No credentials, usernames or URL paths reach the phone: the server is shown as host (and
  port when not the default), never the full URL.
- The gateway's per-role allowlist decides what a phone may call.
- Docs: `docs/gateway.md` and `docs/protocol.md` change with the behaviour.

## Approaches

1. **New daemon method `settings.get`, allowed for phones** (picked). Returns the daemon version,
   protocol, sync summary, reminders summary and server host. Settings fetches it when it opens.
   One small, well-defined addition; fields can grow without touching views.
2. An `about` block on `views.list`: no new method, but every Views refresh carries data only
   Settings uses, and the result mixes two jobs.
3. The gateway answers version itself: the gateway does not know the CalDAV server, so the
   daemon is needed anyway and the job splits in two.

## Decided

- Gear top right of the Views screen; Settings pushed inside the Views tab (user).
- Approach 1 (user).

## Builder's calls (proposals)

- `settings.get` takes no params and returns
  `{ version, protocol, server: Option<String>, sync: Option<SyncSummary>,
  reminders: Option<RemindersSummary> }`, reusing the types `views.get` already sends and the
  same code that fills them. `server` is absent when sync is not configured.
- Adding a method is additive: no new protocol generation. Against an older daemon the call
  fails with "method not found"; Settings then shows the Reminders switch and a short
  "Daemon info unavailable" line instead of the Sync and About sections.
- Settings remembers being open like an opened view does (the `twydo.open` storage key), so a
  reload returns to it.
- Offline: Settings shows what it last fetched, dimmed like other screens; the switch is
  unaffected since it talks to the gateway, not the daemon.
