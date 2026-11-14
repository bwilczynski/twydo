# Notifier (Plan 2)

Status: proposed, awaiting approval. Amends the [main design](2026-10-31-terminal-tasks-design.md)
§5 "Notifications" and §7 "Notification delivery contract", and fills in the
[daemon design](2026-11-09-daemon-design.md) §9. Where this document differs from those
sections, this document wins once approved; the user asked for the old contract to be challenged
against the current code (2026-11-14).

## Intent

Twydo shows tasks but never reminds. The user wants desktop reminders on Omarchy and macOS that
fire with the TUI closed, so that Twydo can replace 2Do on the desktop. A missed reminder is an
anti-goal of the whole project; a duplicate one is an accepted cost.

## Goals, non-goals, anti-goals

Goals:

- Every alert (`VALARM`) of an open task produces a desktop notification at its time.
- Every open task with a due **time** notifies at that time, even without an alert.
- One **morning digest** a day at `morning_time` names the date-only tasks due today and the
  overdue tasks *(user, 2026-11-14: option 1)*.
- Reminders missed while the daemon was down or the machine slept are delivered on start or wake.
- Works on Linux (`notify-send`, shown by Omarchy's mako) and macOS.
- A way to check that delivery works on a machine without waiting for a task to fall due.

Non-goals:

- Complete or Snooze buttons, nagging repeats (main spec §4.7: plain reminders).
- Editing alerts in the TUI (`a`); it stays a later feature.
- A `reminder.due` event for connected clients, and Web Push for the phone app. Both can be
  added as further sinks later (see "Shape").
- Exactly-once delivery. Two machines running a daemon each notify, as does 2Do on the phone.

Anti-goals:

- A reminder silently lost: a failed delivery marked as sent, or a trigger skipped because the
  daemon was not running at that instant.
- A notification for a task that is completed, deleted, or in a hidden or archived list.
- A test run that pops real notifications on the developer's desktop or in CI.
- The notifier writing a task file, or task titles appearing in the daemon log.

## What changed since the main spec

- The daemon exists and owns the snapshot, the state directory and a wall-clock scheduler
  thread. A `tdo notify` command on a minute timer is gone (daemon design §9).
- Repeats are copy-and-advance: one live task carries one due value and completing it moves that
  value. There is no recurrence expansion; "occurrence identity" is the trigger's instant.
- Alerts are already parsed into `Alarm { trigger: Relative | Absolute }`, and Twydo already
  writes 2Do's automatic at-due-time alert when a due time is set, so "alert at due" and
  "implicit due reminder" usually coincide.
- `morning_time` already exists in the configuration, reserved for this.

## Approaches

1. **A notifier thread inside the daemon over a pure planner (chosen).** A pure function in
   `twydo-core` turns a snapshot, the sent-state and "now" into the deliveries that are due and
   the next instant to wake at. A daemon thread calls it, runs the platform command, and records
   what succeeded. It sleeps until the next instant, capped so that it notices a wake from
   sleep, and is poked when a new snapshot is published. Matches the scheduler that exists,
   needs no extra unit, and is testable with a fake clock and a fake sink.
2. A separate `tdo notify` process on a systemd/launchd minute timer, as the main spec proposed.
   Rejected: a second unit to install, a second reader of the vdir beside the single writer, up
   to a minute of lateness, and the daemon design already decided against it.
3. Hand each trigger to the operating system (`systemd-run --on-calendar`, launchd one-shot
   agents) when a task changes. Rejected: two unrelated implementations, stale timers to cancel
   on every edit and sync, and nothing for the digest or catch-up.

Inside approach 1, what notifies (the user's choice among three): alerts and timed dues
individually with date-only tasks in one morning digest (chosen); one popup per due task as the
main spec said (rejected: ten date-only tasks mean ten popups at 09:00); alerts only plus the
digest (rejected: a timed task without an alert would stay silent).

## Design

### Which tasks

A task is eligible when it is open, its list is neither hidden nor archived (the same test the
views use), and it is not a checklist item (a child of a checklist). Project children are
eligible on their own. A start date in the future does not suppress anything: an alert is an
explicit request.

The notifier reads the daemon's published planning snapshot, which already has queued edits
laid over the files. A completion that is queued behind a sync therefore suppresses its
reminders at once, and the notifier never sees a half-written operation.

### Triggers

Each eligible task yields zero or more *timed triggers*, each an instant:

- **Alert, absolute:** its UTC instant.
- **Alert, relative:** the anchor plus the offset. The anchor is the due value when the alert is
  marked related-to-end or the task has no start, otherwise the start value (the rule
  `format::describe_alarm` already states). An alert with no anchor value yields nothing.
  A date-only anchor counts from local midnight of that date *(my call, from 2Do's "counts days
  from the start date and fires at the time you set"; inferred, not observed on a device)*.
- **Due time:** the due instant, when the due value has a time.

Values resolve through the existing `IcalDateTime::resolve`: floating and date-only values in the
configured timezone, zoned values in their own zone, UTC as is.

**Identity and coalescing.** A delivery's key is `(task UID, instant)`. Triggers of one task at
the same instant are one delivery, so the automatic at-due alert and the due time give one
popup; alerts at different times stay separate. When a repeating task advances or a date is
edited, the instants change and the new ones can notify; the old keys simply never match again.
This replaces the main spec's `(UID, occurrence identity, trigger identity)` triple with the
same guarantees and no separate occurrence anchor.

**Morning digest.** Once per local date, at `morning_time` in the configured timezone, one
notification lists eligible tasks that are due today with no time, and eligible tasks that are
overdue (a due date before today, or a due time before today's `morning_time`). The digest is
decided once per local date, at the first wake at or past `morning_time`: when both groups
are empty it is skipped for that day (the date is still recorded) and nothing is sent later that
day, so a task that falls due or is added later has only its own reminder. Its key is the local
date. A task stays in each morning's digest until it is done: that is the intended daily
repeat.

### When things are delivered

On every wake the planner returns:

- every timed trigger with `now - catch_up < instant <= now` whose key is not recorded as sent;
- today's digest, when `now` is at or past `morning_time` and today's date is not recorded;
  when it is empty, the date to record without delivering anything.

and the earliest future instant (a trigger or the next digest time) to wake at.

So there is one rule for normal running, first run, restart, wake from sleep and a late sync:
anything from the last `catch_up` (default 24 hours) that has not been sent is sent now; anything
older is not sent individually and shows up as overdue in the next digest; a digest missed in
the morning is decided at the first plan that day (when the hold below allows it), by the same
morning-instant rule. A task that arrives by sync with an alert two hours in the past notifies
once, late. This replaces the main spec's separate
catch-up summary, supplementary summary and summary coalescing.

**The hold.** After a start, and after a wake from sleep, the snapshot is the one from before
the machine was off, and the sync that follows may complete, move or add tasks (a date-only task
that arrived overnight belongs in the day's digest, which is decided once; a task finished on the
phone must not fire). So the notifier holds: while held, a pass delivers nothing, records
nothing (no skip keys either) and does not plan; it waits, capped at 5 seconds as usual. A hold
starts when the notifier starts and when a pass sees the clock more than 60 seconds ahead of
what really passed since the previous pass (measured against a monotonic clock, so a slow pass
with hung commands is not taken for a sleep; the wait cap is 5 seconds, so a larger surplus is a
sleep or a forward jump; a backward move starts none, and pulls a running hold's start back so
its 120 seconds count from the corrected time). Any such gap restarts the hold, also one that is running: it begins
again at that moment and only a sync that ends after it releases it. It ends when a sync finishes after it started, whatever the result
(after a failed sync there is nothing better to wait for), or 120 seconds after it started on
the notifier's clock, whichever comes first; the notifier then plans at once, and everything that
fell due meanwhile goes out by the catch-up rule. The hold applies only when a sync will come:
`[sync]` is configured, automatic sync is on and the sync is not backing off after a failure
(the conditions under which a background sync runs); otherwise there is none. `notify.test`
ignores the hold. A sync's end reaches the notifier from the one place every kind of sync (startup,
periodic, batch, manual, list) publishes it.

A timed trigger that falls within a minute of the digest is not folded into it.

Clock changes: a rollback cannot replay anything, because recorded keys are instants and dates,
not "last run" marks. A forward jump is a catch-up like any other.

### State

`notifier.json` in the daemon's per-vdir state directory holds the sent keys with the time they
were sent, and the dates a digest was sent. It is rewritten atomically (temporary file, then
rename) after each successful delivery. `t:` entries older than twice the catch-up window are
dropped on write; a digest entry (`d:<date>`) is kept until its date is more than 2 days before
the current UTC date, so a short window cannot make the digest be decided twice in a day. A missing or unreadable file counts as empty, with one log line; the cost is possible
duplicates within the window, never a lost reminder. A crash between delivering and recording
duplicates that one reminder (main spec trade-off, kept).

### Delivery

One command per notification, started without a shell, title and body as separate arguments,
with a timeout of about 10 seconds:

| Platform | Command |
|---|---|
| Linux | `notify-send --app-name=Twydo <title> <body>` |
| macOS, `terminal-notifier` on `PATH` | `terminal-notifier -title <title> -message <body> -group <key>` |
| macOS otherwise | `osascript` with a fixed script that reads title and body from its arguments (`on run argv`), never by building script text from task content |
| `[notifications] command` set | that program and its arguments, then title and body as the last two arguments |

A non-zero exit, a timeout or a missing program is a failure: the key is not recorded, the
delivery is retried on later wakes with backoff (from a few seconds up to five minutes), and
the failure is visible: a key that fails backs off on its own (5 seconds, doubling) while the
other deliveries of the pass go on, control characters are replaced by spaces in the text before
any sink sees it, and a pass that panics (an alert the planner cannot handle) is reported as
`the reminder planner failed; see the daemon log` and retried with the same backoff, never ending
the thread; `StatusView` gains an optional `notifier_error` text, shown where the
TUI and `tdo status` show `drain_error` today, cleared by the next success. A delivery that keeps failing until its instant leaves the
catch-up window is dropped like any other old trigger and the task appears in the digest. The log names UIDs
and keys only.

macOS caveat to document: without `terminal-notifier`, notifications appear under "Script
Editor". The systemd user service already has the session bus; the launchd agent runs in the
GUI session; a daemon started on demand inherits the client's environment.

**Text.** Timed trigger: title is the task's title; body is one line such as
`Due today 14:30 · <list name>`, or `15 min before due · Due today 14:30 · <list name>` for an
alert that is not at the due time (reusing `format::describe_alarm` and `relative_date`). An
absolute alert on a task without a due date shows only the list name. Digest: title
`3 due today, 2 overdue` (omitting an empty half); body lists up to five titles, due-today
first, then `+N more`. Exact wording is the builder's.

### Shape

- `twydo-core`: a pure module (no clock, file or process access, like `views`) with the trigger
  rules, eligibility and the planner: `plan(snapshot, sent, now, settings) -> (deliveries,
  next_wake)`. A delivery carries its key, title and body.
- `twydo-daemon`: `server/notifier.rs`, a thread shaped like `schedule.rs` (the same `Clock`
  trait, a capped wait so a sleep is noticed within seconds, `poke` on every published snapshot
  and configuration change, stopped at teardown before the state directory lock is released).
  It owns the state file and a `Sink` trait with the platform sink, the configured-command sink
  and a recording sink for tests. Later sinks (a client event, Web Push) plug in here.
- The daemon's default sink in library code and tests is "none"; only the `tdo daemon` binary
  path selects the platform sink. Test configurations also set `enabled = false` or a recording
  command, so neither in-process nor process-level tests can reach a real notification service.

### Configuration

```toml
morning_time = "09:00"      # existing key, now used

[notifications]
enabled = true              # false: the notifier does nothing
catch_up_hours = 24         # how far back a missed reminder is still delivered
command = ["my-notify", "--flag"]   # optional; title and body are appended
```

Changes apply to the running daemon at once. The table is not sent to clients (like
`[daemon]`), because a client one release older rejects configuration keys it does not know.

### Checking a setup

`tdo notify test` asks the daemon (new method `notify.test`) to send one fixed notification
through the configured sink and reports success or the command's error. It works with
`enabled = false` too, records nothing, and is how the macOS path gets verified, since this
work is developed on Linux.

### Documentation

`docs/tasks.md` (Alerts), `docs/config.md` (`morning_time`, `[notifications]`, the line about
hidden and archived lists), `docs/install.md` (the service unit is what makes reminders
reliable; `terminal-notifier`; `tdo notify test`), `docs/protocol.md` and the OpenRPC schema
(`notify.test`, `notifier_error`), `docs/architecture.md`, the main spec (a pointer to this
amendment at §5 and §7), and the roadmap paragraph in `AGENTS.md`.

## Tests

Planner (pure, synthetic fixtures): several alerts; alert and due time at one instant coalesce;
relative alerts against due, start, and date-only anchors; a repeating task's next occurrence
notifies after completion; edits that move or remove a trigger; completed, deleted, checklist
item, hidden and archived suppression; floating, zoned, UTC and date-only values, including a
DST change; the catch-up window edge; the digest's content, its once-a-day rule, a late digest
and an empty one.

Notifier thread (fake clock, recording sink): restart does not resend; a failed delivery is
retried and never recorded; `notifier_error` appears and clears; a sleep-sized clock jump
catches up; a clock rollback replays nothing; a snapshot published mid-wait is picked up; a
queued completion suppresses; `enabled = false`; a configuration reload; a corrupt state file;
teardown. The duplicate-on-crash trade-off is shown by a test that crashes between delivery and
recording.

End to end: `tdo daemon` with a recording `command` delivers a due trigger and answers
`tdo notify test`. One manual check on Omarchy with a synthetic vdir and the real `notify-send`
(this pops one real notification), and one by the user on macOS.

## Left to the builder

Names of modules, types and the status field; notification wording; the backoff steps and the
wait cap; whether the planner re-scans all tasks on each wake or keeps an index (a few thousand
tasks every few seconds is cheap); how test helpers disable delivery; the plan's task split.
