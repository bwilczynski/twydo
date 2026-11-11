# Operation contracts and focused evidence

Date: 2026-11-11 · Status: approved (2026-11-11)

## Intent and decisions

Make compatibility evidence small enough to understand and maintain. Organize it by
the operation it proves, replace overlapping compatibility cases with operation
contracts, and remove historical material that adds no distinct proof.

The user chose an operation-contract rebuild over consolidating within the current
matrix model, and explicitly requested a less-is-more approach covering both
fixtures and the matrix. Use the existing `refactor-reorganize-evidence` worktree.

This is a project-sized refactor. The user approved this design on 2026-11-11.
It supersedes the decision in the [incremental capability design](2026-11-07-capability-gating-design.md)
to retain the old matrix and gate paths during migration. Its preservation,
proportional-verification and safety policies still apply, as do the
[main spec](2026-10-31-terminal-tasks-design.md) contracts.

## Current problems

The phase-0 corpus has 25 session groups, 226 step files and 2,402 `.ics` files,
with 830 distinct byte sequences. These are inventory observations, not deletion
targets: different states and contexts can matter even when bytes repeat.

The matrix has 130 operation rows, 15 rule rows and 14 edge rows. Ordinary actions
can require overlapping own-task, family, date and rule verdicts. Newer text,
move, delete and history capabilities bypass parts of that scheme. Older/custom
matrices take fallback paths. Evidence arrays frequently point at whole sessions
instead of identifying the resources and transitions establishing a claim.

Tests across core, daemon and CLI load session directories directly. Some use
captures as convenient setup rather than as interoperability or format evidence.
Reorganizing files alone would preserve those maintenance problems.

## Approaches

1. Curate operation folders and reduce rows inside the existing rule/operation/edge
   model. Less gate migration, but overlapping permission concepts survive.
2. Replace that model with operation contracts and focused evidence cases. More
   migration work, but one vocabulary joins evidence, gate decisions and reporting.

The user selected approach 2 on 2026-11-11.

## Scope and limits

Rebuild compatibility declarations, gate selection, fixture consumers, capture
promotion and their documentation together. Preserve effective admission in both
`verified` and `test` mode, including known failures and narrow shape boundaries.
Reducing rows must not silently broaden or restrict permissions. If an existing
decision cannot be represented faithfully, retain a specific contract variant;
changing that behavior requires a separately identified decision.

No new task features, production access, server/phone actions or device rounds.
Do not remove synthetic lists from the test account. Existing captured evidence
is sufficient input for this refactor. Do not build an evidence database, custom
query language, content-addressed store or permanent migration framework.

## Operation vocabulary and ownership

Use stable operation IDs shared by compatibility declarations, evidence folders
and permission reporting:

`create`, `edit-title`, `edit-notes`, `set-due`, `set-start`, `set-repeat`,
`convert`, `complete`, `reopen`, `move`, `delete`, `reorder`, `insert`, `reparent`,
`list-create`, `list-metadata`, `list-delete`.

These cover implemented and candidate writes; candidate operations stay refused.
Future operations get an ID when there is a concrete implementation or unresolved
compatibility claim, rather than generating speculative empty combinations.
The names are internal stable identifiers, not new CLI subcommands.

Interval editing belongs to `set-repeat`, with a distinct interval-only contract
where its prerequisites or evidence differ from setting/removing a rule. Dates,
conversion and repeat changes retain distinctions currently hidden under `edit`.
Existing wire operations and `Change` variants need no public protocol redesign.

One concrete write may require several contracts. `CreateAt` needs creation and
insertion. A mixed edit needs each changed field's contract, with interaction
checks on the combined result. A save with relocation checks the post-edit
structure. Splitting an edit must not bypass an existing mixed-edit restriction.

## One compatibility definition

Keep `compat/matrix.toml` as the single authored compatibility definition, with
`matrix_version = 2`. Replace independent rule and edge authorization tables
with operation-scoped contract entries. Each entry has:

- A unique stable contract ID and operation ID.
- A finite, typed variant identifying a structural/semantic contract implemented
  in Rust; no arbitrary predicate expressions in TOML.
- The required device-observation verdicts, evidence case IDs, and concise scope
  or failure reason. Preserve pass/fail/unsupported/unverified distinctions.
- Explicit evidence for any generalization through preservation, rather than
  presenting all supported shapes as directly observed on a device.

Structural prerequisites, write sets and invariants belong to the Rust contract
implementation and its tests. TOML supplies evidence verdicts for those contracts;
it does not become another implementation of the gate. Missing contracts refuse.
Unknown IDs/variants, duplicate entries and invalid versions are errors.

Use explicit variants for behavior that differs, such as standalone deletion,
family deletion, item deletion, ordinary completion, recurring-task advancement,
checklist reset, project completion/history, ended-series reopening and history
reopening. Typed recurrence profiles under `complete` and `set-repeat` retain
differences in classes, endings, anchors, dates and alarms that those writes
exercise. Shared recurrence calculations remain ordinary Rust helpers.

Do not recreate a cross-product of every operation, task type, recurrence, date
and status. Text edits need representative text/format observations and exact
preservation tests; they do not need recurrence execution verdicts. Relocation
needs identity/link transformations and preservation. Completion needs targeted
advancement, reset and history proof. Failures are attached to operations that
exercise them: a misread repeat encoding blocks writing that encoding without
blocking a byte-preserving title edit.

Preserve current write-mode requirements. `test` still requires the applicable
baseline, while relaxing only the directions/conditions it relaxes today. Keep
the existing bounded Twydo-originated ordering exception explicit. Remove unused
Mac direction columns; record the actual observed client/version in case metadata.
An untested client contributes no permission or speculative matrix rows.

Version 1 custom matrices are rejected with a clear version/migration error after
the cutover. Update all repository examples, parser tests and hooks together.
There is no silent conversion and no permanent legacy gate branch. This is a
proposed compatibility break for custom matrix authors, not a change to stored
task data, daemon RPC or write modes.

The embedded matrix is validated through the same parsing rules as explicit
matrices. Runtime admission uses the embedded declarations without opening fixture
files. Repository validation checks the evidence references separately.

## Focused cases under operation folders

The target layout is `fixtures/<operation>/<case>/`. Remove `phase0/v1`, plan
numbers, issue numbers and collection nicknames from canonical evidence paths.
Keep hand-authored parser/unit-test fixtures beside their tests when appropriate;
they are not device evidence and need no operation folder.

For example, `fixtures/complete/monthly-day-overflow/` contains `case.toml` and
named states such as `before/`, `phone-after/` and `twydo-after/`. Each state is
a complete small vdir, including collection directories where needed. Files keep
their captured names, bytes, UIDs and links. Different twins may have different
identities; comparison rules must say exactly which differences are expected.

Each `case.toml` records:

- Stable case ID (the operation-relative path), one precise claim and limitations.
- Origin: device capture, local synthetic test, or local/server protocol observation.
- Original capture provenance (old path and source revision for migrated material),
  actual environment versions when known, action and recorded observation.
- Named state paths with explicit roles and the complete resource membership of
  each retained state. Only applicable states are required.
- Assertions/regression test references protecting the claim and preservation.

Retain additional states only for meaningful transitions, such as successive
month-end completions, a phone edit after Twydo's write, or deletion/last-item
behavior. Empty collection states must be representable explicitly. A visual
observation or server-atomicity failure can have a concise recorded result without
inventing an `.ics` difference or requiring a fake phone-baseline state.

No step numbering, implicit previous-step lookup, base-step chains, stored
`changes.diff`, copied collection metadata unrelated to the claim, or duplicated
verdicts in case files. A before/after diff is derived on demand. Keep a complete
family when interpreting an operation depends on its relationships; keep unrelated
bystanders only when an assertion needs them.

Cases have one owning operation. Other contracts/tests may reference them by ID
and named state rather than copy captures or create a generic shared dump. Small
duplication across independently meaningful cases is preferable to a storage
deduplication framework. A case can support multiple contracts; document which
part of its observation supports each claim.

## Evidence pruning and historical documents

Retain a case only if it protects an active contract, explains a current refusal
or known failure, or proves a distinct parser/model behavior worth keeping in a
focused test. Test usage alone does not establish value: incidental setup should
be replaced with small local synthetic builders.

For each current claim, choose the strongest minimal existing capture(s). Remove
repeated observations of the same behavior, unrelated snapshot contents and test
combinations whose only distinction is a field the operation preserves. Keep
independent failure mechanisms: recurrence advancement, endings, anchors, offsets,
resets, history, last-item deletion, identity/link rewrites and atomicity failures.
An unused known-failure case can still justify a refusal and earns its place.

Local tests may generalize supported shapes by proving exact preservation. They
cannot establish a new device observation. Never rewrite captured resources to
make a case more generic and continue calling them device evidence. Report
unavailable provenance as unknown; do not infer versions or outcomes as facts.

Replace `docs/reference/phase0/` historical session narratives, completed checklists
and capability audits with concise operation evidence records and current
compatibility documentation. Move still-actionable unresolved work or consent
boundaries into one short `docs/reference/evidence.md` guide before deleting its
source. Keep stable technical 2Do format references. Update project handoff,
as-is docs and links in retained specs/plans; do not leave historical files just
to satisfy an inbound link. Keep specs/plans as design history rather than expand
this into deleting all project history. Git retains removed session material.

## Gathering and promoting new evidence

Keep raw capture output and session scratch outside git. Adapt the capture helper
to select an operation/case and explicit state, plus explicit collections/resources,
instead of discovering a previous lexical step or committing an entire session.
Support selecting whole families and multiple collections for move cases.
The helper reads source data and writes only the chosen staging destination.

Promotion means selecting resources, recording the claim and provenance, attaching
the exact test/contract, privacy scanning, and obtaining the existing human
confirmation that staged content is synthetic. A prospective device round still
requires a bounded unresolved question and fresh approval. No additional device
work is necessary merely to fit an existing observation into this format.

Add a small repository validator to the normal checks: unique/recognized contract
and case IDs, valid references/state membership, no dangling evidence, no inherited
snapshot chains, required provenance fields, and no unowned case/resources. Check
test references for existence without inventing a new test framework. Device-backed
passing contracts need actual observation references; synthetic-only assertions
cannot satisfy that requirement. Privacy scanning covers the new metadata format.

## Migration and verification

The user additionally requires a breaking release visible in `tdo --version`.
The current workspace version is `0.1.9`; target `0.2.0`, with the workspace
packages and lockfile in agreement. Use a release-triggering conventional commit
such as `feat(compat)!: replace session evidence with operation contracts`, with
a `BREAKING CHANGE:` explanation of the v2 matrix requirement. Document custom
matrix migration in the changelog and compatibility guide. Verify the release-plz
result on a disposable local copy so this is a minor bump in the `0.x` series,
not an ordinary patch release or an accidental second minor increment.
Publishing, tagging and production upgrades are outside this implementation.

Before deleting anything, make a temporary claim-to-test-to-capture inventory and
map old gate decisions to proposed contracts. Use that inventory to identify
retained claims, redundant inputs and refusals, not as a permanent second registry.

Capture current decisions with concrete operations over representative existing
states and negative neighbors, in both write modes. Preserve narrow completion,
checklist schedule, conversion, item/parent, ordering, list atomicity and unsupported
repeat boundaries. Compare the rebuilt gate against those baseline decisions;
do not use a row-count assertion as compatibility proof. Cases needing input use
explicit proposals for this comparison. Cover mixed edits and post-edit moves.

Migrate by operation with tests passing at each reviewable step. Temporary adapters
are allowed during the refactor; remove them at completion. Rewrite fixture consumers
at their proper layer, preserve byte-exact writer/undo/queue/recovery assertions,
and replace incidental capture setup with synthetic builders. Remove obsolete
row-count and corpus-size tests; validate claims and actual behaviors instead.

Then remove old corpus/session documents, obsolete matrix sections, fallback paths
and capture formats. Keep permission reports on the real shared gate, including
privacy defaults, no invented input values and allowed/refused/inapplicable/needs-input
states. Recurrence evidence, if still shown, is a projection of operation evidence,
not a separately maintained permission source.

Run focused contract/case tests as migrated, then `mise run test`, `mise run lint`,
the new evidence validator and privacy scan. Confirm no personal data was introduced,
all retained links resolve, and the compiled binary needs no fixture checkout.
Report before/after corpus and contract counts as outcomes, with no arbitrary quota.

## Approved implementation boundaries

The approved design includes the operation IDs, single v2 TOML definition, explicit
rejection of v1 custom matrices, complete named states, one owning operation per
case and behavior-preserving cutover. The breaking-release requirement was added
explicitly with approval; the `0.2.0` target follows the current `0.1.9` baseline.

Leave module layout, exact TOML spelling, test helper APIs, migration step sizes and
the final minimal set of cases to the implementation plan/audit, within the rules above.
