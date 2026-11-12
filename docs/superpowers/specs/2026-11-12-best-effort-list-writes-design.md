# Best-effort list rename, recolor and deletion

Date: 2026-11-12 · Status: draft for user review.

## Intent and decisions

[Issue 69](https://github.com/bwilczynski/twydo/issues/69) shipped list creation and local
archiving. Rename/recolor and deletion were built behind gates that stay closed because the
server enforces no protection against a concurrent remote change: the approved
[lifecycle round](../../reference/evidence.md) showed a lock-free PROPPATCH succeeding while a
lock was held and a stale `If-Match` DELETE removing the collection
(`list-metadata.collection` and `list-delete.collection`, `atomicity = "fail"`).

Decisions made by the user on 2026-11-12 (fixed):

- **Accept best-effort list writes.** The user is the only person editing these lists, so a
  phone change landing in the gap between Twydo's last check and its write is unlikely. A
  change that lands in that gap may be overwritten (rename/recolor) or lost (deletion). This
  is an explicit user decision that relaxes the issue-69 design's requirement for a
  server-enforced precondition; it supersedes that design's "Open decisions" on this point.
- **On for everyone**, no config opt-in. The user is the only user of released builds.
- **Rename/recolor goes live without a phone round.** The user will watch the phone the first
  times it is used. No device/NAS session is part of this work.
- **Deletion safety net is the existing pre-delete backup plus export.** Restoring a deleted
  list from inside Twydo stays in [#75](https://github.com/bwilczynski/twydo/issues/75).
- [#84](https://github.com/bwilczynski/twydo/issues/84) (refusal message always blames
  atomicity) is folded in, since the same function changes.

Closes [#73](https://github.com/bwilczynski/twydo/issues/73) and
[#74](https://github.com/bwilczynski/twydo/issues/74) by policy rather than by proof; their
comments should say so.

## Goals

1. Sidebar `i` → **Save to CalDAV** renames/recolors a list in both write modes.
2. Sidebar `dd` (and the card's delete action) deletes the list from CalDAV and all devices
   after the existing backup and confirmation, then retires the local copy so sync does not
   bring it back.
3. The compatibility record stays truthful: atomicity remains `fail`; the accepted risk is
   recorded separately and visibly.

## Non-goals

- Restore/import of a deleted list, or resolving vanished pending targets (#75).
- Shared archive state (archive stays local, unchanged).
- Any new phone/server evidence round, or changes to task-write gates.
- Native CalDAV (#86) or Twydo-owned connection config (#88).

## Anti-goals (failure even if it "works")

- Pretending protection exists: no `atomicity = "pass"`, no docs claiming conflict-safe edits.
- Relaxing any other contract, or making a plain `atomicity = "fail"` (as in custom matrices
  without the acceptance) suddenly permissive.
- Deleting without a persisted, verified backup, or after the remote list visibly changed
  since the confirmation.
- Repeating a create/delete blindly after an uncertain outcome, or letting ordinary sync
  recreate a deleted list.

## Compatibility gate

Add an explicit acceptance to the two contracts, e.g. `accepted_risk = "best-effort"` (field
name is the builder's call), allowed only on `list-metadata.collection` and
`list-delete.collection`; the matrix parser rejects it elsewhere, as it does `atomicity`.

`Matrix::require` for these two contracts with the acceptance:

- atomicity `fail` or `unverified` no longer refuses; `unsupported` still does;
- direction and baseline verdicts refuse only on `fail`/`unsupported`; `unverified` passes in
  both modes (this is what "go live without a phone round" means);
- without the acceptance, behavior is exactly today's (including in test mode).

Matrix notes for both rows say plainly: best-effort, accepted by the user on 2026-11-12, server
does not enforce concurrent-edit protection. The delete row may cite the lifecycle round's
observed phone disappearance as `twydo_to_iphone` evidence if the evidence validator accepts
it; otherwise leave it unverified.

#84: `capability` reports the `RequirementFailure` text; it adds the concurrent-edit
explanation only when the failing direction is `atomicity`. Unknown cases name the case.

## Rename/recolor

The existing helper path is unchanged: held-value check against the card's `expected`, field-
only PROPPATCH, per-property 207 verification and readback, durable operation record and
reconciliation. Only the gate opens. The card's **Save to CalDAV** explanation adds that a
simultaneous phone edit can be overwritten.

## Deletion

Current state: `preview_delete` takes a full remote+local backup under the operation locks and
then releases its operation; `delete_list` always refuses.

New flow:

1. **Preview (exists).** Backup persisted and validated before the confirmation is shown.
   Confirmation names the list, task/resource counts, all-device scope; Cancel is default.
2. **Delete request** carries the backup id. It starts a new operation (`action = "delete"`)
   with the usual preflight: no active list operation, task journal recovered, no pending
   queue entries, journal entries or conflicts for the target.
3. **Revalidate** under the helper's locks: fresh remote inventory (hrefs, ETags, content
   hashes) and collection properties, plus local bytes, must equal the backup. Any difference
   refuses with "the list changed since the backup; review again" and the client re-runs the
   preview (new backup, new confirmation). Incomplete/ambiguous inventory refuses.
4. **DELETE** the exact resolved collection URL (no redirects, same target rules as other
   helper actions). Record `RemoteStarted` before sending. No `If-Match`: it was shown not to
   protect anything, and sending it would imply otherwise.
5. **Outcome.** 2xx → `RemoteConfirmed`. Timeout/interruption → `Uncertain`. Reconciliation
   reads the exact URL: absent → confirmed; present and still equal to the backup → the user
   may retry the delete; present and changed → refuse, require a new preview. Distinguish an
   absent target from an unreachable server.
6. **Local retirement** after confirmed absence: move the local collection out of the vdir into
   private state (reusing the `forget` retirement machinery where its contract fits), refresh
   the synced list set so vdirsyncer neither recreates the remote collection nor resyncs the
   local one, then `Complete`. Interrupted retirement resumes before ordinary task sync, as
   today's operation fence already enforces.

The backup stays in `lists/backups/` and remains exportable through the existing backup
browser. No automatic cleanup.

## Docs and records

Update in the same changes: `docs/tui.md` (list card, `dd`), `docs/compatibility.md`,
`docs/sync.md` if retirement interacts with discovery, the two matrix rows, and a short note
in the issue-69 design and plan pointing here. Main spec §11 says the compatibility gate
cannot be overridden by the builder; this relaxation is the user's decision and should be
recorded as such.

## Verification

Offline, using the existing synthetic transport/adapter fixtures; no NAS or phone access:

- gate: acceptance opens only the two contracts; `fail`/`unsupported` still refuse; matrices
  without the acceptance behave as today in both modes; #84 messages name the real direction;
- rename: existing prototype tests now run against the embedded matrix;
- delete: zero DELETEs before a verified backup; changed inventory/properties/local bytes
  refuse; queue/conflict/journal refusal; DELETE success → retirement → sync does not
  recreate; kill/timeout at each phase reconciles without a blind repeat; absent vs
  unreachable; real TUI `dd` confirm/cancel/re-preview.

Required checks: workspace tests, Python tool tests, clippy (`-D warnings`), fmt, docs links,
evidence validator, `git diff --check`, privacy review of new fixtures.

## Left to the builder

Field names and schema for the acceptance, operation/protocol shapes for the delete request,
how much of `retired::forget` is reused versus mirrored, message wording, and how the TUI
routes a "changed since backup" refusal back into a fresh preview.
