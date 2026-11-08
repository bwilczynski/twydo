# Capability implementation checkpoint — 2026-11-08

All five committed implementation plans have been implemented and integrated on
`feat/completness`, after the planning checkpoint `c7083c5`. The branch and approved
worktrees are retained. Nothing was pushed or merged into the base branch.

## Priority and delivered behavior

| Priority | Capability | Shipping status | Main integration commits |
|---|---|---|---|
| 1 | Exact resource/family deletion | Enabled for recognized standalone resources and flat families; malformed identities/links remain refused | `1cc41ae`, `28cef4a`, `ee6fe86`, `5cd817d` |
| 2 | Top-level task/family list moves | Enabled by relocation effects; scheduling stays intact and mixed saves remain atomic | `30d79bb`, `f56b305` |
| 3 | Recurring checklist scheduling | Narrow monthly date-only due edits enabled; interval 1→2 remains candidate-only | `22dbd1a` |
| 4 | Detached history reopening | Standalone and whole flat history family lifecycle changes enabled; individual history items remain refused | `a1084ed`, `2cbaca3`, `e164ca8` |
| 5 | Item relocation, indent/outdent | Core/store/CLI/TUI implemented; all three dedicated membership rows remain unverified | `cc7c3a7`, `a2dceb9`, `0f00f1b` |

Deletion and moves unlock the most existing workflows without executing or editing
recurrence. Checklist due changes unlock existing date controls. History reopening
has a separate lifecycle contract. New membership changes need evidence before
shipping permission; exposing the controls does not bypass that barrier.

The matrix now contains 126 operation rows: seven newly passing narrow capability
rows and four unverified candidates were added to the original 115. No broad rule
or operation row was promoted. These are operation-row counts, not a new audit of
private task data.

## Review and corrections

The final review was an author self-review. Independent review agents became
unavailable due to the account usage limit; no model/account/billing change was
made to bypass it. A fresh independent reviewer remains advisable before merging.

The review checked each plan's Review focus: exact resource scope, standard/JSON
links and internal aliases, self/foreign series identities, independent changed
fields in mixed saves, strict queued-file checks, both relocation dependencies,
creation collisions, partial recovery, exact inverses, and UI cancellation/context.

Corrections, including `5ff64a8`, were verified with observed failing regressions
followed by passing tests:

- JSON-only or internal-alias links cannot hide members from admission, commit,
  queue drain, recovery or undo/redo. Alias guards remain useful after deletion.
- New members and duplicate identity owners in another list cannot bypass a
  family guard. Recorded source/destination paths permit legitimate retained
  internal identities during partial moves.
- Duplicate metadata, colliding internal identities and duplicate history
  lifecycle properties refuse instead of reporting misleading permissions or
  producing inconsistent output.
- Raw checklist date/start/alarm ambiguity refuses before date cleanup.
- Candidate relocation refuses a hidden descendant of an apparent leaf.
- Cancelling checklist inbound date loss closes the picker and preserves selection;
  a snapshot reload invalidates the outstanding confirmation.

Synthetic tests also cover foreign RUID, a distinct internal parent identity,
appended ranks, destination path/UID collisions, unchanged parents/siblings,
queued relocation, partial create/remove recovery and exact undo/redo.

No Critical or Important finding remains from this review. No minor finding was
deferred. Successful cross-list navigation retains existing move behavior; this
increment does not add automatic destination-view navigation or list creation.

## PR #62 review follow-up

A subsequent review reproduced two gaps with synthetic tests. Both were fixed:

- An inverse that recreates a UID already present under another filename now
  refuses before journal preparation. Direct and queued undo preserve all files,
  leave no unfinished entry, and allow an unrelated edit afterwards.
- Candidate reparenting retains both old and new leaf paths in its identity
  guards. A distinct internal JSON identity no longer conflicts with its own
  source or destination during commit, queue drain, partial recovery or undo/redo.
  The shipping membership rows remain unverified.

Both permanent regressions were observed failing before the production changes
and passing afterwards. The architecture documentation records these guarantees.

## Validation

All checks exited 0 on the integrated implementation:

- `cargo test --offline --workspace`: **1,136 passed**, 46 targets including doc
  tests, zero failed/ignored.
- `python3 -m unittest discover -s tools/tests -t .`: **49 passed**.
- `python3 tools/check_links.py`.
- `cargo clippy --offline --workspace --all-targets -- -D warnings`.
- `cargo fmt --all --check`.
- `git diff --check`.

Builds used the existing disk-backed worktree target directories, keeping generated
artifacts ignored and out of /tmp. CLI smoke/permission tests ran against temporary
synthetic vdirs with isolated configuration/state and no sync configuration.
The earlier orchestration sample returned the expected package before implementation;
subsequent work followed the user's instruction to continue without OpenCode.

## Decisions recorded during execution

- The user's direct-execution instruction superseded the older executor/model
  constraint. Existing design, worktree and commit authorization was retained;
  no repeated approval was needed. Cost if misunderstood: a different executor
  performs the same authorized local work.
- Integration continued in the existing isolated feature worktree, retaining the
  per-plan worktrees for recovery. Cost: those snapshots are not the final
  combined branch; use the integration commits above when reviewing delivery.
- Additive alias/multi-family guards refuse ambiguous links instead of normalizing
  them or guessing membership. Cost: a valid but unobserved encoding can be refused;
  existing old journal/queue JSON remains readable.
- The unrelated-file regression fixture was corrected in both standard and JSON
  parent representations. Cost: it no longer treats an inconsistent JSON link as
  unrelated; that inconsistent input is intentionally tested as a refusal instead.

## Remaining evidence boundary

Checklist interval changes and single-item membership changes remain candidate-only.
The [checklist evidence](checklist-scheduling-capability.md) and
[item relocation proposal](item-relocation-capability.md) identify the missing
mechanisms and minimum future cases. They require fresh approval before any device
round. Individual history-item reopening also remains refused for lack of its
own evidence. No production task data, NAS, phone, account, global permissions or
write-mode configuration was changed, and no synthetic device lists were deleted.
