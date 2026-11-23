# Repeat unblock design (issues #172, #174, part of #29)

Status: draft for review, 2026-11-23.

## Intent

In daily use the user was refused twice by Twydo and had to finish in 2Do on the phone:

- #174: setting "3m after done" (every 3 months, from completion) on an existing project or
  checklist (the user recalls it was one of the two).
- #172: changing a project's repeat, including making it repeat after completion
  (`set-repeat.project: iphone_baseline` unverified).

They want both to work in Twydo, together with the neighbouring repeat refusals that one short
phone session can clear (#29's weekly, interval-2, ending and from-completion checklists). The
session must be cheap for them: Twydo seeds every task, and the phone only edits and completes.
Evidence is reused wherever it already proves something; new phone work covers only what is
genuinely unobserved.

## Goals

- Set any monthly or weekly "after done" rule, any interval, on a standalone task (#174, no
  phone needed) and on a project or checklist (#174's real case, via the round).
- Set "after done" on a task with a start date, and complete it.
- Set, change and remove a project's repeat, including "after done", from Twydo; read the same
  change made on the phone (#172).
- Complete a recurring project that repeats after done.
- Create (directly or by converting a task) and complete recurring checklists that repeat
  weekly, every 2 weeks/months, after done, or end on a date (#29).

## Non-goals

- Yearly checklists, projects that end (count or until), reopening an after-done task (undo
  already covers a mistaken tick), daily after-done edits.
- Nested families, independently repeating checklist items, repeating children of a repeating
  project, start/alert/delay combinations on the newly admitted family shapes.
- Any change to the encodings Twydo writes: all rules are written exactly as today.

## Anti-goals

- Admitting a shape only because a neighbouring one passed, where the phone could plausibly
  behave differently (family reset, history copies, completion-anchored dates).
- Asking the user to type task titles or notes on the phone, or to create tasks there.
- A partial write: a refused part of a save still writes nothing.

## Constraints

- AGENTS.md device boundary: the phone round below needs the user's explicit approval of its
  checklist before it runs. As in the alerts and start-time rounds, it runs on the production
  account's synthetic "Twydo test" list; no other lists, account or global write-mode changes.
- Evidence: complete named states under `fixtures/<operation>/<case>/` via
  `tools/capture_fixture.py`, scanned and confirmed synthetic by the user before promotion;
  `python3 tools/check_evidence.py` and `python3 tools/check_links.py` pass.
- Spec §11 contracts (compatibility gate, preservation) cannot be overridden: a matrix verdict
  becomes `pass` only with an observation behind it.

## Approaches

1. **#174 alone, no phone.** Cheapest, but #172 stays blocked. Rejected by the user.
2. **#174 plus a project-only round.** Fixes exactly the two refusals; #29 stays open.
3. **#174 plus one lean round covering projects and the #29 checklist variants (chosen).** One
   session, about fifteen taps, no typing, every task seeded by Twydo; yearly checklists, ending
   projects and after-done reopening are left out as not blocking.

## Part 1: #174 without a phone round

Today `gate::captured_from_completion_edit` admits only the captured RF/RA transitions:
interval 1, plus monthly interval 1 → 2. Creating the same rules is already admitted
(`create.task` + `set-repeat.from-completion`, evidence `complete/recurrence-profiles`,
`set-repeat/encoding-roundtrip`), and an edit writes the same recurrence fields a create does
(`RRULE`, `RecurrenceFrom` 2, the 2Do codes; `set-repeat.interval` passes). So:

- Admit `Change::Repeat` and `Change::RepeatInterval` to monthly or weekly after-done rules with
  any interval Twydo can complete (`interval_refusal` still applies), within the existing shape
  guards: standalone open JSON task, due day 1–28, date-only (or the captured automatic alarm
  for field edits), no end, start, custom alert or delay.
- The captured anchor transitions keep their table, with interval ignored on both sides (so an
  edit may change anchor and interval at once). Admitted (old → new) pairs: none → monthly after
  done; monthly due → monthly after done; monthly after done → weekly after done; weekly after
  done → weekly due; monthly or weekly after done → none; and any after-done rule → the same
  frequency after done with another interval. Pairs not listed (for example weekly due → weekly
  after done) stay refused; they change where the next date comes from, which only the RF/RA
  captures cover.
- Part 1 alone does not fix the user's real case (a family); Part 2 does. Part 1 ships first
  because it is independent and removes the same refusal for standalone tasks.
- Tests: extend `verified_from_completion_edit_rows_do_not_enable_uncaptured_shapes` with
  "3m after done", "2w after done", `RepeatInterval(3)`, and refusals for daily/yearly/ended.
- Docs: the "Repeat from completion" row of `docs/compatibility.md` and footnote ².

## Part 2: one phone round

### Unresolved questions (and nothing else)

- Q1 (#172 baseline, phone → Twydo): does the phone change a project's repeat to "every 3 months
  after completion" while keeping its items and links, and does Twydo read the result as such?
- Q2 (#172 Twydo → phone): does the phone show and keep a Twydo-written after-done repeat on a
  project, set by an edit (not a create)?
- Q3: what does the phone write when it completes a project that repeats after done (next due
  from the completion day, item reset, history copy), and does Twydo's completion match it?
- Q4 (#29): the same for recurring checklists that repeat every 2 weeks, monthly after done, and
  monthly ending on their due date (the last occurrence completes in place).
- Q4b: what does the phone write for the start date when it completes a task that repeats after
  done and has a start date (keep the gap before the due, move by the due's days, or other), and
  does Twydo's completion match it? The RF/RA captures cleared the start before completing.
- Q5: does the phone show the Twydo-completed twins the same as its own completions?

### Seeds (Twydo writes all of them before the round)

Each family has two open items, one already ticked by Twydo (so the reset is visible). Twins
are byte-for-byte equal apart from identity.

| Alias | Shape | How Twydo makes it |
|---|---|---|
| P1a, P1b | Project, every 3 months after done | Created as plain monthly project, then repeat edited (answers Q2) |
| P2 | Project, plain monthly | Created |
| C1a, C1b | Checklist, every 2 weeks | Created |
| C2a, C2b | Checklist, every 3 months after done | Created as a task, then converted to a checklist |
| C3a, C3b | Checklist, monthly until its own due | Created |
| T1a, T1b | Task, every 3 months after done, start date 3 days before due | Created plain with the start, then repeat edited |

Twydo writes these with a private candidate matrix in test mode (never the shipped matrix), on
a branch daemon pointed at the "Twydo test" list.

### Phone actions (no typing)

1. Sync; open P1a and P1b and look at their repeat (Q2).
2. P2: set repeat to every 3 months, after completion, with the picker (Q1).
3. Complete P1a, C1a, C2a, C3a, T1a (Q3, Q4, Q4b).
4. Twydo captures, then completes P1b, C1b, C2b, C3b, T1b the same day and uploads.
5. Open P1b, C1b, C2b, C3b, T1b and say in chat whether each matches its "a" twin (Q5).

The detailed checklist (aliases, expected resource counts, capture commands, rollback) belongs
to the implementation plan and is shown to the user for approval before the round runs.

### Reused evidence (why the round is this small)

- Item reset, family history copies and identity handling: `complete/family-lifecycle`,
  `complete/checklist-reset` (monthly checklists and plain recurring projects).
- Rule advancement for weekly, interval, after-done and until: `complete/recurrence-profiles`,
  `complete/checklist-and-until` on plain tasks.
- Phone-side checklist repeat edits: `set-repeat.checklist` already passes all three ways, so no
  phone checklist edit is needed.
- Task → checklist conversion preserves recurrence bytewise: C2 extends it to a new rule shape at
  no extra phone cost.

The round only observes those pieces combined on a family.

### Outcomes (my calls)

- Twin match on all of Q3–Q5 and a clean Q1/Q2: `set-repeat.project` becomes `pass` both ways;
  `PROJECT_END` stops refusing after-done (projects that end stay refused); the recurring-checklist
  creation, conversion, repeat-change and completion guards widen as the composition rule
  below states.
- Composition (my call, the reviewer's main question). Each axis is observed on a checklist by
  the round (frequency weekly in C1, interval 2 in C1 and 3 in C2, after-done anchor in C2, until
  in C3), and
  the axes' combinations are already observed on plain tasks (`complete/recurrence-profiles`).
  The interval is one encoded field, independent of frequency, anchor and family
  (`set-repeat.interval`). So the admitted checklist rules are {weekly, monthly} × any interval
  Twydo can complete × {due, after done}, plus monthly interval 1 ending on a date-only until; all with the
  existing checklist guards (top-level, date-only day 1–28, no start/alert/delay, no cancelled
  items).
- Checklist interval edits. A Twydo-side interval edit writes the same fields as creating that
  rule (the Part 1 argument); the phone-side change is already covered by `set-repeat.checklist`.
  `set-repeat.checklist-interval` therefore becomes `pass` with C1's case as evidence, rather
  than being deleted, so the verdict keeps a named observation.
- Projects. The round observes the phone setting one transition (plain monthly → 3 months after
  done) and Twydo writing one by edit. Admitted on that, with the Part 1 transition table
  applied to top-level flat projects: due-anchored ↔ after-done monthly, after-done interval
  changes, after-done or due-anchored → none, none → monthly after done. Removal writes the
  same fields as removal on a task. Project completion keeps the existing guards
  (`recurring_complete_shape`: starts, alerts, delays, late completion) and, like checklists,
  refuses cancelled items. Projects that end stay refused.
- Any mismatch: that shape stays refused, the finding is recorded as a known failure, and the
  rest still ships. If the phone misreads a Twydo write (Q2/Q5), work stops and comes back to the
  user before anything else is built.
- Start dates (T1). On a match, after-done edits and completion admit a date-only start on a
  standalone task; the writer moves it as the phone did. Timed starts on after-done repeats stay
  refused (not captured). Starts on after-done families stay refused.
- Fixtures: `fixtures/set-repeat/project-roundtrip/`, `fixtures/complete/project-from-completion/`,
  `fixtures/complete/checklist-variants/`.

## Design touch points

- `crates/twydo-core/src/gate.rs`: `captured_from_completion_edit` (Part 1);
  `PROJECT_END` check, `captured_checklist_write`, `checklist_repeat_shape`, the
  recurring-checklist completion guard (Part 2).
- `compat/matrix.toml`: `set-repeat.project`, `set-repeat.checklist-interval`, evidence lists.
- `crates/twydo-core/src/edit/complete.rs`: only if Q3/Q4 show behaviour the existing family
  completion does not produce (for example the after-done next due on a family).
- Docs: `docs/compatibility.md`, `docs/reference/evidence.md` (remaining restrictions), the
  issues closed by the PR.
- No protocol or encoding change, so no breaking release.

## Order and stop points (my calls)

- Part 1 is its own commit set and does not depend on the round.
- The plan then prepares the round: seed script, private candidate matrix and staging outside
  git (as `~/twydo-<topic>-round/` in earlier rounds), and the step-by-step checklist. It stops
  there for the user's approval.
- I run the round with the user: a branch daemon on the "Twydo test" list with the production
  `twydo.service` stopped, restored afterwards. Seeds stay in "Twydo test" like earlier rounds'
  synthetic tasks; the user deletes them whenever they like.
- After the round, each shape (projects, C1, C2, C3) is enabled by its own commit, conditional on
  its own outcome; a failed shape gets a known-failure note instead. A Q2/Q5 misread stops all
  Part 2 work.

## Left to the builder
- Exact due dates for the seeds (day 1–28, a future date so the twins advance identically).
- Test layout, following the existing gate and `edit_complete.rs` twin tests.
