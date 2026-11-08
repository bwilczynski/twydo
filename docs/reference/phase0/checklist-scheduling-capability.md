# Recurring-checklist scheduling capability audit

Local audit, 2026-11-08. No new phone or server session was conducted.
The [capability design](../../superpowers/specs/2026-11-07-capability-gating-design.md)
permits representative write evidence plus exact preservation reasoning.

## Due-date contract: enabled by preservation generalization

The direct evidence is [Plan 6's checklist-date round](plan6-baseline.md#checklist-date-change-written-by-twydo-edge-case-checklist-item-dates),
fixtures `plan6-checklist-date-list1/00-setup`, `01-twydo-date` and
`02-phone-date`. Moving the nonrecurring parent's date updates its date-only
`DUE` and matching `DTSTART`. Dated children lose both representations; their
other fields survive. The phone accepts that Twydo write and subsequently
changes the parent's date again without changing the undated items.

The existing writer removes dates from **every dated indexed item**, rather than
checking whether its date equals the old parent's date. The new recurring
capability therefore admits only items whose due equals the parent's due, or
which have no due. It refuses independently dated items before the writer runs.
JSON `StartDate`, `duetime`, start-day delays and alarms are excluded on both
parent and items; `DTSTART` here is a copy of `DUE`, not an independent start.

[Plan 8](plan8-tui-completeness-session.md) captures FCa in
`plan8-tui-a/010-first-phone-actions` with a plain monthly interval-1 parent and
three inherited-date items, including a completed item. The `020` completion
capture removes live item dates, but **also reopens completed items**. It is
only a comparison for the date-cleanup representation, never an expected full
output for a due-only edit. C29 (`plan8-checklist-b`) also provides monthly
parents and inherited/undated item shapes. None of these captures directly
observes a recurring-checklist due-only edit.

Generalization: apply the already verified date writer while leaving RRULE,
all recurrence JSON, identities, links, lifecycle fields, ordering, unknown
properties and other resources unchanged. Parent and dated children may update
modification stamps. Completed items keep STATUS, PERCENT-COMPLETE and COMPLETED;
this edit neither resets items nor creates history. Empty parents change only
one resource; undated items retain their exact bytes. Tests compare each edited
resource's remaining content with its original and preserve unrelated resources
byte for byte. This is preservation evidence, not a claim of another phone round.

The `recurring-checklist-due/edit` row enables open top-level readable-JSON
plain monthly interval-1 due-anchored checklists, with source and resulting
**date-only day 1–28**, no start, time, alarms, end or delay. Members must be
flat same-list plain tasks, open or completed, with matching standard/JSON links.
Cancelled, independently recurring, nested, malformed or independently dated
members remain refused. Summary/description may compose with the due edit;
other changed fields must use their own supported path and incompatible mixed
saves are refused atomically. The existing checklist-item-date edge is still
required when dated items need cleanup. Broad edit and completion rows do not change.

## Interval contract: candidate only

The `recurring-checklist-interval/edit` row remains **unverified in all shipping
phone columns**, and refuses in both verified and test modes. No checklist
interval-1 to interval-2 write and subsequent phone edit appears in these captures.
The observed monthly-to-weekly transition is an independently supported full
repeat change; it does not prove a monthly interval-2 edit.

An explicit passing candidate matrix admits only monthly interval 1 → 2 with
unchanged due, anchor, selectors and ending, under the same parent/member
boundary. CLI `RepeatInterval(2)` and the card's full monthly `Repeat` normalize
to the same contract and produce identical bytes: only RRULE, JSON
`RecurrenceValue`, and modification stamps change. Child bytes are untouched
and all are held as dependencies. Existing rule preserve/edit verdicts and
written-interval checks still apply. Resulting interval-2 checklist completion
remains refused by its existing execution gate. Unsupported intervals, anchor,
end, selector and mixed date/interval edits remain refused.

Missing bounded evidence, **not scheduled or authorized**: compare a synthetic
monthly checklist interval 1 → 2 on the phone with the candidate write (including
all recurrence metadata and unchanged inherited child dates), then a subsequent
phone title edit retaining interval 2. This would answer the unresolved checklist
interval write/retention question; no completion experiment is needed to grant
only editing. Until separately approved and observed, the row stays unverified.

## Local safeguards and verification

The real gate handles both CLI and TUI. Gate tests cover field boundaries,
missing rows and candidate matrices. Writer tests assert exact field/lifecycle
preservation, empty and undated families, cancellation, mixed saves and permitted
completion after a due edit. Queue tests cover new, changed and removed members;
TUI stale refusal retains typed title/date. Family membership and unchanged-item
bytes are durable journal dependencies, retained through inverses and checked
before recovery. Interrupted parent/child cleanup rolls forward idempotently;
undo/redo restore exact dates and completed-item state. New members or remote
changes block recovery/inverses before any write.

The shared journal family rescan also protects members added after preparation;
its independent relationship-hardening work must be retained when integrating
this capability with the other capability branches.
