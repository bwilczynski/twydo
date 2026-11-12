# Best-effort list writes implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open list rename/recolor and list deletion under an explicitly accepted best-effort risk, and implement the missing DELETE path.

**Architecture:** A new `accepted_risk` field on the two list contracts lets `Matrix::require` admit them despite failed atomicity. Rename/recolor already works end to end behind the gate. Deletion adds a `delete` helper action (revalidate against the preview backup, DELETE the exact URL, reconcile by reading), a Rust `delete_list` that drives it as a durable list operation, and local retirement through the existing `retired::forget` machinery.

**Tech Stack:** Rust workspace (`twydo-core`, `twydo-daemon`, `tdo`), bundled Python helper over vdirsyncer 0.21 (`crates/twydo-core/src/lists/helper.py`), offline DAV backend `crates/twydo-core/tests/common/list_backend.py`.

**Spec:** [best-effort list writes design](../specs/2026-11-12-best-effort-list-writes-design.md); background in the [issue 69 design](../specs/2026-11-08-issue69-list-management-design.md) and [plan](2026-11-08-issue69-list-management.md).

## Global Constraints

- Atomicity stays recorded as `fail` on both rows; never set it to `pass`.
- Best-effort is on for everyone through the embedded matrix; no config switch.
- No NAS, phone or production access. All tests use the offline backend/transport.
- Matrices without `accepted_risk` behave exactly as today in both write modes.
- No `If-Match` on the collection DELETE. No blind DELETE repeat after an uncertain outcome.
- Restore/import of deleted lists is out of scope (#75); export of the backup must keep working.
- Tests isolate `XDG_CONFIG_HOME` and `TWYDO_SOCKET`; cargo target stays out of `/tmp`.
- Commits have no agent attribution trailer (user's global instructions).

## Review Focus

1. A list edited on another device between preview and confirmation: delete must refuse and ask for a fresh preview, never delete the changed list. (Task 3 `test_delete_refuses_changed_inventory_or_properties`, Task 4 `delete_refuses_changed_local_bytes_without_sending_delete`.)
2. Crash/timeout after the DELETE was sent: reconciliation must not resend DELETE when the list is gone, and must not leave a fence that blocks all sync. (Task 4 `timeout_after_delete_reconciles_absence_and_retires_without_a_second_delete`.)
3. After a successful delete, the next ordinary sync must not recreate the list from the local copy or the vdirsyncer status. (Task 4 `deleted_list_is_not_recreated_by_the_next_sync`.)
4. A custom `--matrix` without `accepted_risk` must keep refusing; a contract with `accepted_risk` on an unrelated row must fail to parse. (Task 1.)
5. The deleted list being the currently open view in the TUI: the TUI must leave that view and drop it from the sidebar. (Task 5 `deleting_the_open_list_returns_to_today_and_drops_it_from_the_sidebar`.)

---

### Task 1: Accepted-risk gate and truthful refusals (#84)

**Files:**
- Modify: `crates/twydo-core/src/compat/contracts.rs` (`Contract`, `Matrix::from_toml_str`, `Matrix::require`)
- Modify: `crates/twydo-core/src/lists/remote.rs:43-60` (`capability`)
- Modify: `compat/matrix.toml` (rows `list-metadata.collection`, `list-delete.collection`)
- Modify: existing refusal tests in `crates/twydo-core/tests/lists_remote.rs` (`shipping_metadata_gate_requires_server_protection_even_in_test_mode`, `metadata_interoperability_cannot_bypass_missing_or_failed_atomicity`, `missing_or_failed_capabilities_refuse_test_mode_too`) and the parity case `list_atomicity_failures_remain_refused` in `crates/twydo-core/tests/gate_contract_parity.rs`
- Test: compat tests next to the existing `rejects_unknown_and_duplicate_contracts` (find with `grep -rn rejects_unknown_and_duplicate_contracts crates/twydo-core`)
- Docs: `docs/compatibility.md`

**Interfaces:**
- Produces: `pub enum AcceptedRisk { BestEffort }` (serde `"best-effort"`), `Contract.accepted_risk: Option<AcceptedRisk>`.
- Produces: refusal text = `RequirementFailure`'s `Display`, plus the sentence `" The server does not protect against concurrent remote edits."` only when `direction == Some("atomicity")`.

- [x] **Step 1: Write failing tests**
  - `accepted_risk_is_only_valid_on_list_metadata_and_delete`: parsing a v2 matrix with `accepted_risk = "best-effort"` on `title.*` (any non-list row) → `MatrixError::Invalid`; on either list row → Ok.
  - `best_effort_admits_failed_atomicity_and_unverified_directions_in_both_modes`: row with `atomicity = "fail"`, all directions `unverified`, `accepted_risk = "best-effort"` → `require` Ok in `Verified` and `Test`.
  - `best_effort_still_refuses_fail_or_unsupported`: same row with `twydo_to_iphone = "fail"` → Err direction `twydo_to_iphone`; with `atomicity = "unsupported"` → Err direction `atomicity`; with `iphone_baseline = "unsupported"` → Err.
  - `without_acceptance_list_rows_refuse_as_before`: the embedded rows with `accepted_risk` removed → Err direction `atomicity` in both modes.
  - `embedded_matrix_admits_list_metadata_and_delete`: `Matrix::embedded().require(ListMetadata|ListDelete, Verified)` → Ok.
  - `refusal_names_the_failing_direction` (#84): custom matrix from the issue (atomicity pass, `twydo_to_iphone` unverified, no acceptance, Verified mode) → `ListService::execute(Edit…)` error contains `list-metadata.collection: twydo_to_iphone` and not `concurrent`.
- [x] **Step 2: Run** `cargo test -p twydo-core --test lists_remote --test gate_contract_parity` plus the compat test target; expect the new tests to fail to compile or fail.
- [x] **Step 3: Implement.** Parser rejects `accepted_risk` outside the two list contracts (same place as the `atomicity` check). In `require`, for `ListMetadata|ListDelete` with `accepted_risk == Some(BestEffort)`: refuse only if atomicity, `iphone_baseline` or either direction is `Fail`/`Unsupported`; otherwise Ok, skipping the `strict`/Verified direction loop. Without acceptance, unchanged. `capability` uses the failure's `Display` (+ atomicity sentence) and reports the unknown `case` name.
- [x] **Step 4: Update matrix rows.** Add `accepted_risk = "best-effort"` to both; keep `atomicity = "fail"`. Notes: `"Best-effort: accepted by the user on 2026-11-12. The server does not enforce protection against concurrent remote edits; a phone change racing a Twydo write can be overwritten (metadata) or lost (delete)."` For delete, add the lifecycle-round phone disappearance as `twydo_to_iphone` evidence only if `python3 tools/check_evidence.py` accepts the existing case; otherwise leave it unverified.
- [x] **Step 5: Re-point the old refusal tests** at a matrix with the acceptance stripped (helper `without_acceptance(Matrix) -> Matrix` in the test file), keeping their assertions. Update the parity case: list rows are now admitted under the embedded matrix and still refused without acceptance.
- [x] **Step 6: Run** `cargo test -p twydo-core` and `python3 tools/check_evidence.py`; all pass.
- [x] **Step 7: Docs.** `docs/compatibility.md`: explain `accepted_risk`, that it exists only for the two list contracts, and what risk was accepted.
- [x] **Step 8: Commit** `feat(compat): accept best-effort list metadata and deletion`.

### Task 2: Rename/recolor live

**Files:**
- Modify: `crates/twydo-core/tests/lists_remote.rs` (`prototype_metadata_service` callers)
- Modify: the list card's Save to CalDAV explanation (find with `grep -rn "Save to CalDAV" crates/tdo/src`)
- Test: `crates/tdo/tests/lists.rs`
- Docs: `docs/tui.md` (the "Name/color editing is currently refused" paragraph, ~line 131)

**Interfaces:**
- Consumes: Task 1's embedded matrix admitting `ListMetadata`.

- [x] **Step 1: Write failing tests**
  - core `embedded_matrix_renames_and_recolors_in_verified_mode`: offline backend, `ListService::open(&cfg, None)` with `cfg.writes.mode = Verified`, create then Edit name and color → `state()` shows both; task file bytes unchanged; exactly one PROPPATCH call in `server.json`.
  - tdo `card_explains_best_effort_overwrite_risk`: rendering the edit card contains `can overwrite a simultaneous change made on another device`.
- [x] **Step 2: Run** them; expect failures (the first currently refuses on atomicity, the second lacks the text).
- [x] **Step 3: Implement.** Replace `prototype_metadata_service` uses that only existed to bypass the gate with `ListService::open`; keep the synthetic matrix only where a test needs an otherwise-unavailable verdict. Add the copy to the card.
- [x] **Step 4: Run** `cargo test -p twydo-core --test lists_remote` and `cargo test -p tdo --test lists`; pass.
- [x] **Step 5: Docs.** `docs/tui.md`: rename/recolor saves to CalDAV in both modes, best-effort, may overwrite a simultaneous phone edit.
- [x] **Step 6: Commit** `feat: enable best-effort list rename and recolor`.

### Task 3: Helper `delete` action

**Files:**
- Modify: `crates/twydo-core/src/lists/helper.py` (`validate_request`, `dispatch`, `main`'s failure receipts)
- Modify: `crates/twydo-core/tests/common/list_backend.py` (DELETE support + flags)
- Test: `tools/tests/test_list_helper.py` (extend `InventoryDAV`/`MemoryDAV` with DELETE)

**Interfaces:**
- Request: `action = "delete"`, requires `operation`, `state`, `collection`, and `backup` (plain id of a backup under `<state>/lists/backups/<backup>/remote.json`). Modes: `execute`, `reconcile`.
- Execute: resolve target exactly as `preview-delete`; `read_properties` + `capture_inventory`; compare with the backup's `remote.json` (`properties` bytes, and the set of `(href, etag, sha256)`) → mismatch raises `Refused("changed")`. Persist `remote_target` in the record, then `DELETE` the exact URL without `If-Match`. 2xx or 404 → result `{"deleted": true, "collection": ...}`; other statuses → `Refused("dav")`.
- Reconcile: read the saved `remote_target` only. PROPFIND 404 → `{"deleted": true}`; present and equal to backup → `Refused("present")`; present and different → `Refused("changed")`. Unreachable → `Refused("dav")` (never treated as absence).
- Failure receipts (in `main`): execute-mode `changed` records `{"no_remote_write": true, "reason": "changed"}` (as `stale`/`exists` do today); reconcile-mode `present` or `changed` records `{"present": true, "reason": code}`.
- `"delete"` is added to the modes allowed with `reconcile`; `refresh` stays create/edit only.

- [x] **Step 1: Write failing tests**
  - `test_delete_request_requires_backup_operation_and_state`
  - `test_delete_revalidates_then_sends_one_unconditional_delete`: DAV records one `DELETE` to the exact collection URL with no `If-Match` header.
  - `test_delete_refuses_changed_inventory_or_properties`: an extra resource, a changed ETag, changed bytes, or changed properties → `Refused("changed")`, zero DELETEs.
  - `test_delete_refuses_incomplete_inventory_before_delete` (reuse the inventory faults).
  - `test_delete_reconcile_distinguishes_absent_present_changed_and_unreachable`.
- [x] **Step 2: Run** `python3 -m unittest tools.tests.test_list_helper`; new tests fail.
- [x] **Step 3: Implement** per Interfaces. Reuse `capture_inventory`/`read_properties`; compare hashes, not the raw base64.
- [x] **Step 4: Backend.** `list_backend.py` records `DELETE` in `calls`, removes the collection from `server.json`, and supports flags `hang_after_delete_once` and `mutate_before_delete` (adds a resource after the preview) for Task 4.
- [x] **Step 5: Run** the Python tests and `python3 -m unittest discover -s tools/tests -t .`; pass.
- [x] **Step 6: Commit** `feat(helper): delete a list collection after revalidating its backup`.

### Task 4: `delete_list` service, retirement and reconciliation

**Files:**
- Modify: `crates/twydo-core/src/lists/backup.rs` (`delete_list`, module doc)
- Modify: `crates/twydo-core/src/lists/remote.rs` (`ListResult`, `reconcile_with_server` delete branch)
- Modify: `crates/twydo-core/src/lists/helper.rs` (`HelperRequest.backup: Option<String>`)
- Modify: `crates/twydo-core/src/retired.rs` (constructor for a `Preview` from a list-backup's local tree if `forget` cannot take the backup directly)
- Test: `crates/twydo-core/tests/lists_recovery.rs`

**Interfaces:**
- Produces: `ListResult::Deleted { operation: String, collection: String, name: String, backup: String, retired: String }` (`retired` = the `retired::Backup` id).
- Produces: `ListService::delete_list(&mut self, preview: &DeletePreview) -> Result<ListResult, ListError>` (signature unchanged).
- Flow: `validate_binding` → `capability(ListDelete)` → load backup `preview.backup`, require `backup.collection == preview.collection` → compare current local tree of `vdir/<collection>` with `backup.local_hashes` (mismatch → `Refused("the list changed since the backup; review deletion again")`, no operation prepared) → `operation::prepare` with action `delete` and `backup` → `helper::invoke`. On `RemoteConfirmed`: `retired::forget` (exclusive locks, writes `sync/rediscover`, retires journal entries) using the backup's local files as the expected tree → `operation::finish` → `Deleted`.
- Helper `changed` → cancel the unstarted operation, return the same "review deletion again" refusal. Timeout/other failure after start → keep the record `Uncertain`, return `"list operation {id} needs reconciliation: …"` (existing pattern).
- Produces: `operation::release_present(cfg: &Config, id: &str) -> Result<(), ListError>` in `crates/twydo-core/src/lists/operation.rs`: only for a `delete` record whose receipt has `present: true`; marks it `Complete` and clears `lists/active` under the same locks `finish` uses. (`cancel_unstarted` refuses started operations, so it cannot serve here.)
- Reconcile (`action == "delete"`): `RemoteConfirmed` or reconcile result `deleted` → retire and finish. `present` → `release_present` and refuse `"the list still exists on the server; delete it again if you still want to"`. `changed` → `release_present` and refuse with "review deletion again". `dav` → keep `Uncertain`.

- [x] **Step 1: Write failing tests**
  - `delete_sends_one_delete_retires_local_copy_and_keeps_backup`: preview then `delete_list` → `Deleted`; `server.json` has one `DELETE` and no collection; `vdir/synthetic` is gone; `backup::get` and `backup::export` still work; `lists/active` absent; `sync/rediscover` written.
  - `delete_refuses_changed_local_bytes_without_sending_delete`: modify `vdir/synthetic/local.ics` after preview → refusal containing `review deletion again`, zero DELETEs, no active operation.
  - `delete_refuses_changed_remote_inventory_without_sending_delete` (flag `mutate_before_delete`).
  - `delete_refuses_pending_queue_or_conflict_for_the_list` (reuse the existing preflight fixtures from `lists_operations.rs`).
  - `timeout_after_delete_reconciles_absence_and_retires_without_a_second_delete` (flag `hang_after_delete_once`, short `timeout_secs`): first call errors with `needs reconciliation`; `reconcile(op)` → `Deleted`; exactly one DELETE.
  - `reconcile_with_list_still_present_releases_the_fence`: DELETE never reached the server (fault before send) → reconcile refuses with `still exists`, no active operation, local copy intact.
  - `deleted_list_is_not_recreated_by_the_next_sync`: after delete, run the existing offline sync path used in `sync_discover.rs`; no MKCALENDAR, no `vdir/synthetic`.
- [x] **Step 2: Run** `cargo test -p twydo-core --test lists_recovery`; new tests fail (today `delete_list` always refuses).
- [x] **Step 3: Implement** per Interfaces. Update the `backup.rs` module doc (no longer "No DELETE is issued").
- [x] **Step 4: Run** `cargo test -p twydo-core`; pass.
- [x] **Step 5: Commit** `feat: delete lists from CalDAV after a verified backup`.

### Task 5: Daemon result, TUI completion and docs

**Files:**
- Modify: `crates/twydo-daemon/src/protocol/jobs.rs` (`JobResult::ListDeleted`), `crates/twydo-daemon/src/server/lists.rs` (`job_result`), `crates/twydo-daemon/src/schema.rs` if it enumerates results
- Modify: `crates/tdo/src/app/lists.rs` (completion handling), `crates/tdo/src/app/recovery.rs:33-36` (confirmation text)
- Test: `crates/twydo-daemon/tests/lists.rs` (replace `delete_keeps_todays_refusal`), `crates/tdo/tests/lists.rs`
- Docs: `docs/tui.md` (`dd` paragraph ~lines 142-148), `docs/protocol.md`, issue-69 design/plan pointer lines

**Interfaces:**
- Produces: `JobResult::ListDeleted { operation, collection, name, backup }` and the TUI-side equivalent of `ListResult::Deleted`.
- Confirmation text: `Delete {name:?} ({collection}) from CalDAV and ALL DEVICES? Backup {backup} preserves {resources} remote resource(s) and {local_files} local file(s); it can be exported but not yet restored. Best-effort: a change made on another device at the same moment can be lost. y delete · n/Esc cancel`
- Success message: `Deleted {name} from CalDAV; backup {backup} kept`.

- [x] **Step 1: Write failing tests**
  - daemon `delete_job_reports_the_deleted_list` (offline backend): `lists.preview_delete` then `lists.delete` → result kind `ListDeleted` with the backup id.
  - tdo `deleting_the_open_list_returns_to_today_and_drops_it_from_the_sidebar`.
  - tdo `delete_confirmation_defaults_to_cancel_and_states_best_effort`: Enter/Esc/`n` cancel without a job; text contains `Best-effort`.
  - tdo `changed_since_backup_refusal_is_shown_and_dd_previews_again`.
- [x] **Step 2: Run** `cargo test -p twydo-daemon --test lists` and `cargo test -p tdo --test lists`; fail.
- [x] **Step 3: Implement.** On `Deleted`: remove the collection from `snapshot.collections`, `open(View::Today)` if it was open, close the card, `sync_sidebar()`, set the success message. Remove the `ListAction::PreviewDelete | Delete => "List saved"` arm's misleading text.
- [x] **Step 4: Run** both test targets plus `cargo test -p tdo`; pass.
- [x] **Step 5: Docs.** `docs/tui.md`: `dd` deletes after backup/confirmation, best-effort, backup exportable, restore not yet (#75). `docs/protocol.md`: new result. Add a one-line pointer to this plan in the issue-69 design's "Open decisions" and the issue-69 plan's ledger.
- [x] **Step 6: Commit** `feat(tdo): delete lists from the sidebar`.

### Task 6: Full validation and review

- [x] Run `cargo test --workspace`, `python3 -m unittest discover -s tools/tests -t .`, `python3 tools/check_links.py`, `python3 tools/check_evidence.py`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`, `git diff --check`. All pass.
- [x] Packaged-helper smoke: run the embedded helper source from a scratch directory with `{"version":1,"action":"probe",...}`; it answers `vdirsyncer-0.21`.
- [x] Privacy-scan new fixtures and staged content (`git diff main --stat`, inspect new fixture bytes).
- [x] Whole-branch review via `superpowers:requesting-code-review`; fix concrete findings in focused commits with targeted reruns.
- [ ] Offer the Codex review (`hc run … reviewer`) and a terminal diff review (`git diff main HEAD | hunk patch`).
- [ ] After merge (user's call): comment on and close #73, #74 and #84, stating the policy basis. No release or installed-binary update is included.

## Execution ledger

- 2026-11-12: design approved by the user and committed (`975c590`). Plan written; no product code changed.
- Tasks 1–5 implemented subagent-driven with a task review each (Task 3: one fix round, 207 Multi-Status no longer counts as deleted; Task 4: two fix rounds, retirement after a confirmed or uncertain delete can no longer wedge the vdir fence). Final whole-branch review found no Critical/Important issues; one fix wave corrected two stale statements, recorded the retired id in the delete operation and split a key arm. Final validation: 1,621 Rust tests, Python tools, links, evidence, clippy, fmt and whitespace checks pass; packaged helper probe passes from a scratch directory. No NAS/phone access.
- Rulings: under `accepted_risk`, failed atomicity is admitted and only `unsupported` refuses; `retired::forget_fenced`/`operation::finish_deleted` added because the plain forget/finish cannot run under the delete's own fence; a deleted list's local copy also appears in forgotten-list recovery; after a "changed since backup" refusal the card stays on Delete (Enter or Esc then `dd` re-previews). Parked: the crash-window fallback in `deleted()` could report a future-dated retired id.
