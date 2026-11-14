# Alert editing design (issue #111)

Status: draft for review, 2026-11-14.

## Intent

Twydo shows a task's alerts and, since v0.4.0, delivers them as desktop reminders, but an alert can
only be created or removed indirectly: setting a due time adds 2Do's automatic at-due-time alert,
removing it drops that alert. The user sets reminders such as "15 minutes before", "4 hours before"
or "1 day before" and has to open 2Do on the phone to do it. This change lets them set, change and
remove a task's alert from the TUI and CLI, with the result read correctly by 2Do on the phone.

The main spec suggested `a` for alert editing; `a` is now "append item" (issue #111).

## Goals

- One **custom relative alert** per task, at any whole-minute offset before the due date/time,
  editable from the edit card and the CLI: set, change, remove.
- The written encoding is what 2Do itself writes, established by a phone capture round
  (approved by the user on 2026-11-14, bounded below) and verified in both directions.
- Undo/redo, the write queue, the compatibility gate and the notifier behave as for any other edit.

## Non-goals

- More than one custom alert per task (user: "no need for more than one alarm").
- Absolute alerts (a fixed date/time), alerts after the due time, start-relative alerts, alerts on
  tasks without a due date, nagging/snooze.
- Changing how the automatic at-due-time alert is added or removed by due-time edits, except where
  the capture shows 2Do treats it differently once a custom alert exists.

## Anti-goals

- An alert the phone misreads, shows at another time, or silently drops: worse than no feature.
- Rewriting or losing alerts Twydo does not model (absolute, multiple, foreign VALARMs). Those
  tasks keep their bytes; the alert field on them is read-only and edits are refused with a reason.
- Guessing the encoding. No write is enabled in the shipping matrix before the capture backs it.

## Constraints

- AGENTS.md device boundary: this round is the one approved device scope. Isolated test account
  and synthetic test list only; no production 2Do, NAS, account or global write-mode changes.
  Existing synthetic lists/families are not deleted. Anything outside the checklist below needs
  fresh approval.
- Evidence format: fixtures under `fixtures/set-alert/<case>/` with complete named states and
  `case.toml` (`docs/reference/evidence.md`), captured with `tools/capture_fixture.py`, scanned and
  confirmed synthetic by the user before promotion.
- Compatibility gate: a new `set-alert` operation with matrix rows; shapes not covered by the
  capture stay refused (custom matrices are version 2).
- Protocol: adding a `Change` variant changes the daemon protocol schema; refresh it as CI expects.
- Commit messages pass commitlint; no personal data in fixtures.

## Approaches

1. **Typed Alert field on the edit card (chosen).** An `Alert` row below `Repeat`, typed like the
   date and repeat fields; `A` opens the card on it, as `r` opens it on Repeat. Consistent with the
   existing editing model, shares one parser with the CLI, composes with other field edits in one
   atomic save, and gives the flexibility the user asked for (any offset).
2. **A preset picker on `A`.** Fast for a few fixed values but contradicts "more flexibility", adds
   a new UI kind, and the card would still need to display the alert. Rejected.

On evidence the user chose **capture first** over building behind a test-only matrix row.

## Phone capture round (approved 2026-11-14, steps 8–9 added with approval the same day)

Unresolved questions it answers, and nothing else:

- Q1 How 2Do encodes a custom relative alert: `TRIGGER` value and `RELATED`, `ACTION`,
  `DESCRIPTION`, and the `X-2DOAPP-*` properties (`ISAUTO`, `RELDUETIME`, `SOUND`, others), and
  whether `<2Do Meta>` JSON or other task properties change with it.
- Q2 Whether a custom alert replaces, keeps or alters the automatic at-due-time alert.
- Q3 What "1 day before" means on a date-only due (time of day, `RELDUETIME`), and whether 2Do
  allows it.
- Q4 How changing the offset and removing the alert are written.
- Q5 Whether the alert survives completion of a repeating task and moves with the new due date.
- Q6 Whether 2Do on the phone reads Twydo-written alerts correctly (shown in its alert editor at
  the right offset, still editable, no duplicate), including after a phone title edit.
- Q7 Whether a task with a separate start date encodes the alert relative to the due value, and
  what happens to a custom alert when the due date is removed.

Phase A — phone writes, Twydo captures after each batch (synthetic tasks on the test list):

1. Timed task, alert 15 min before. 2. Timed task, alert 4 h before. 3. Timed task, alert 1 day
before. 4. Date-only task, alert 1 day before (record what 2Do offers). 5. A task from 1–3, offset
changed. 6. A task from 1–3, alert removed. 7. Monthly repeating timed task with a 15 min alert,
then completed on the phone. 8. Task with a start date and a due time, alert 15 min before.
9. A task from 1–3, due date removed on the phone.

Phase B — Twydo writes with the encoding learned in phase A, phone checks:

10. Set an alert (minutes, hours and days offsets) on fresh timed tasks; set one on a date-only task
if Q3 allows. 11. Change one, remove one. 12. On the phone: open each alert editor, then edit one
title; capture the phone's writes.

The detailed checklist (aliases, expected resource counts, capture commands) belongs to the
implementation plan and is shown to the user before the round runs.

**Outcomes.** The round is the first milestone; no parser, card or CLI work starts before phase A
is captured. Rules for each answer (my calls, made during review):

- Q1/Q2/Q4: Twydo mirrors exactly what 2Do writes, including what happens to the automatic alert
  when a custom one is added and removed (if 2Do drops it on add and restores it on removal,
  Twydo does the same using the existing `auto_alarm`).
- Q3: if 2Do does not offer or misreads date-only alerts, the feature ships for timed dues only
  and a date-only task refuses with a reason.
- Q6: if the phone misreads, duplicates or loses a Twydo-written alert, work stops after phase B
  and the findings come back to the user before anything else is built.
- Q7: tasks with a separate start date are admitted only if 2Do relates the alert to the due
  value; removing the due date from a task with a custom alert mirrors 2Do (drop or refuse).

## Design

**Core.** `Change::Alert(Option<Duration>)` where the duration is the positive offset before the
due value; `None` removes the custom alert. The writer adds, replaces or removes the single custom
VALARM using exactly the phase-A encoding, preserves every other VALARM and property byte-for-byte,
and treats the automatic alert as the capture shows (Q2). `Change::Due` keeps its current
automatic-alert behaviour. A task "has a custom alert" when it has one non-automatic relative
alert before its due value in the captured shape.

**Gate.** New operation `set-alert`. Admitted: an open task or project, top-level or a project
item (not a checklist item), with a due date (timed, and date-only if Q3 allows), zero or one
custom alert of the captured shape, plus the automatic alert; repeating or not, any rule. Setting
an alert changes only VALARMs, so recurrence needs no execution evidence; the existing
`complete.start-offsets` contract already admits completing repeating tasks with relative alerts,
and Q5 confirms it for a Twydo-written one. Refused, with a reason: no due date ("an alert needs a
due date"), absolute/after-due/start-relative/multiple/foreign alerts ("this task has alerts Twydo
can't edit"), and a start date if Q7 says so.

**Other operations are unchanged.** Plain due edits and repeating completion already accept tasks
with relative alerts; the alert is relative, so it moves with the due date. The narrow existing
refusals that require only the automatic alert (from-completion due/repeat edits, UNTIL
completion, recurring-checklist scheduling) stay: they were captured without custom alerts, and
widening them is out of scope. `docs/tasks.md` says so.

**Input grammar** (shared by card and CLI): empty or `none` removes; `0`/`due`/`at due` means at
the due time if the capture shows that is a distinct custom alert, otherwise it is rejected as
redundant with the automatic one; otherwise one or more `<n><unit>` terms with units `m`/`min`,
`h`, `d`, `w` (e.g. `15m`, `1h30m`, `4h`, `1d`, `2w`), optionally followed by `before`. Whole
minutes only, positive. Days and weeks are fixed 24 h / 7 × 24 h durations, as in an iCalendar
`TRIGGER`. Display uses the largest unit that divides the offset evenly (`w`, `d`, `h`, `m`), the
rule `format::span` already follows, so `24h` reads back as `1d`; a card saved with an unchanged
alert writes nothing.

**TUI.** An `Alert` row below `Repeat` in the edit card, showing the current alert in the input
grammar (`15m`, `1d`). Hidden for checklist items, as dates and repeat are. On a task whose alerts
can't be edited it shows the Details description and is read-only; typing there explains why. `A`
opens the card on the Alert field (`r`'s pattern). Help and the Details pane list the key. Saving
with other fields is one atomic write and one undo step, as today.

**CLI.** The same option on the existing edit command (`--alert 15m`, `--alert none`), matching
how `--repeat` is spelled there.

**Notifier.** No change expected: it already plans every relative alert and coalesces same-instant
triggers. A test pins that a Twydo-written alert produces the expected reminder.

**Docs.** `docs/tasks.md` Alerts (drop "not available", describe the supported shape and refusals),
`docs/tui.md` (key table, card fields, alert input table), `docs/compatibility.md` (new rows, the
evidence), `docs/reference/evidence.md` (operation table row; record the round as complete).

## Left to the builder

Exact type and module names, the parser's location (beside the date/repeat parsers), card layout
details, the precise refusal wording, and the detailed device checklist, all within the above.

## Capture results

Phase A ran on 2026-11-14 on the production account's synthetic "Twydo test" list (user direction:
the same setup as earlier rounds), 2Do 3.8 on the iPhone. Evidence:
`fixtures/set-alert/phone-encodings` (states `phone-a-before`, `phone-a-batch1`, `phone-a-batch2`,
`phone-a-batch3-open`, `phone-a-batch3`).

**Q1 encoding.** Every alert 2Do wrote is a single VALARM of this shape (A1, 15 min):

```
BEGIN:VALARM
TRIGGER:-PT15M
ACTION:AUDIO
DESCRIPTION:A1 alert
X-2DOAPP-SOUND;SHARE-SCOPE=GLOBAL:Basso
X-2DOAPP-ISAUTO;SHARE-SCOPE=GLOBAL:1
X-2DOAPP-RELDUETIME;SHARE-SCOPE=GLOBAL:1
END:VALARM
```

`TRIGGER` is a negative duration with no `RELATED` (`-PT15M`, `-PT4H`, `-P1D`). Nothing outside the
VALARM changes except `DTSTAMP`/`LAST-MODIFIED`; `<2Do Meta>` is untouched.
`X-2DOAPP-ISAUTO:1` stays when the user edits the default alert in place (A1, A2, A7, A8) and is
absent on an alert added as new (A3; A4 had no default alert). So `ISAUTO:1` marks an alert that
started as the default, not one at the due time. `X-2DOAPP-RELDUETIME` was `0` only on A2, where the
user had to pick an exact time (06:00) because 2Do has no 4 h preset; the presets gave `1`.

**Q2.** A custom alert takes the automatic alert's place; a task never had both.

**Q3.** A date-only task (A4) takes `-P1D` with no time-of-day option; the phone shows the exact
time. Twydo counts it from local midnight of the due day, which agrees.

**Q4.** Changing the offset rewrites only `TRIGGER` (A1 `-PT15M` → `-PT30M`). Removing the alert
deletes the VALARM, and 2Do does not restore the automatic one (A2 has no VALARM).

**Q5.** Completing monthly A7 kept the open task's VALARM unchanged with its due moved to
2026-12-16 09:00, and the completed copy carries the same VALARM.

**Q7.** With a start date (A8: `StartDate` 2026-11-18 in the JSON), `DTSTART` still mirrors `DUE`
and the trigger counts from it, so the alert is 15 min before the due time. Removing the due date
(A3) drops the VALARM together with `DUE`, `DTSTART` and `duetime`.

### Rules for the build

- **Write.** Setting an alert on a task that has a VALARM rewrites that VALARM's `TRIGGER` only
  (2Do's in-place edit, as for A1/A7). On a task with no VALARM, Twydo adds the A3 shape: `TRIGGER`,
  `ACTION:AUDIO`, `DESCRIPTION:<summary>`, `X-2DOAPP-SOUND;SHARE-SCOPE=GLOBAL:Basso`,
  `X-2DOAPP-RELDUETIME;SHARE-SCOPE=GLOBAL:1`, without `ISAUTO`. Removing deletes the VALARM.
- **Recognize.** The automatic alert is `ISAUTO:1` with a zero offset. A custom alert is a single
  2Do-shaped VALARM (`ACTION:AUDIO`, `X-2DOAPP-SOUND`) with a negative relative trigger and no
  `RELATED=END`, whatever its `ISAUTO`/`RELDUETIME`. Anything else is unsupported.
- **Existing code that the capture shows wrong** (fixed on this branch):
  - `edit::is_auto_alarm` and the gate's `captured_automatic_alarm` read `ISAUTO:1` alone, so a
    due-time removal would delete an edited default alert such as A1's 30 min. They must also
    require a zero offset.
  - The notifier and `format::describe_alarm` count a relative alert from the JSON start date when
    the task has one; 2Do counts it from `DTSTART`, which mirrors `DUE`. For a task with 2Do JSON
    metadata, a relative alert counts from the due value. Without this, A8's reminder fires on
    17 Nov at 23:45 instead of 20 Nov at 09:45.
  - Removing the due date keeps a custom alert today; 2Do drops it. `Change::Due(None)` drops a
    custom alert too.
- **Admitted shapes.** Timed and date-only dues, with or without a start date, repeating or not.
- **Q6** is open until phase B.

### Phase B

Twydo (this branch, candidate matrix) set 15m, 4h and 1d on timed tasks and 1d on a date-only task;
the phone showed exactly one alert each at the expected times. Twydo then changed one to 1h30m
(`-PT1H30M`) and removed another; the phone showed 08:30 and no alert, without restoring the
automatic one. A phone title edit kept the Twydo-written trigger. **Q6 passes**:
`fixtures/set-alert/twydo-roundtrip`, and `set-alert.task` is verified.

Side observation, out of scope: on a title edit 2Do rewrites the VALARM `DESCRIPTION` to the new
title; Twydo's title edits leave it unchanged (Ruling R14 stands, now known to differ).
