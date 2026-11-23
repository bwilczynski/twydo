# Phone app icon and Today badge

Date: 2026-11-23. Status: draft for review. Issue #168. Builds on the
[phone companion MVP](2026-11-17-phone-companion-mvp-design.md) and
[Web Push](2026-11-22-web-push-design.md) designs.

Points marked *(my call)* are proposals the builder may refine with a reason.

## 1. Intent

The home-screen app needs a better icon, in the spirit of 2Do's, and the icon should carry a
badge with the number of tasks left to do today (user, issue #168). The current icon's check mark
is drawn wrong (a lopsided wedge on flat blue).

## 2. Goals, non-goals, anti-goals

**Goals**

- A new icon: a bold, rounded check mark on a blue tile, in the spirit of 2Do without copying
  its artwork *(my call)*. One SVG source in the repo, rendered to the existing
  `apple-touch-icon.png` (180), `icon-192.png` and `icon-512.png`.
- A badge on the home-screen icon showing Today's open count, the same number the Views tab
  shows for Today (`open_count`). Zero clears the badge.
- The badge is refreshed (user, approach A):
  - whenever the open app receives Today's count (the views list or the Today view), and
  - whenever a reminder push arrives, including the morning digest. A morning with nothing
    due sends no digest, so an earlier count can survive it (Codex review, 2026-11-23).

**Non-goals**

- Exact count all day while the app is closed: it drifts between pushes and the app being
  opened (user accepted, 2026-11-23).
- Extra pushes only to update the badge: iOS requires every push to show a notification.
- A badge on the desktop or for any other view.

**Anti-goals**

- A badge that breaks or blocks a reminder: a badge failure must never stop the notification.
- A count in the push that a push service can read: it rides inside the existing encrypted
  payload.

## 3. Approaches

| | Approach | Trade-off | Outcome |
|---|---|---|---|
| A | Badge set by the app while open and by the service worker on each reminder push; the push carries Today's count. | Cheap, reuses push; stale between pushes. | **Chosen** (user, 2026-11-23). |
| B | As A, plus a push on every count change. | Exact, but each push shows a notification. | Rejected: too noisy. |
| C | Badge set only while the app is open. | Simplest; wrong most of the time. | Rejected. |

## 4. Design

**Daemon.** `CommandSink` adds `TWYDO_TODAY_OPEN` (decimal) to the command's environment: Today's
open count as `views` computes it (`counts(snap, &View::Today, now).total`) at delivery time
*(my call: environment, like `TWYDO_REMINDER_KEY`, so user commands are unaffected)*. If the
count is unavailable the variable is unset. `notify.test` sets it too. Documented in
`docs/config.md` under `command`.

**Gateway.** `tdo gateway push` reads `TWYDO_TODAY_OPEN` and adds `"badge": <n>` to the payload
when it parses as a number; otherwise the field is omitted. The payload stays inside the existing
padding bucket.

**Service worker.** On `push`, alongside `showNotification`, call
`self.navigator.setAppBadge(n)` (or `clearAppBadge()` for 0) when `badge` is a non-negative
integer and the API exists; errors are swallowed. The notification is shown regardless.

**App.** When a views list or Today view result arrives, set the badge from Today's
`open_count` the same way (feature-detected, errors swallowed). No new protocol field; protocol
generation stays 4.

**Icon.** `web/icons/icon.svg` (source) rendered with `rsvg-convert` to the three PNGs; the
512 one opaque (no transparency) for iOS *(my call)*. The builder shows the rendered icon to the
user before committing it.

**Docs.** `docs/gateway.md` (badge behaviour and its drift), `docs/config.md` (new variable).

## 5. Testing

- Daemon: the existing notifier command test also checks `TWYDO_TODAY_OPEN`.
- Gateway: payload includes `badge` when the variable is set, omits it when not or malformed.
- `sw.test.ts`: push with a badge sets it; 0 clears it; missing API or a throwing API still
  shows the notification.
- App test: views result sets the badge from Today's `open_count`.
- Phone check (the user's, not authorized here): add to home screen, confirm the badge after
  opening and after a reminder.

## 6. Left to the builder

The exact shape and colours of the check mark, within §2; where in the app the badge update
hooks in.
