# Title and notes capability

Local audit, 2026-11-08. No phone, server or production write was performed.
This implements the text-edit increment of the
[approved capability design](../../superpowers/specs/2026-11-07-capability-gating-design.md).

## Evidence and write contract

Direct representative evidence comes from Plan 4's task field edits
(`plan4-list1/02-edit`) and C29's recurring-checklist parent/item text edits
(`plan8-checklist-b/011-phone-items-and-text`). Twydo's corresponding writer
barrier is `021-twydo-c-items-text`; subsequent phone note edits in
`028-phone-writer-note-followup` retain the accepted Twydo-written families.
That phone save also has the documented producer/sync initialization decorations;
these are phone effects, not extra fields Twydo needs to write.

The existing writer changes only top-level `SUMMARY`, `DESCRIPTION`, `DTSTAMP`
and `LAST-MODIFIED`. It validates nonempty titles and escapes text. Clearing notes
empties an existing `DESCRIPTION`, leaving an absent property absent. An alarm's
description is unchanged, even after a task title edit, matching the existing writer.

The `resource-text` / `edit` row generalizes this operation to recognized standalone
tasks and flat checklist/project parents/items, including open, completed and
cancelled resources, detached history and unchanged scheduling contexts. Dates,
recurrence and its metadata, alarms, lifecycle, ranks, identities and links keep
their exact bytes. Other resources are held, never rewritten. This is preservation
generalization, not new device observations for every combination.

## Admission and boundaries

A nonempty title/notes-only edit uses the dedicated row before scheduling checks.
Unknown task kinds, unreadable required metadata, duplicate resource/internal
identities, ambiguous edited properties, overrides, broken/disagreeing or cross-list
links, nested families and plain tasks with children remain refused. Every member's
identity ownership and incoming standard/JSON references are checked, including
hidden descendants.

Unchanged schedules need no recurrence execution/edit verdict. Text edits neither
set nor execute a rule the phone misreads. No broad operation or recurrence verdict
changes. Creation, conversion, scheduling, completion, reopening and relocation
retain their own gates. Mixed saves follow their existing non-text contracts:
supported checklist due/text saves still check text separately; unsupported
date/interval/type changes refuse atomically and the card retains typed input.

Matrices without `resource-text.edit` retain legacy handling. A present row follows
normal mode rules: verified mode requires the phone baseline and both Twydo
directions; test mode still needs the baseline. User configuration is unchanged.

## Durable dependencies and verification

The store holds the flat family's bytes and membership, including member
resource/internal identity aliases, through queueing, recovery, undo and redo.
New owners or incoming references to any held member invalidate the plan.
Remote changes refuse under existing strict whole-file checks; recovery remains
idempotent and inverses never overwrite changed family resources.

Existing `gate`, `edit_fields`, `store` and `store_queue` targets cover independent
evidence, lifecycle/scheduling preservation, malformed neighbors, exact serialized
bytes (including metadata and alarm descriptions), stale family dependencies,
new identity owners, recovery and inverses. CLI and real card tests cover completed
recurring-checklist text saves, blank titles, report agreement and atomic mixed-save
refusal. All fixtures and mutations are synthetic.
