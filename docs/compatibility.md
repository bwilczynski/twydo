# Living alongside 2Do

Twydo is meant to be used together with 2Do on the iPhone: both apps edit the same tasks through
a CalDAV server. This page explains what that needs, how Twydo avoids writing anything the
iPhone could misread, and what has been checked.

## What you need

- **2Do on the iPhone, syncing over CalDAV.** 2Do's Dropbox sync is not supported: Twydo reads and
  writes iCalendar files, and 2Do uses that format only for CalDAV.
- **A CalDAV server that stores tasks (`VTODO`).** Testing used Synology Calendar; any server that
  keeps tasks as 2Do uploads them should work.
- **vdirsyncer**, to mirror the server into a local folder ([sync.md](sync.md)).

2Do for the Mac has not been tested (it needs a subscription that was not available). The iPhone
app is the reference for everything below.

## Moving from Dropbox to CalDAV

Switching 2Do's sync method replaces what the server holds, so do it carefully:

1. **Back up.** In 2Do, make a backup. If you use Dropbox sync, also copy the Dropbox folder
   `Apps/2Do` somewhere safe (copy it, never move it).
2. **Take an inventory** of the Dropbox data, so you can compare after the move:
   `python3 tools/decode_2do_dropbox.py <copy of Apps/2Do> inventory.2do.json`. The output holds
   your task data: keep it out of any repository.
3. **Create an empty task collection** on the CalDAV server.
4. **In 2Do, add the CalDAV account** and upload your data to it. Use *Replace Server Data* only
   when the destination collection is empty.
5. **Set up vdirsyncer and Twydo** ([sync.md](sync.md), [install.md](install.md)) and pull the
   collections.
6. **Compare**: `tdo export-json > export.json`, then
   `python3 tools/reconcile.py inventory.2do.json export.json` reports tasks and lists that do not
   match. Its output contains list names (and titles with `--show-titles`).

Then run `tdo check`: it reports data health, repeat-class evidence and the actual
permissions for the tasks it can load. A class verdict is not a promise that every
operation on those tasks is available.

## What Twydo keeps intact

Twydo changes only what an operation is about. A file it does not change is never rewritten; in a
file it changes, every unrelated line keeps its exact bytes, including properties and metadata
Twydo does not understand. Details: [data-model.md](data-model.md#what-twydo-preserves).

Nested checklist/project items use `DisplayOrder`, then creation time, rather than the
containing list's date/title sort; Details uses the same sequence. The
[ordering check](reference/phase0/issue35-checklist-order.md) covered undated items under
non-recurring checklist and project parents: phone-created sequences, phone appends,
Twydo appends displayed last on the phone, and subsequent phone renames preserving identity
and position. Existing siblings remained byte-unchanged. The later
[reordering round](reference/phase0/issue41-item-reorder.md) verified explicit ranks with
**Manually** selected on the phone, including preservation through phone renames. This
does not add phone-side drag reordering or widen recurrence gates.

## Verified mode

Most writes first require an iPhone-originated capture, so Twydo can use the same
representation (the `iphone_baseline` column). Twydo-originated item reordering/insertion
has the bounded exception described [below](#item-reordering), because the phone does not
offer the reorder operation. A write counts as
verified once Twydo's own result was also checked on the iPhone: read back correctly, and edited
again (the `twydo_to_iphone` and `iphone_to_twydo` columns). The results are recorded in
[`compat/matrix.toml`](../compat/matrix.toml), which is built into `tdo`. Many writes have a
baseline but are not verified yet.

By default (`[writes] mode = "verified"`), Twydo runs **only** writes the matrix marks as passing,
for the kind of task at hand. Anything else is refused before a file is touched, with the reason
on the status line (or on stderr from `tdo op`, which exits 1):

```
months-last-day repeats are not verified yet
reopen for yearly tasks is not verified yet (twydo_to_iphone)
nested projects and checklists are not verified yet
converting a project with items to a checklist is not supported by 2Do
```

"Not verified yet" means Twydo has not checked the write against the iPhone. "Not supported by
2Do" means the iPhone itself cannot do it (it greys out the type selector on a project with
items), so there is nothing to verify and the write stays refused in every mode, including `test`.
The bounded Twydo-originated ordering exception below is checked separately.

The part in parentheses names the direction that is missing: `twydo_to_iphone` (the iPhone has
not been checked reading Twydo's version) or `iphone_to_twydo` (Twydo has not been checked
reading the iPhone's). Reading is never refused: every task is shown.

`tdo check` separates three questions:

- **Data health:** task count, per-file parse errors and model-level read-only resources,
  including completed tasks. These counts alone do not establish write permissions.
- **Recurrence evidence:** preservation, execution and editing verdicts for repeat classes
  on open tasks. Shape restrictions and operation-specific evidence still apply.
- **Operation availability:** allowed, refused, inapplicable and needs-input counts for
  concrete actions, using the configured write mode and the writer's gate. Refusals are
  grouped by action and reason. Title and notes edits are separate; complete applies to
  open tasks and reopen to completed tasks. Cancelled tasks have neither lifecycle probe.

Dates, repeats, conversion, creation, moves and sibling placement need proposed values
or a destination/context; the report does not invent successful edits for them. It is a
permission snapshot: later conflicts, changed dependencies and other store preconditions
can still prevent a write. An empty input explicitly reports no tasks to assess. If files
cannot be parsed, permission counts cover only loaded tasks.

Task titles and parse-error paths are omitted unless `--verbose` is supplied; task notes
are never printed by this report. Exit 0 means a report was produced, even with refused
actions or individual parse errors. Exit 2 means a configuration, invocation or vdir-read
failure. An explicit `--matrix` that cannot be loaded is an error; without it, the matrix
built into the binary is used.

For one task, `tdo op check <uid>` uses the same probes and reports `allowed`,
`refused` with a reason, `inapplicable`, or `needs input`. It does not treat a title
edit as permission for arbitrary fields, or a new plain task as proof for creating
a matching family. Both commands include queued edits in their task view and use
`[writes] mode`; neither starts a sync or drains pending writes. Existing startup
journal recovery still applies. This increment improves reporting; it does not
change which writes are admitted.

`[writes] mode = "test"` relaxes the check: it drops the `twydo_to_iphone` and `iphone_to_twydo`
columns of the operation rows and the checklist item-dates edge case (both apply only in verified
mode), but still needs the phone's baseline (`iphone_baseline`, except the bounded
Twydo-originated ordering operations), the rule verdicts and the safety rules, and still refuses the repeat kinds the iPhone misreads. It exists for testing against a
separate test account, and must never point at your real data: an unverified write can produce
something 2Do on the iPhone reads differently. See [config.md](config.md).

## What is verified today

The [editing-completeness audit and plan](superpowers/plans/2026-11-04-plan-8-editing-completeness.md)
tracks the remaining operations, imported-shape restrictions and later editing features. The
table below summarizes individual matrix cases, not every combination: a recurring project
also needs its own repeat's operation rows, for example, so its monthly/yearly repeat can still
block a move even though the recurring-project row passes. `tdo op check <uid>` gives the
effective result for an existing task.

The [Plan 8 phone session](reference/phase0/plan8-tui-completeness-session.md#regression-test-map)
finished its first batch: five logical synthetic cases, eight newly passing operation rows,
and narrowly guarded `until` execution. Its later project/item, lifecycle and native-date
batches remain unfinished. A passing row is combined with the shape restrictions below.

| Kind of task | Create | Ordinary edit | Complete | Reopen | Move | Delete |
|---|---|---|---|---|---|---|
| One-time task | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Monthly repeat (any day, Nth weekday) | ✅ | ✅ | ✅ | — | — | ✅ |
| Yearly repeat (29 February included) | ✅ | ✅ | ✅ | — | — | ✅ |
| Daily repeat | ✅ | ✅ | ✅ | — | — | — |
| Weekly repeat | ✅ | ✅ | ✅ | — | — | — |
| Repeat from completion | ✅ | Monthly only² | ✅ | — | — | — |
| Checklist | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Recurring checklist | Monthly only³ | Text/conversion only³ | Monthly only¹ | — | — | — |
| Project | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Recurring project | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Repeating item of a project | — | ✅ | ✅ | ✅ | — | ✅ |

“—” means not verified yet. Repeat changes use separate rows from ordinary edits. The complete
column also needs the rule/modifier's execution verdict and the gate's scheduling/family checks.

Parent Delete columns describe whole-family deletion. Selected-item deletion uses the two
dedicated rows below, independently of the item's ordinary task/repeat operation rows.

The newly enabled operations are:

| Matrix case / operation | Verified scope |
|---|---|
| `daily.repeat` | Set, remove or change between plain due-anchored daily/weekly/monthly/yearly rules, without ends or weekday selectors |
| `yearly.repeat` | Same scope; existing interval encodings are preserved |
| `checklist.repeat` | Set/remove plain date-only monthly/weekly recurrence on an existing checklist |
| `recurring-checklist.repeat` | Change/remove those monthly/weekly rules: interval 1, no end, start, alert, delay or completion anchor |
| `recurring-checklist.create` | ³Direct plain monthly checklist creation and adding items under an open monthly checklist: interval 1, date-only day 1–28, due anchored, no end/start/alert/delay |
| `recurring-checklist.edit` | ³Title/notes on those open parents/items; convert a top-level open monthly task to a checklist, preserving recurrence. No due/start/time/interval or other type changes |
| `checklist-item.delete` | Remove only the selected item, including completed/last items and items under completed parents; preserve parent/sibling bytes, type, state and schedules |
| `project-item.delete` | Same operation, including independently recurring children and recurring project parents; no advancement or history mutation |
| `recurring-checklist.complete` | ¹Plain monthly day 1–28, interval 1, date-only, due anchored, no end/start/alert/delay; mixed or all-done items, no cancelled items |
| `ended-until-daily.reopen` | Reopen the final original of a top-level date-only interval-1 daily until series, without children/start/alerts/delay |
| `ended-until-monthly.reopen` | Same for plain monthly day 1–28 |
| `history-copy.reopen` | Top-level non-recurring, date-only task copies linked by RUID; no children/start/alerts/delay |
| `from-completion.edit` | ²Standalone open monthly interval-1 field edits, day 1–28 date-only/floating due; no end, zoned dates or custom alarms. Pure date-only monthly interval 1 → 2 also passes |
| `from-completion.repeat` | Captured interval-1 transitions: plain or monthly due → monthly completion, monthly completion → weekly completion or none, and weekly completion → weekly due; standalone open JSON tasks, date-only day 1–28, no end/start/alert/delay |

`until` completion now passes for top-level plain daily/monthly day 1–28 tasks with interval 1,
date-only due/end dates, no children/start/alerts/delay and a due-date anchor. Continuation,
inclusive boundary, final in-place completion and completion after reopening were checked both
ways. Other until combinations stay refused even though offset/base-rule evidence passes.
Count-ended originals, family history and other ended-series reopen shapes remain refused.

A recurring checklist's completion copies the family before resetting live items and removing
inherited dates. Item history keeps earlier individual completion times. Independent item starts
or due times, including start-only items without a due date, are refused without partial writes.
The [B-only checklist session](reference/phase0/plan8-checklist-29-checklist.md) verifies
monthly checklist creation, conversion, title/notes, item addition/ticking and individual item
deletion within scope ³. New items carry inherited parent-date copies; reset removes those
live copies. A phone-converted parent's internal identity is retained live, while its history
parent and copied items receive consistent new identities. Deletion removes exactly one item;
parent/sibling read dependencies protect apply, undo, redo and recovery against remote changes.
Weekly/yearly creation/conversion/completion, ordinary due/start/time/interval changes, and
completed-parent item creation/reopening remain unverified. A mixed card
save that includes a refused field writes nothing and retains its text. Pure previously captured
monthly/weekly repeat changes remain supported separately.

Existing repeat completion support remains: daily, weekly, weekday sets, monthly including the
29th–31st and Nth/last weekdays, yearly including February 29, from completion and count ends.
The [Plan 7 session](reference/phase0/plan7-capture.md) supplies those fixtures. Recurring projects
that end or repeat from completion remain refused. Moving one item, editing existing
weekly/interval-2 from-completion tasks, project repeat changes, empty-project-to-checklist
conversion and native alert editing remain unverified. The [audit](superpowers/plans/2026-11-04-plan-8-editing-completeness.md)
tracks the remaining combinations; this batch does not certify the entire TUI surface.

The [issue 27 session](reference/phase0/issue27-item-deletion.md) verifies representative
checklist/project item deletions in both directions, including last/completed items, a
completed parent, monthly/weekly parents and an independently recurring project child.
All six phone deletions and six Twydo deletions remove exactly one resource; the phone then
edits all four Twydo-empty parents while preserving type, dates, rules and completion state.
Local exact-preservation tests generalize this invariant across recognized schedules,
statuses, alerts, history and opaque content, with durable family checks through queue,
undo/redo and recovery. This is operation-level generalization, not a device test of every
combination. Nested families, ambiguous identities (including multiple UID properties or
parent links), broken links, unreadable metadata and unrecognized rules stay refused. No recurrence-execution support is inferred from deletion.

The [RF/RA checklist](reference/phase0/plan8-rf-ra-checklist.md) records both-direction
phone checks and exact writer comparisons for the two from-completion rows. Retained inactive
repeat codes after removal were accepted and edited by the phone. The interval edit works
through the card and CLI; mixed interval/field saves are refused atomically and keep typed
text. Daily/yearly rules, endings, families, ordinary weekly/interval-2 edits and custom alarms
remain outside this evidence. Setting completion recurrence on a plain task with a
foreign series link is also refused, including reopened history; existing live recurring
phone tasks retain their opaque series ids. The [task guide](tasks.md) gives the supported
edit vocabulary.

During the clean fields return, 2Do displayed the advanced date while the server initially
had only its new completed history copy; the live advance arrived with a subsequent phone
note save under the same UID/RUID. A history upload alone therefore did not prove complete
remote advancement in this session. Preserve both versions and check the live resource;
do not repeat completion to repair an apparently stale date.

A pure repeat set/removal/reshape needs the repeat rows of both its source and destination
cases. Interval-only edits still need ordinary edit evidence; mixed saves need both. Newly
verified daily/yearly transitions do not establish changing count/date ends, completion anchors,
weekday sets or Nth rules. Their repeat writer preserves due/start/alerts bytewise, allowing
those already verified field representations to compose with a pure repeat edit; completion
has stricter shape restrictions because it advances dates and creates history.

Task/checklist/project and item creation and the previously verified type conversions remain
as documented in [tasks.md](tasks.md). `tdo op add` takes `--kind`, `--parent` and `--repeat`;
`tdo op edit` takes `--repeat`, `--no-repeat` and `--kind`. The TUI card exposes the same gate.
Known unsafe conversions and the three misread rule classes remain refused.

Three repeat kinds fail on the iPhone and Twydo never writes them in any mode: the last day of
the month, the last weekday and the Nth weekend day. 2Do 3.91 misreads them and its picker cannot
create them. Twydo still reads them, and the refusal says `… repeats are not supported by 2Do on
the iPhone`.

## How far this has been tested

One user's data, one iPhone (2Do 3.91 on iOS 27), one server (Synology Calendar), vdirsyncer
0.21.0 on Linux. Each verified row was tested both ways: the iPhone's change read by Twydo, and
Twydo's change read by the iPhone. The session logs are in
[reference/phase0/](reference/phase0/README.md), with synthetic test data in
[`fixtures/phase0/`](../fixtures/phase0/).

Verifying more needs a 2Do device and a test account. The method is in the
[phase-0 checklist](reference/phase0/README.md); `tools/seed_fixture.py` sets up test tasks on
the server so they do not have to be typed on the phone.

## Item reordering

`J`/`K` reordering works in recurring and non-recurring checklists/projects, including
dated families, alarms and completed/cancelled siblings. It changes only `DisplayOrder`
and normal modification timestamps. The gate requires a recognized, non-nested family,
editable JSON metadata, valid identities/links and the verified ordering row for the
parent's kind. Dates, recurrence and completion are preserved rather than edited or run,
so their separate operation verdicts do not apply to a pure reorder.

On the phone, select **Manually** to display Twydo's saved sequence. Status sort did not
show the intended manual sequence; changing to Manually made it appear. No shared sort
setting was observed, and Twydo does not change the phone's sort setting or add a picker.

The direct [phone samples](reference/phase0/issue41-item-reorder.md) used open, undated,
non-recurring parents and verified display plus preservation through unrelated renames.
Applying the same field operation to other recurrence/date/status contexts is supported
by exact field-preservation tests over the existing recurring checklist/project captures,
including queue/undo and an already-supported recurring completion. This is preservation
reasoning, not a new phone round or a new recurrence execution/edit verification.

Anchored `o`/`O` insertion uses the same verified rank operation, and **also must pass the
existing Create gate** for the new item and its family. Thus it retains all existing
creation, inherited-date and recurrence restrictions; ordering approval cannot authorize
an otherwise unverified create. The phone displayed before/after placement and preserved
ranks/identities/links/creation times through Before/After → BEFORE/AFTER renames. It also
added its documented producer/sync/audio initialization properties to the new tasks.

Phone-side reordering is unavailable. For these two Twydo-originated operations only, an
unsupported phone baseline does not prevent verification: the required evidence is that
the phone displays Twydo's order and preserves it through an unrelated edit and sync.
Other operations keep their existing phone-baseline requirements. Further device/server
rounds require fresh approval. No account or global write-mode change is performed.
