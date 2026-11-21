# Start time design (issue #98)

Status: draft for review, 2026-11-21.

## Intent

A task's start in Twydo is a date only: the edit card, CLI and phone form refuse `fri 9am` in
Start with "start dates have no time". 2Do itself offers a start date with an optional time
("Start / Time ▶"). The user tried to create a "change tyre" task that happens at a set time and
could not. They want start times supported for full compatibility with 2Do, read and written
correctly in both directions, and approved a phone capture round to establish the encoding.

## Goals

- Set, change and remove a start **time** (with its date) from the TUI card, `tdo op` and the
  phone web app, using the existing date grammar (`tue 10am`, `2026-11-24 10:00`).
- Read start times 2Do writes, so a task started on the phone at 10:00 shows and behaves that way.
- Views follow the time: a task with a timed start stays Scheduled until that moment and is In
  Progress from it (Today, Next 3 Days, Scheduled, date groups, phone views).
- The written encoding is what 2Do writes, established by the capture round below and verified
  both ways. Undo/redo, the write queue, the gate and the notifier behave as for any other edit.

## Non-goals

- Start-relative alerts (still out, as in the alert design).
- Changing how due dates, due times or `StartDayDelay` work beyond what the capture shows a timed
  start requires.
- A separate start-time field: the time is typed in the Start field, as for Due.

## Anti-goals

- A start the phone misreads, shows at another time or silently drops: worse than no feature.
- Rewriting a phone-set start time Twydo does not understand. Until this ships, such tasks must
  keep their bytes (the gate already treats a non-midnight `StartDate` as unusual, `gate.rs`).
- Guessing the encoding. No write is enabled in the shipping matrix before the capture backs it.

## Constraints

- AGENTS.md device boundary: this round is the approved device scope (user, 2026-11-21). Same
  setup as the alerts round: the "Twydo test" list, synthetic tasks only; no other lists, account
  or global write-mode changes. Anything outside the checklist below needs fresh approval.
- Evidence: fixtures under `fixtures/set-start/<case>/` (and `fixtures/complete/<case>/` for the
  repeat case) with complete named states and `case.toml`, captured with
  `tools/capture_fixture.py`, scanned and confirmed synthetic by the user before promotion.
- Compatibility gate: matrix rows under the existing `set-start` operation (`SetStartTask`,
  `CompleteStartOffsets`); shapes the capture does not cover stay refused.
- Protocol: `Change::Start(Option<NaiveDate>)` becomes date-or-time (like `Due`); this changes the
  daemon protocol schema, refreshed as CI expects.

## Approaches

1. **Capture first, then build read and write together (chosen by the user).** One bounded phone
   round answers the encoding questions; the parser, writer and gate are built to what was
   observed, and anything not observed is refused.
2. **Build on a guessed encoding, verify afterwards.** Faster to start, but a wrong guess writes
   tasks 2Do misreads, which the preservation contract forbids. Rejected.
3. **Read and display only, write later.** Safe, but does not fix the user's case. Rejected.

## Phone capture round (approved 2026-11-21)

Unresolved questions it answers, and nothing else:

- Q1 How 2Do encodes a start time over CalDAV: the `StartDate` instant itself (local or UTC),
  a separate JSON key (as `duetime`), or another property; whether `DTSTART` changes.
- Q2 Whether the encoding depends on the task having a due date, a due time, or neither.
- Q3 How removing only the time (keeping the date) and removing the start are written.
- Q4 Whether completing a repeating task with a timed start keeps the time and moves the date by
  the same days as the due, and what happens to `StartDayDelay`.
- Q5 Whether 2Do on the phone reads Twydo-written timed starts correctly (shown at the right
  time, editable, hides the task until then), including after a phone title edit.

Phase A: the phone writes, Twydo captures after each batch:

1. Start date + time, no due. 2. Start date + time, due date only. 3. Start date + time, due date
+ time. 4. A task from 1–3 with its start time changed. 5. A task from 1–3 with the time removed
but the date kept. 6. A task from 1–3 with the start removed. 7. A weekly repeating task with a
timed start and a timed due, completed on the phone. 8. A weekly repeating task with a timed
start and a date-only due, completed on the phone. 9. A start date + time after the March 2027
daylight-saving change (the round runs in winter time), to see whether the stored value shifts.

Phase B: Twydo writes with the encoding learned in phase A, the phone checks:

10. Set timed starts on fresh tasks covering 1–3 and 9; change one; drop one's time; remove one.
11. On the phone: open each start editor, then edit one title; capture the phone's writes.
12. Complete Twydo-written repeating tasks shaped like 7 and 8 in Twydo; check them on the phone.

The detailed checklist (aliases, expected resource counts, capture commands) belongs to the
implementation plan and is shown to the user before the round runs.

**Outcomes.** Phase A is the first milestone; no parser or writer change starts before it is
captured. Rules for each answer (my calls):

- Q1/Q3: Twydo mirrors exactly what 2Do writes for set, change, drop-time and remove.
- Q2: shapes whose encoding differs are each admitted only when captured; an uncaptured shape
  refuses with a reason.
- Q4: if 2Do moves the time with the date, completion does the same for the captured shapes
  (weekly, timed or date-only due). Monthly and yearly repeats are admitted only if the start
  moves by the due's day count exactly as for date-only starts today (H11); otherwise they stay
  refused. If the capture is unclear, completing a repeat with a timed start stays refused.
- Time zone: if 2Do stores an instant, the encoder converts with the daemon's configured
  `timezone` (already in `Ctx`), and case 9 decides whether that is the zone in force on the
  start day or at write time.
- Read before write: any timed-start shape Twydo can decode is read and shown, even if writing
  that shape is refused. A shape it cannot decode shows the date only and its bytes are kept.
- Q5: if the phone misreads or drops a Twydo-written start time, work stops after phase B and the
  findings come back to the user before anything else is built.

## Design

- **Model/read.** `meta.rs` maps `StartDate` (and any key Q1 finds) to `startdate` plus
  `starttime`; `model/task.rs::meta_start` already builds a timed `IcalDateTime` from those
  tokens. The time is read in the user's configured time zone if 2Do stores local wall time.
- **Edit.** `Change::Start` carries `Option<Due>`-like date-or-time; `edit::set_start` writes the
  captured encoding; `complete.rs` moves a timed start per Q4. `has_own_dtstart` rules stay.
- **Gate.** The "UTC midnight only" check widens to exactly the captured encodings; matrix rows
  for each admitted shape; everything else keeps today's verdict.
- **Input.** `form.rs::parsed_start` accepts a time; the "start dates have no time" error goes.
  Typing a bare date removes the time; an unchanged field sends no change (forms already compare
  text), so "drop the time" needs no new syntax.
- **`StartDayDelay`.** A non-zero delay stays refused, as today; timed starts do not change that.
- **Protocol.** The schema is refreshed; daemon, CLI, TUI and the gateway's web app ship in one
  binary, so no compatibility shim for older clients. The version bump follows the usual release.
  The phone form gets this for free through the daemon's form check.
- **Views.** `When::is_reached` already compares times; verify Scheduled/In Progress grouping and
  the Today "Later Today" group with timed starts, and the phone's `Starts …` text.
- **Notifier.** Unchanged (alerts count from the due), but its tests cover a timed start.
- **Docs.** `docs/tasks.md`, `docs/tui.md`, `docs/data-model.md`, `docs/compatibility.md` and the
  2Do reference `06-sync.md` §2.6 (observed encoding) updated; `tools/check_links.py` passes.

## Left to the builder

- Exact type for the start change (reuse `edit::Due` or a sibling).
- How the start time is displayed in TUI rows and the details pane, following the due's format.
- Test layout, following the alerts work.

## Capture results

Phase A ran on 2026-11-21 (2Do 3.8 on the iPhone, production account, synthetic "Twydo test"
list). Evidence: `fixtures/set-start/phone-encodings/` (`phone-a-before`, `phone-a-batch1`,
`phone-a-batch2`).

- **Q1 encoding.** The start time lives only in `<2Do Meta>` `StartDate`, as epoch seconds of the
  local wall-clock date and time read as UTC: start 2026-11-25 10:00 is `1795600800`
  (`2026-11-25T10:00:00Z`). No new JSON key, no change to `DTSTART` (it still mirrors `DUE`, or is
  absent without a due) or to any other property. A start without a time is the existing UTC
  midnight. So `StartDate % 86400 == 0` means a date-only start, any other value a timed start
  at `(n / 86400)` days and `(n % 86400)` seconds; seconds were always 0.
- **Q2 shapes.** Identical with no due (S1), a date-only due (S2) and a timed due (S3). The timed
  due's own `duetime` key (`1800` = 18:00) is the due's, unchanged by the start.
- **Time zone (S9).** A summer-time start (2027-04-06 10:00, captured in winter time) is
  `2027-04-06T10:00:00Z`: wall time, no zone involved. The encoder needs no time zone.
- **Q3 change, drop time, remove.** S1 10:00 → 14:30 writes `1795617000` (14:30Z); S2 time removed
  writes `1795564800` (UTC midnight, the date kept); S3 start removed drops the `StartDate` key.
  Nothing else changed in those resources beyond 2Do's usual stamps.
- **Q4 completion.** Weekly S7 (due 18:00) and S8 (date-only due), both starting 2026-11-23 09:00,
  completed on 2026-11-21: the live tasks move `DUE`/`DTSTART` by 7 days and `StartDate` to
  `2026-11-30T09:00:00Z` (time kept); the completed copies keep `2026-11-23T09:00:00Z`. This is
  the existing H11 rule (the start moves by the due's day count) applied to the whole value, so
  monthly and yearly repeats follow it too (spec Outcomes Q4).

**Admitted shapes.** Timed starts with no due, a date-only due or a timed due; changing, dropping
the time and removing; completing a repeat whose start is timed (moved by the due's days, time
kept). The recognizer: any positive `StartDate` is `(date, Some(time))` when not a multiple of
86400 and `(date, None)` otherwise. No timed-start shape is undecodable, so the spec's
"undecodable" fallback now covers only non-positive or non-integer values. `StartDayDelay`
stays as before.
