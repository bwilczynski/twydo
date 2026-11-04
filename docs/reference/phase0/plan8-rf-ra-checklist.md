# RF/RA bounded capture checklist

Status: user approved proceeding and confirmed current backup / 2Do 3.91.
Minimum evidence set reduced to five pairs; ten tasks seeded and synced.
Preflight and seed sync passed; preexisting local resource bytes unchanged. Continues the
[Plan 8 checkpoint](../../superpowers/plans/2026-11-04-plan-8-editing-completeness.md).
Authorization comes from the conversation; the scope remains bounded below.

## Scope and reused evidence

Target only top-level, JSON-backed monthly/weekly from-completion tasks without
ends, children or custom alerts. Enable only demonstrated `from-completion.edit`
and `from-completion.repeat` shapes. Move/delete and family operations stay outside
this increment. Zoned dates, unusual start metadata and unsupported rule shapes
remain refused.

Reuse Plan 7 R14 (`18669d33edad43c7a70152cd1b49fc0c`): plain → monthly after done,
checked by `set_repeat_matches_phone_edit_from_completion` in `edit_fields.rs`.
Reuse FC/FS completion fixtures and `twin_from_completion_matches_the_phone` /
`twin_from_completion_with_a_later_start_matches_the_phone` in `edit_complete.rs`.
These establish completion-day anchoring and start-offset movement, not editing
an existing after-done task. Do not repeat batch 1 rounds 000–153.

Writer audit: `edit/mod.rs::set_repeat` writes RRULE and JSON RecurrenceType,
RecurrenceValue and RecurrenceFrom (2 after done, 1 from due). It retains a
nonempty RUID. Removal drops RRULE and sets RecurrenceType to zero, retaining
inactive fields. Capture the phone's later normalization separately. Existing
field writers must be compared for recurrence/identity preservation before
ordinary field evidence can justify reuse. Both target matrix rows remain gated.

## Capabilities unlocked if verification passes

These are intended verified-mode capabilities, not capabilities enabled by this
planning commit. Every row remains conditional on both-direction evidence.

| User action | Exact intended scope |
|---|---|
| Edit title and notes | Existing standalone monthly after-completion task; set/clear notes |
| Edit Due | Change date, add/change time, remove time while retaining the date |
| Edit Start | Set/change/clear the start date |
| Change interval | Monthly after completion → every 2 months after completion |
| Switch to completion anchoring | Monthly from due → monthly after completion |
| Change recurrence class | Monthly after completion → weekly after completion |
| Switch back to due anchoring | Weekly after completion → weekly from due |
| Remove recurrence | Tested after-completion rule → one-time dated task |
| Add recurrence | Plain dated task → monthly after completion |

Expose verified actions through the edit card and applicable date shortcuts.
Include local undo/redo and atomic grouped-save checks. Completion is already
supported: this increment proves edited tasks still advance from completion day,
preserve start offsets and generate correct history; it does not introduce
completion itself.

Ordinary field edits are tested on monthly after-completion tasks only. Weekly
after-completion appears in recurrence transitions, not as proof of general weekly
field editing. Gate any broader row accordingly unless a documented equivalence
argument establishes it. Interval-2 coverage likewise does not establish arbitrary
intervals or every combination of edited fields.

Excluded: daily/yearly after-completion editing, recurring parents/items, end
conditions, move/delete, arbitrary imported date/time/alarm shapes, and the other
unfinished Plan 8 increments. No blanket from-completion enablement.

## Synthetic targets

Use fresh deterministic UIDs, never reuse the historical fixture UID on production.
Each suffix below denotes an independent pair: `twydo-p8-rfra-<suffix>-p` (phone
baseline) and `twydo-p8-rfra-<suffix>-t` (Twydo twin). Bind filenames and collection
IDs in a private manifest, collision-check before seed, and stop on any collision.

The minimum set is **five twin pairs (10 tasks)**. Independent fields share a
single sequential pair, with a capture after each action. Separate tasks are
needed only where changing recurrence or completing a task would destroy the
starting state needed by another comparison. The earlier 18-task offline draft
is superseded and must not be seeded.

| Suffix | Starting state and exact sequence | Why this pair exists |
|---|---|---|
| fields | Monthly after done, date-only: change title; set/clear notes; change due date; set 09:00; change to 11:00; remove time; set/change/clear start; complete | Proves ordinary writers preserve recurrence and identity. Final completion proves the edited due date did not become the completion anchor. Separate per-action captures attribute metadata changes without separate tasks. |
| interval | Monthly after done → every 2 months after done; complete | Interval changes write distinct recurrence metadata; completion proves the new interval is executed, not merely displayed. |
| anchor | Monthly from due → monthly after done → weekly after done → weekly from due → none | Tests both anchor directions and class change on an existing series, including preservation of its nonempty RUID. Final removal covers due-anchored removal only. |
| direct | Plain dated task → monthly after done → none | Tests establishing recurrence/identity on an existing plain task and removing recurrence while still after-completion anchored. Anchor's final removal cannot prove this. Prior R14 evidence is reused for setting; this pair supplies the missing phone return and removal sequence. |
| offset | Monthly after done with start two days before due: change due; complete | Proves an edited resource with a retained start offset advances correctly. Fields clears its start before completion, so cannot supply this evidence. |

Choose exact future dates from the actual approved session date and record them
before seed. Complete phone/Twydo twins on the same local day. A day rollover
invalidates comparison and requires rescheduling, not assumed equivalence.

## RFRA-00 seeded checkpoint

Ten tasks are uploaded in the existing synthetic capture A list. Their visible
names are `RFRA <suffix> p` / `RFRA <suffix> t`; UIDs follow the pattern above.
All are due 2026-12-07; offset starts 2026-12-05. The prepared source used
verified-mode creation, followed by `seed_fixture.py` identity rewriting. No
candidate gate or normal configuration was changed. Backup, manifests, before/
after captures and logs remain outside git. Seed validation found no unresolved
conflicts or prepared journal entries; upload succeeded and preexisting local
files remained byte-identical. No phone edits have been captured for RF/RA yet.

First phone barrier (edit only `p` twins, then sync and report):

1. `RFRA fields p`: change title to `RFRA fields p edited` only.
2. `RFRA interval p`: change monthly after completion to every 2 months after completion.
3. `RFRA anchor p`: change monthly from due to monthly after completion.
4. `RFRA direct p`: set monthly after completion.
5. `RFRA offset p`: change Due to 2026-12-09; leave Start untouched manually.
   Capture whether 2Do retains December 5 or shifts it automatically; do not
   compensate for its behavior before capture.

Capture 210: phone performed all five first actions. Sync passed; exactly the
five `p` resources changed and no other vdir files changed. Title editing left
metadata intact; interval changed RecurrenceValue 1 → 2; anchor changed
RecurrenceFrom 1 → 2; direct setting established monthly after-completion and a
phone series identity. Offset Due/DTSTART moved to December 9 while metadata
StartDate remained unchanged (December 5). Both conflict check and untouched-twin
comparison passed. These are baseline observations, not operation enablement.

Next baseline barrier: set fields notes to `RFRA notes`; change anchor to weekly
after completion; remove direct's repeat. Leave interval/offset and all `t` twins
unchanged. Capture before any further action.

Capture 220: exactly those three resources changed; no unrelated files or twins
changed. Fields DESCRIPTION is `RFRA notes`. Anchor RRULE becomes FREQ=WEEKLY,
RecurrenceType 258 → 257, retaining RecurrenceFrom 2. Direct removal drops RRULE
and clears RecurrenceType/Value/From to zero. The writer currently retains the
inactive Value/From fields, so this difference requires an explicit comparison
and phone return check; do not silently exclude it as stamps/layout.

Next barrier: clear fields notes; switch anchor to weekly from due. All other
tasks stay unchanged. These are separate captures from note setting and the
weekly after-completion transition above.

Capture 230: fields notes cleared; anchor RecurrenceFrom 2 → 1 with weekly
RRULE retained. Exactly those two resources changed; all twins and unrelated
files remained unchanged. Next barrier: change fields Due to 2026-12-09
(date-only); remove anchor recurrence. Do not edit other tasks.

Capture 240: fields Due is December 9, date-only, with monthly RRULE retained;
anchor removal drops RRULE and clears RecurrenceType/Value/From. Exactly those
two resources changed; twins and unrelated files remain unchanged. Anchor and
direct baseline transition chains are now complete. Next barrier: add 09:00
due time to fields only, leaving its December 9 date unchanged.

Capture 250: only fields changed; metadata gained duetime 900 and monthly
recurrence remained intact. Twins and unrelated resources are unchanged.
Next barrier: change fields due time from 09:00 to 11:00, retaining December 9.

Capture 260: fields DUE/DTSTART are December 9 at 11:00; duetime changed 900 →
1100. Exactly one automatic PT0S alarm remains. Only fields changed; twins and
unrelated files are unchanged. Next barrier: remove its due time while keeping
December 9 (date-only), to capture time/alarm cleanup.

Capture 270: fields is date-only December 9 again, duetime is absent, and the
automatic alarm is removed. Exactly that resource changed; twins/unrelated files
are unchanged. User requested batching to reduce sync back-and-forth: combine
independent task actions at every remaining barrier. Preserve intermediate
captures only where overwriting the same task would lose distinct evidence.

Next combined baseline barrier:

- Fields: set Start to December 7, retaining Due December 9 and monthly after
  completion. This captures adding a start offset on an existing series.
- Interval: complete once. This proves the edited two-month interval executes
  from completion day and creates correctly linked history.
- Offset: complete once. This proves the prior due edit retained a usable start
  offset and completion advances it consistently.

Do these three actions before one phone sync. Do not complete fields yet, and
leave anchor/direct and all `t` twins untouched. Completion history means the
next capture must validate expected new resources, not require the old count.

Capture 280 (2026-11-04): fields gained December 7 Start with December 9 Due.
Interval completion advanced Due to 2027-01-04, retaining interval 2 and completion
anchoring. Offset completion advanced Due to 2026-12-04 and Start to 2026-11-30,
preserving the four-day gap left by the earlier Due edit. Exactly two completed
non-recurring history resources appeared, one for each completed series. Only
the three expected originals and those histories changed; twins and unrelated
files remained unchanged. Full writer/identity comparisons remain pending.

Remaining baseline barriers: (1) change fields Start from December 7 to December
8, then capture; (2) clear fields Start and complete it once, then capture live
and history resources. The history snapshot provides the cleared-start state
before advancement. If the phone inserts or changes a start during completion,
stop and request a separate pre-completion clear capture rather than infer it.
These sequential edits share one task and cannot be batched into one sync
without losing the intermediate changed-start evidence.

Do not complete anything or edit `t` twins yet. Each task has only one action in
this barrier, so a single synchronized capture attributes every change. Stop and
report if the phone does not offer an action or displays an unexpected shape.

## Fixed capture barriers and phone return rounds

Plan **two phone return rounds** after baseline capture; each contains the fixed
actions above, not an open-ended request to explore the app.

1. `RFRA-00`: after fresh approval, backup/conflict checks and allowlisted seed,
   capture all twins before edits. Phone performs the `-p` actions, with numbered
   `RFRA-P-<suffix>-<action>` snapshots after each. Pull and validate each snapshot.
2. Local checkpoint: compare resources, implement only evidenced behavior, run
   ordinary core/gate tests and offline verified-mode TUI writer tests. Record
   exact field reuse arguments and any narrower gate restrictions.
3. Return round 1: pull and validate latest phone state before any local write;
   Twydo performs corresponding `-t` actions with `RFRA-T-<suffix>-<action>`
   snapshots. Upload only approved targets. Phone inspects each resulting shape,
   then edits its notes (use a distinct synthetic marker and preserve the state
   needed by the next comparison); capture `RFRA-RETURN-1`. Intermediate anchor transitions
   must each be inspected/acted on before Twydo proceeds to the next transition.
4. Return round 2: after another pull/validation, complete the edited `fields`,
   `offset` and `interval` twins on the phone and capture `RFRA-RETURN-2`, including
   live resources and every new history resource. Compare completion anchor,
   due/start delta, RUID and fresh history identities against phone baselines.
   Capture matching baseline interval completion in this round as well.

RF field actions must not be collapsed into one save for baseline comparison.
Any unexpected normalization, unsupported phone action, conflict, date rollover,
or mismatch stops enablement. Explain and request approval for any extra device
round before performing it; do not silently expand this checklist.

## Closeout and approval boundary

- Compare intended standard fields and metadata, identity/history links and
  unrelated properties; ignore only explicitly justified stamps/layout differences.
- Add assertions to existing `edit_fields`, `edit_complete`, `gate`, store and
  TUI write suites, including negative neighboring shapes and atomic grouped-save
  refusal with text retained. No session-named test target.
- Verify card saves, date shortcuts, repeat set/change/remove, completion and
  local undo/redo for enabled shapes. Do not infer move/delete coverage.
- Update matrix evidence, scoped gates, compatibility/task docs and the session
  record together; run `mise run test`, `mise run lint`, links and diff checks.
- Stop with either both-direction evidence plus passing local/TUI checks, or a
  precise unsupported/unverified result. Never mark a broad row pass without
  documenting its writer-equivalence argument and effective shape restrictions.

Before execution, obtain fresh approval for these exact synthetic targets in the
two existing throwaway lists, backup confirmation, named sync pair and two return
rounds. Bind account/collection IDs, versions, timezone, config and shared lock
identity privately. No new lists, account/global mode changes, unrelated tasks or
test-list deletion are included. Preserve remote edits and both conflict versions.
