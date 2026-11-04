# RF/RA bounded capture checklist

Status: local preparation; device scope awaiting approval. Continues the
[Plan 8 checkpoint](../../superpowers/plans/2026-11-04-plan-8-editing-completeness.md).
No production reads, writes or sync are authorized by this document.

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

| Suffix | Starting state | Captured actions, one barrier after each |
|---|---|---|
| title | Monthly after done, date-only | Change title |
| notes | Same | Set notes; clear notes |
| due | Same | Change due date; set 09:00; change to 11:00; remove time |
| start | Same | Set start two days before due; change to one day before due; clear start |
| interval | Same | Monthly after done → every 2 months after done |
| anchor | Monthly from due | Monthly after done → weekly after done → weekly from due → none |
| direct | Plain dated task | Set monthly after done → none |
| complete | Monthly after done | Change due to a date distinct from completion day; complete |
| offset | Monthly after done, start two days before due | Change due retaining offset; complete |

Choose exact future dates from the actual approved session date and record them
before seed. Complete phone/Twydo twins on the same local day. A day rollover
invalidates comparison and requires rescheduling, not assumed equivalence.

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
   then edits its notes; capture `RFRA-RETURN-1`. Intermediate anchor transitions
   must each be inspected/acted on before Twydo proceeds to the next transition.
4. Return round 2: after another pull/validation, complete the edited `complete`,
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
