# Operation Evidence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Replace session-based evidence and overlapping matrix gates with focused operation contracts, shipped as breaking version `0.2.0`.

**Architecture:** One version-2 `compat/matrix.toml` supplies observed verdicts to typed Rust operation contracts. Complete minimal cases live under `fixtures/<operation>/<case>/`; repository tools validate references and provenance, while runtime admission needs no fixture checkout. Preserve current permissions in both write modes.

**Tech Stack:** Existing Rust workspace, serde/TOML, Python standard library, mise and release-plz. No new dependencies expected.

**Spec:** [Operation contracts and focused evidence](../specs/2026-11-11-operation-evidence-design.md).

## Global Constraints

- Work in the existing `refactor-reorganize-evidence` worktree; no production/server/phone access or new device rounds.
- Preserve effective admission in both `verified` and `test` mode, including known failures and narrow shape boundaries.
- `matrix_version = 2`; reject version 1 custom matrices explicitly. No permanent legacy gate branch.
- Preserve pass/fail/unsupported/unverified distinctions and the bounded Twydo-originated ordering exception.
- Runtime admission uses embedded declarations without opening fixture files.
- Do not rewrite captured resources and continue calling them device evidence. Unknown provenance stays unknown.
- No personal data, agent attribution, speculative combinations, evidence database or content-addressed store.
- Target one workspace version `0.2.0`; use a release-triggering breaking conventional commit. No publishing/tagging in this plan.

## Review Focus

1. Splitting mixed edits accidentally admits unsupported combinations: compare concrete combined requests, including post-edit relocation.
2. Version-1 files silently parse as an empty permissive matrix: reject their version before interpreting v2 fields.
3. Extracting a family or move case drops a dependency/deletion: validate full membership, absent resources and explicit empty collections.
4. Test mode or ordering exceptions accidentally become general bypasses: pin per-direction requirements and structural refusals.
5. A generic local fixture is mislabeled as phone proof: validate evidence origin and require observation-backed claims for passing device requirements.

---

## File and interface map

- `crates/twydo-core/src/compat.rs`: v2 parsing, typed operation/contract IDs, verdict requirements and embedded validation.
- `crates/twydo-core/src/gate.rs`, `gate/*.rs`: contract selection, structural checks, mixed-save composition; reuse current relationship checks.
- `crates/twydo-core/src/store/{mod,family}.rs`, `lists/remote.rs`: replace direct legacy matrix queries without weakening store safeguards.
- `crates/twydo-daemon/src/server/{permissions,reports}.rs`: operation vocabulary and reports from the same gate; recurrence reporting is derived.
- `fixtures/<operation>/<case>/case.toml`: observation/provenance and complete named vdir states; no verdicts.
- `tools/evidence.py`: shared manifest loading and validation; `tools/check_evidence.py`: CLI; `tools/capture_fixture.py`: explicit staged capture and privacy scan.
- `crates/twydo-core/tests/common/{mod,recurrence}.rs` and cross-crate test helpers: case/state loading and local synthetic builders.
- `docs/reference/evidence.md`: concise maintenance, unresolved questions and consent boundaries; update compatibility/testing/handoff documents and retained historical links.
- `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, release metadata as needed: breaking `0.2.0` release.

Use these Rust interfaces (additional private helpers remain implementer choices):

- `OperationId`: serde kebab-case enum for the exact 17 operation IDs in the spec; `as_str(self) -> &'static str`.
- `ContractId`: finite serde enum covering the audited variants from Task 1, with `operation(self) -> OperationId`.
- `Matrix::from_toml_str(&str) -> Result<Matrix, MatrixError>`, `Matrix::load(&Path) -> Result<Matrix, ConfigError>` and `Matrix::embedded() -> &'static Matrix` keep their external entry points.
- `Matrix::require(&self, id: ContractId, mode: WriteMode) -> Result<(), RequirementFailure>` evaluates the contract's direction requirements; structured failure retains missing direction and verdict.
- `gate::Checker::check(&self, op: &Op) -> Result<(), Blocked>` stays the shared admission entry point.
- Test helper `case_state(case: &str, state: &str) -> PathBuf` resolves one complete named state; state membership has no inherited base.
- Python `load_case(root: Path, case_id: str) -> dict`, `validate(root: Path) -> list[str]`; CLI exits 0 only with no errors.

### Task 1: Inventory claims and pin current admission

**Files:** create temporary `/tmp/twydo-evidence-inventory.md`; create `crates/twydo-core/tests/gate_contract_parity.rs`; extend existing gate/test helpers only as needed.

**Interfaces:** produces the finite contract catalog and retained-claim map consumed by Tasks 2–6, and checked-in concrete request expectations independent of old matrix rows.

- [x] Inventory every matrix claim, direct consumer and captured state, including store/list queries and indirect common helpers. Identify the strongest retained proof, incidental setup, refusal evidence and redundant history for each.
- [x] Record the proposed contract IDs, structural boundaries, direction requirements and operation mapping in the temporary inventory. Do not generate every task-shape combination.
- [x] Write `current_contract_decisions_are_preserved_in_both_modes`: evaluate explicit proposals with the current embedded matrix and pin allow/refuse plus meaningful refusal category. Cover all selected contract boundaries, known failures, positive cases and nearest negative neighbors.
- [x] Include named cases `mixed_checklist_due_and_interval_remains_refused`, `save_move_checks_post_edit_structure`, `ordering_exception_does_not_relax_other_writes`, `list_atomicity_failures_remain_refused`, and `recurring_end_history_and_alarm_boundaries_remain_narrow`.
- [x] Run `cargo test -p twydo-core --test gate_contract_parity`; confirm expected behavior with the old implementation before replacing it. Keep the concrete baseline expectations through cutover; do not retain a second old gate implementation.
- [x] Review the inventory against the approved spec; defer any permission change instead of silently adjusting expected outcomes. Record progress in this plan.

### Task 2: Establish the case format, validator and first focused evidence

**Files:** create `tools/evidence.py`, `tools/check_evidence.py`, `tools/tests/test_evidence.py`; modify `tools/capture_fixture.py`, `tools/tests/test_capture_fixture.py`; add the initial focused cases selected in Task 1.

**Interfaces:** produces `load_case`/`validate` above and a case TOML schema with required `claim`, `origin`, `provenance`, `states`, `tests` fields. State paths are relative to the case and contain complete collection directories; metadata specifies members, including empty collections.

- [x] Write failing validator tests: `test_rejects_duplicate_or_escaping_case_paths`, `test_rejects_missing_or_unowned_state_resources`, `test_empty_collection_state_is_valid`, `test_rejects_inherited_states_and_verdicts`, `test_unknown_provenance_is_explicit`, `test_test_reference_must_exist`.
- [x] Run `python3 -m unittest tools.tests.test_evidence`; confirm failures expose the missing implementation.
- [x] Implement manifest loading and filesystem/reference validation with the standard library. Recognize device, synthetic and protocol origins; permit result-only observations when no file difference exists. Reject absolute/escaping paths and symlink traversal outside the case.
- [x] Curate a small first slice: title/notes, standalone/family move, item deletion and a known repeat failure, selecting exact resources using Task 1. Record source revision, original path, versions when actually known and the observations supporting each claim. Keep all captured bytes unchanged.
- [x] Extend the existing privacy scanner for `case.toml`; keep its existing sensitive-content checks. Add tests for the new metadata and forbidden personal fields.
- [x] Run the Python tests and `python3 tools/check_evidence.py`; confirm the curated cases validate without requiring the rest of the old corpus to be migrated yet.

### Task 3: Implement v2 contracts and migrate preservation operations

**Files:** modify `crates/twydo-core/src/compat.rs`, `gate.rs`, `gate/{text_edit,relocate,delete,relationships}.rs`, `store/{mod,family}.rs`, `compat/matrix.toml`; update gate/compat/store tests and common matrix builders.

**Interfaces:** produces `OperationId`, `ContractId`, `Matrix::require`, the v2 compatibility records and operation contract selection. Temporary migration adapters are internal and removed in Task 6.

- [x] Write failing parser tests `rejects_v1_with_migration_error`, `rejects_unknown_and_duplicate_contracts`, `embedded_uses_the_same_validation`, plus requirement tests for every verdict, missing contracts, both modes and the ordering-origin exception.
- [x] Run the focused compat tests; implement strict version-first decoding and the v2 records. Keep one authored matrix and explicit direction requirements; remove Mac direction fields from the new schema.
- [x] Migrate title, notes, move and delete to typed contract selection using current structure/identity checks. Replace direct store matrix queries with the same contract APIs; preserve exact queued-family dependencies and byte/stale checks.
- [x] Link v2 records to case IDs and add validator tests `test_synthetic_only_cannot_prove_device_requirement`, `test_contract_evidence_must_resolve`, `test_contract_operation_matches_case_or_explicit_cross_reference`.
- [x] Run `cargo test -p twydo-core --test gate_contract_parity`, focused gate/store family/queue tests and Python evidence tests. Require unchanged concrete decisions and retained byte-preservation assertions.

### Task 4: Migrate scheduling, lifecycle, structure and collection contracts

**Files:** modify `gate.rs`, `gate/{checklist_edit,history_reopen,reparent}.rs`, `compat.rs`, `compat/matrix.toml`, `lists/remote.rs`; curate remaining required operation cases and update core edit/gate/store/list tests.

**Interfaces:** completes contract selection for all 17 operation IDs. The old rule/edge tables cease authorizing writes; recurrence profiles belong to concrete scheduling operations.

- [x] Extend focused tests before each operation migration using Task 1's baseline, including date-only/timed, interval-only versus reshape, from-completion, end/count, history, child-reset, conversion and last-item distinctions.
- [x] Migrate `set-due`, `set-start`, `set-repeat`, `complete`, `reopen`, `convert`, `create`, `insert`, `reorder`, `reparent` and list operations in that order; retain narrow variants where parity requires them. Preserve list metadata/deletion atomicity refusals as operation-local failure requirements.
- [x] Compose mixed writes against the whole proposed result, including creation plus placement and edit plus move. Keep explicit existing mixed-edit restrictions rather than treating individually passing fields as sufficient.
- [x] Extract targeted state sequences for distinct recurrence/reset/history behavior; drop unrelated captured tasks. Use local builders for broad preservation combinations and retain real byte captures for encoding/observed transitions.
- [x] Run parity plus `cargo test -p twydo-core` and the evidence validator. Investigate any permission difference; do not change the baseline merely to make it pass.

### Task 5: Switch reports, remaining consumers and capture workflow

**Files:** modify `crates/twydo-daemon/src/server/{permissions,reports}.rs`, daemon/CLI/TUI tests loading old captures, common fixture helpers, `tools/capture_fixture.py`, its tests and `mise.toml`.

**Interfaces:** completes `case_state`; reports use stable operation IDs, real gate probes and derived recurrence evidence. Capture selects an operation/case, named state and explicit collection/resource set.

- [x] Add report tests for stable operation names, shared gate results, missing proposal values, empty input, completed/history tasks, privacy defaults, pending queue overlay and explicit v1 matrix errors.
- [x] Change reporting without altering daemon RPC shape. Remove the separately maintained recurrence verdict source; if recurrence evidence remains visible, derive it from applicable complete/set-repeat evidence and state its scope.
- [x] Replace session-path loaders throughout core, daemon and CLI/TUI with `case_state`; retain parser/model corpus tests only for distinct claims, not file-count floors. Replace incidental capture inputs with small synthetic builders at the lowest owning layer.
- [x] Write capture tests `test_capture_requires_explicit_state_and_selection`, `test_capture_multiple_collections_and_empty_destination`, `test_capture_does_not_read_previous_steps`, `test_capture_preserves_selected_bytes_and_source`.
- [x] Implement explicit staged capture; remove automatic lexical previous-step/diff behavior once its consumers are migrated. Retain privacy scanning and human confirmation for new promotion.
- [x] Add `python3 tools/check_evidence.py` to `mise run test`. Run Python tests, daemon report tests and affected client suites; confirm runtime tests work with only temporary state and their own sockets/configuration.

### Task 6: Remove obsolete corpus, legacy gates and session documents

**Files:** remove `fixtures/phase0/` and replaced `docs/reference/phase0/` files; create `docs/reference/evidence.md`; update `AGENTS.md`, `docs/{README,testing,compatibility}.md`, `docs/reference/README.md`, retained specs/plans and source comments.

**Interfaces:** leaves one compatibility schema, one contract evaluator and one case format; the evidence guide owns remaining unresolved questions and consent boundaries.

- [x] Account for every inventory claim: retained minimal case, equivalent focused/synthetic test, explicit refusal, or redundant historical material. Resolve unowned new cases instead of keeping a catch-all folder.
- [x] Move current limitations, actionable unresolved work and device-consent boundaries into the evidence guide. Explain case ownership, direct observations versus preservation generalization and the staging/promotion workflow.
- [x] Update retained links and handoff instructions to current claims before deleting obsolete session material. Keep technical 2Do format references and design history. Mark removed evidence provenance with the recorded source revision rather than preserving dead file links.
- [x] Remove legacy rule/operation/edge readers, temporary adapters and fallback gate paths. Search `rg 'fixtures/phase0|reference/phase0|combined_verdicts|OperationEntry|RuleEntry|EdgeCase' crates tools docs AGENTS.md compat`; permit only explicit historical provenance strings, never active dependencies.
- [x] Run `python3 tools/check_links.py`, `python3 tools/check_evidence.py` and focused parity tests. Inspect corpus/contract counts and retained test coverage; no numeric deletion target.

### Task 7: Make the binary and release visibly breaking

**Files:** modify `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/compatibility.md`, `README.md`; modify `release-plz.toml` only if scratch verification demonstrates it is necessary.

**Interfaces:** `tdo --version` prints `tdo 0.2.0`; all three workspace packages agree. The release pipeline recognizes the breaking change and does not propose `0.1.10` or `0.3.0`.

- [x] Set the inherited workspace version to `0.2.0` and update only corresponding workspace package versions/dependencies in the lockfile. Add breaking migration notes for custom matrix v1, operation report vocabulary and the evidence layout.
- [x] Build/run `cargo run -q -p tdo -- --version`; require exactly `tdo 0.2.0`. Check workspace metadata and lockfile agreement. Keep daemon protocol version independent; this evidence-format change does not imply an RPC version bump.
- [x] Use `feat(compat)!: replace session evidence with operation contracts` and a `BREAKING CHANGE:` explanation for the integration commit/PR title so the existing `^(feat|fix)` release filter includes it. No agent attribution.
- [x] Verify release-plz with the installed pinned version in a disposable local copy containing the intended breaking commit, without network publication. Confirm its proposed release remains `0.2.0` and the breaking migration appears in the changelog. If it double-increments a manual bump, resolve version ownership there before touching the real branch's release configuration.

### Task 8: Final verification and review

**Files:** update this plan with results; no unrelated changes.

- [x] Run `mise run test` and `mise run lint`; inspect every failure/skip, and run the new-format privacy scanner over the retained corpus.
- [x] Verify a built binary from a temporary directory without a fixture checkout using isolated synthetic configuration/socket state; no production daemon or default configuration.
- [x] Review the full diff for lost assertions, private data and undocumented permission changes. Require parity, evidence validation, link checks and the exact binary version result before declaring completion.
- [x] Record before/after file/case/contract counts, checks, meaningful skips and custom-matrix migration in the final report. Obtain the required whole-branch code review for the chosen execution method.
- [x] Offer `hunk` review at completion unless the user already has a live viewer. Keep changes uncommitted while a working-tree review is in progress. Do not publish/tag/merge without separate authorization.

## Planning review

The tasks cover the approved spec's schema, gate composition, observation provenance,
resource membership, pruning, gathering, reports, historical cleanup and release
requirements. The concrete contract catalog and case selection intentionally come
from Task 1's audited current behavior rather than an invented case count. The
temporary inventory is removed when migration is complete.


## Implementation results — 2026-11-11

Completed in focused commits on `refactor/reorganize-evidence`. The contract catalog
contains 82 operation-local contracts across 17 operations. The corpus moved from
3,184 files (2,402 ICS), 25 sessions and 226 steps to 1,866 files (1,840 ICS),
23 focused cases and 116 complete named states. Every retained ICS hash matches
an approved original capture. Result-only observations remain where file bytes
cannot represent the observed behavior.

The concrete pre-cutover baseline still matches all 1,888 decisions (118 tasks ×
eight requests × both write modes). Existing focused tests cover the named plan
boundaries; equivalent tests were retained rather than duplicating tests solely
to match the proposed names. Obsolete v1 row-product/fallback tests were replaced
by strict v2 parser/requirement tests. List atomicity remains independent, and
same-kind/stamp-only saves cannot bypass missing contracts.

Independent whole-branch review found two important issues and one stale source
comment. Three regression tests first demonstrated the issues: capture could
write through an existing manifest symlink/hardlink, and protocol-only evidence
could satisfy a phone requirement. Capture now rejects manifest symlinks and
replaces manifests atomically; passing phone directions require device origins.
The existing list-creation phone confirmation was restored as a focused,
result-only case. The source comment now names current cases. No device round
or production-data access was performed. One final review fix pass was used;
no additional reviewer was dispatched. Live interoperability was outside the
approved scope; offline release behavior was verified independently below.

Verification:

- Full `RUST_TEST_THREADS=4 mise run test`, including real vdirsyncer integration,
  Python helpers, documentation links and evidence validation.
- `mise run lint` (Clippy with warnings denied and rustfmt), privacy scan and
  retained-byte audit.
- An isolated temporary daemon and synthetic vdir confirmed embedded admission
  without a fixture checkout; the built binary reports exactly `tdo 0.2.0`.
- Offline release-plz in a disposable clone proposed `0.1.9 → 0.2.0` for all
  three packages and retained the breaking migration notes; no double increment.
  Nothing was published or tagged.

An earlier unrestricted-parallel full run exhausted the local scratch quota.
Only this worktree's generated incremental build cache was cleared; the entire
suite was then rerun with four test threads. No failing test was skipped.

Custom matrices must migrate to `matrix_version = 2`; v1 fails explicitly before
field decoding. Report operation names and capture layout also change in this
breaking release. Daemon RPC version remains unchanged. The old global gate,
session corpus and historical session documents have been removed. Technical
2Do references, actionable limitations and consent boundaries are retained in
[the evidence guide](../../reference/evidence.md).

The final committed diff is available for terminal review. Integration remains a
separate user decision; the branch and worktree are preserved.
