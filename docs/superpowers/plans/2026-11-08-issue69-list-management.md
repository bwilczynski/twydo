# Issue 69: list management implementation plan

> **For agentic workers:** Use `superpowers:executing-plans` to implement this plan inline, task by task. Track checkboxes and record verification here. The user requested a branch in the existing checkout and focused commits.

**Goal:** Create, edit, archive/unarchive and safely delete/recover lists from Twydo.

**Architecture:** Rust owns the list service, persisted operation phases, local preferences and TUI. A bundled Python helper uses the configured vdirsyncer installation for CalDAV collections; task sync stays in the existing runner. Local archive uses existing view preferences. Remote permissions remain separate until their bounded interoperability checks pass.

**Tech stack:** Existing Rust workspace, ratatui, TOML editing, bundled Python, vdirsyncer 0.21 async adapter, existing journal/queue/locks and synthetic DAV tests.

**Spec:** [Approved list-management design](../specs/2026-11-08-issue69-list-management-design.md). [Completed archive investigation](../../reference/evidence.md).

## Global constraints

- Use branch `feat/69-list-management` in this checkout; no worktree. Focused commits, no agent attribution.
- Local tests and synthetic inputs only during implementation. No user-config edits, installed-binary replacement, publication or push.
- The archive round is complete. Do not repeat its uploads/actions, reuse its list for deletion, or clean up any synthetic lists.
- Fresh approval is required for any additional NAS/phone round. Prepare exact candidates, target binding and a bounded checklist first.
- Archive is local to this desktop, available offline, and must not claim to change phone archive state.
- Preserve existing Next 3 Days/Done semantics. Keep unknown task bytes and properties intact on unrelated changes.
- Remote changes are explicit immediate actions, including when automatic sync is off; label saves **Save to CalDAV**. No offline collection queue.
- Do not enable deletion until server-enforced collection concurrency protection is demonstrated or a replacement policy is explicitly agreed.
- Bundle the helper in the installed binary; no dependence on a repository-relative script or separate credentials.

## Review focus

1. A sync/credential wrapper need not use the same Python as `python3`; unsupported adapters must fail before network access.
2. Display names can duplicate and discovery can map different local/remote IDs; all writes use verified pair/collection identity.
3. A child helper can outlive its TUI; no other process may sync or write through unfinished remote/local finalization.
4. Remote-only resources and malformed/unrecognized task content must remain recoverable after deletion, independent of task operation gates.
5. Archive can remove the selected sidebar entry; chooser access, selection fallback and other running TUIs must remain coherent.

## File and interface map

- `crates/twydo-core/src/lists.rs`: public service and types. `ListService::open(cfg: &Config, config_path: Option<&Path>) -> io::Result<ListService>`, `set_archived(&mut self, id: &str, archived: bool, expected: &ListState) -> Result<ListState, ListError>`, `execute(&mut self, request: ListRequest) -> Result<ListResult, ListError>`, `reconcile(&mut self, operation: &str) -> Result<ListResult, ListError>`.
- `lists/prefs.rs`: `set_archived(path: &Path, vdir: &Path, id: &str, archived: bool, expected_bytes: &[u8]) -> io::Result<Config>`. Atomic, stale-checked TOML change to exactly `[lists.<id>].archived`.
- `lists/operation.rs`: versioned inspectable `ListOperation` with stable ID, exact target, request, held local bytes, phases, remote receipt and backup reference. States `Prepared`, `RemoteStarted`, `RemoteConfirmed`, `LocalConfirmed`, `Complete`, `Uncertain`.
- `lists/helper.rs`: embedded helper launch, versioned stdin/stdout JSON, timeout/process-group cleanup and redacted errors. `invoke(cfg: &Config, request: &HelperRequest) -> Result<HelperReply, ListError>`.
- `lists/helper.py`: vdirsyncer adapter, validated identity resolution, guarded DAV operations and child-held store locks. No imports from the repository.
- `lists/backup.rs`: private durable raw-resource/property/local-state backup, `DeletePreview`, and recovery export/import planning. Reuse retirement storage/UI where contracts fit; do not reinterpret old backups.
- `crates/tdo/src/listform.rs`, `app/lists.rs`, `ui/listcard.rs`: list card, list-management picker, confirmation and progress; terminal loop runs service actions off the input thread.
- Tests: `crates/twydo-core/tests/lists.rs`, `lists_helper.rs`, `lists_recovery.rs`; `crates/tdo/tests/lists.rs`; `tools/tests/test_list_helper.py`. Integrate queue/sync regression tests in their existing suites.

`ListRequest` variants: `Create { id, name, color }`, `Edit { id, name, color, expected }`, `PreviewDelete { id }`, `Delete { preview }`, `Restore { backup, destination }`. `ListState` contains stable collection ID, current display name/color and archive preference. `ListResult` carries operation ID, resulting state or preview/backup, and completion status; no credentials or raw connection settings in displayable errors.

## Task 1: Local archive persistence and reload

**Files:** `lists.rs`, `lists/prefs.rs`, `config.rs`, `lib.rs`, core Cargo files, `tdo/src/term.rs`, `tests/lists.rs`.

- [x] Add failing tests for target-only preference changes, stale config, quoted IDs/comments/groups, wrong vdir/symlinks, and missing saved config. Assert unrelated config text and task bytes remain unchanged.
- [x] Run `cargo test -p twydo-core --test lists`; observe the missing archive service fail before implementation.
- [x] Implement the service/archive interface above with the bound vdir lock, a config-file lock and atomic writes. `toml_edit` preserves comments, formatting and unrelated values; compare original bytes before replacement. Reject a changed vdir binding. Preserve config permissions.
- [x] Ruling: move terminal preference reload and explicit CLI config-path wiring into Task 5, where the service first enters the TUI. Its cross-process reload and active-card tests belong to that integration; this core commit adds no exposed write key.
- [x] Run focused archive/config tests without sync configuration. Nine archive tests and 18 config tests pass; core-library clippy passes. Commit `feat: persist local list archiving`.

## Task 2: Bundled helper, adapter and exact target resolution

**Files:** `lists/helper.rs`, `lists/helper.py`, `config.rs`, `SyncConfig`, core helper tests and Python tool tests, `docs/config.md`, `docs/install.md`.

- [x] Add helper tests for protocol/target validation, duplicate mappings/discovery identities, traversal/encoded separators, account roots, foreign-origin redirects, read-only storage and wrong local vdir. Credential-boundary tests prove invalid pairs do not fetch secrets. Rust subprocess tests cover redacted malformed output and bounded timeout.
- [x] Run Python/core helper targets; missing helper/module/config fields fail before implementation. A further duplicate-discovery regression failed before adding duplicate-aware selection.
- [x] Add `[sync].list_python` defaulting to `python3`, embedded `include_str!` source and direct interpreter launch. Initially support only vdirsyncer `0.21.x`, checked before config/credential/network access.
- [x] Reuse vdirsyncer config/fetch/auth/TLS and discovered mapping APIs without creation prompts. Validate local storage/path and remote type/read-only state before remote credentials. Reject targeted redirects and unverified response identities.
- [x] Exercise actual DAV parsing over an offline transport: VTODO validation, duplicate responses, failed property status and redacted errors. Raw library stdout/stderr does not enter displayed error messages.
- [x] Helper/config Rust tests and seven Python helper tests pass, core all-target clippy passes, and a bundled-source probe runs from `/tmp` with installed vdirsyncer 0.21.0 without config or network access. Commit `feat: bundle vdirsyncer collection helper`.

## Task 3: Durable operations and locking through helper completion

**Files:** `lists/operation.rs`, `lists/helper.rs`, `lists/helper.py`, `store.rs`, `store/queue.rs`, `sync/run.rs`, core helper/operation tests.

- [x] Add coordination tests: helper locks after parent exit, unfinished marker blocking recovery/sync/queue, stale bytes, phase prerequisites, malformed markers, unreadable queue and unrelated queued work retention. Action-specific uncertain-create tests are assigned to Task 4 below.
- [x] Run affected targets before adding coordination; the absent operation module fails. The unreadable-queue regression then fails against the existing queue reader before the strict preflight fix.
- [x] Recover task journal and check target pending/queue/conflicts under run/vdir locks; persist `Prepared` and exact collection bytes. Publish the active marker under a fence shared with queue insertion.
- [x] Helper-owned run/vdir locks and bound request/byte checks are implemented. Cross-process Python `flock` versus Rust `File::lock` and parent-exit lifetime tests pass on Linux; macOS execution remains unverified.
- [x] Persist remote-started/confirmed/uncertain receipts under child locks. Store/queue/recovery paths check the marker; finalization requires explicit local confirmation and fresh local bytes under run/vdir/fence locks. Completed records remain inspectable.
- [x] Ruling: exact remote outcome reconciliation belongs to Task 4 (create/edit) and Task 6 (delete/restore), where desired remote state is defined. This task retains the immutable target/request and refuses incomplete phases; it does not add a blind reissue path. The later tasks must test timeout after remote success and per-action phase interruptions.
- [x] Run affected operation/journal/queue/sync/helper targets. Commit `feat: coordinate collection operations with task sync`.

## Task 4: Collection creation and metadata edits

**Files:** `lists.rs`, `lists/helper.py`, `lists/operation.rs`, `compat/matrix.toml`, core list tests, compatibility docs.

- [x] Add tests for VTODO-only create, existing identity refusal, exact metadata write set/read-back, property failure/false success, stale values, unknown content/task preservation, mapped identities, timeout after create, explicit save with automatic sync off, missing pair and capability refusal.
- [x] Observe missing service and create/edit functions fail first. Additional local metadata replay, refresh-mode protocol and stale saved-write-policy regressions fail before their fixes.
- [x] Implement create with caller-held fresh identity and create-only MKCALENDAR; verify properties before local metadata/discovery. Use vdirsyncer's own cache API with both missing-collection creation entry points disabled, and reject redirects/foreign origins. No cache-format rewriting or blanket prompt acceptance.
- [x] Implement field-only metadata edits with held-value checks, per-property 207 verification and read-back. Record local_base/local_after before metadata writes. Read-only reconciliation uses the saved exact URL; Keep server is an explicit adoption choice, never an edit retry.
- [x] Add separate collection capability rows. Phone-created list baseline is passed from the approved archive setup, while all helper round trips and metadata/deletion baselines remain unverified. Verified mode refuses; test mode requires an existing non-failed row. No task verdict changes.
- [x] Positive/negative service tests pass and verified mode refuses unverified remote capabilities. Fresh workspace validation passes 1,188 Rust tests across 50 targets; 62 Python tool tests, workspace clippy, formatting, links and whitespace checks pass. Commit `feat: create and edit CalDAV lists through the helper`.

## Task 5: List card, management chooser and local archive UX

**Files:** `listform.rs`, `app/lists.rs`, `app.rs`, `app/recovery.rs`, `ui/listcard.rs`, `ui/picker.rs`, `ui/mod.rs`, `ui/status.rs`, `term.rs`, TUI tests, `docs/tui.md`, `docs/tasks.md`.

- [x] Add failing key/render tests for sidebar `o`/`i`, non-list views, `:lists` including hidden/archived/empty/duplicate names, palette/custom RGB validation, empty names, Esc cancellation, archive unfinished confirmation, and unarchive access.
- [x] Run `cargo test -p tdo --test lists`; verify missing UI behavior fails.
- [x] Implement the card and chooser using stable IDs and the service interface. Start with the 11 exact preset colors in the reference palette, then accept any valid `#RRGGBB`. Keep the existing group on edit; new lists join the ungrouped Lists section. Do not add group editing.
- [x] Separate **Save to CalDAV** from **Archive on this desktop**. Retain input and show a specific refusal when remote capability/pair/interpreter is unavailable. Run service work on a worker thread and route structured completion to the card; a second action cannot supersede an active request.
- [x] Archive/unarchive uses the confirmed preference snapshot, warns for unfinished tasks, updates view/selection coherently, and preserves existing Next 3 Days/Done semantics. Test another running app reload, selected-list disappearance, active card input and task keyboard regressions.
- [x] Run list TUI tests plus keyboard, recovery and write targets. Update as-is docs for enabled local behavior and accurately label gated remote actions. Commit `feat: manage lists from the sidebar and list chooser`.

## Task 6: Deletion preview, raw backups and guarded recovery

**Files:** `lists/backup.rs`, list service/helper/operation modules, existing retirement/recovery integration, core recovery tests, TUI confirmations/browser.

- [ ] Add failing tests for remote-only resources, malformed VTODOs/unknown files, incomplete inventory, weak/missing validators, property-read failures, stale task bytes, queue/conflicts, backup fsync failure, collection identity change, and a recreated same-name collection. Assert zero DELETEs until the backup and concurrency gate pass.
- [ ] Run the new targets; observe the intended missing safety/recovery assertions.
- [ ] Implement `PreviewDelete`: under operation locks, capture complete remote inventory/bytes and collection properties plus raw local files and relevant state; reject ambiguous/incomplete results. Preserve private backup trees outside vdir before confirmation and record target inventory validators/hashes. Show all-device deletion scope and task/resource counts with default cancel.
- [ ] Implement `Delete` only behind its unverified capability and a demonstrated server precondition. Revalidate the confirmed preview; use the verified collection validator atomically at DELETE. If no proven validator exists, return a precise refusal rather than substituting an item ETag or claiming an atomic inventory check.
- [ ] After confirmed absence, retire the local copy under existing recovery/discovery coordination. Timeout/partial finalization retains backup and `Uncertain`; prevent ordinary sync from recreating it. Test kill/retry at every remote/local phase.
- [ ] Implement a versioned list-backup browser/export and restore into a fresh or explicitly selected destination through guarded operations. Include remote-only resources. Keep unknown/unimportable raw content exportable and report it explicitly; never silently skip it or replay a foreign/stale task journal.
- [ ] Add actual TUI `dd`, cancel, backup selection, recovery and failure tests. Document distinct collection recovery versus task undo. Run focused service/TUI/recovery/sync tests. Commit deletion/recovery separately from UI changes where practical.

## Task 7: Bounded collection lifecycle verification and capability decisions

**Files:** new session protocol/evidence under `docs/reference/phase0/`, sanitized fixtures, matrix and corresponding regression tests.

- [x] Prepare one new disposable synthetic-only collection with exact create/name/color candidates and a small fixed task inventory. Privately bind account/IDs, backup paths, helper version and exact request scope. No access is authorized by this plan.
- [x] Present a bounded checklist and obtain fresh approval for creation, metadata phone round trips, conditional deletion tests, deletion and recovery. Do not reuse/delete `Twydo 69 Archive` or any previous families. Give one phone action at a time and compare before proceeding.
- [ ] Verify helper-created VTODO collection/name/color on the phone and a subsequent phone edit. Capture exact identity/metadata/task preservation; enable only passed capability rows.
- [ ] On the disposable collection, prove the proposed collection deletion precondition rejects a stale validator after a resource mutation. A version that does not track membership/content is insufficient. If unsupported, leave deletion refused and present the concrete limitation and possible policy change for user decision; do not widen permission on optimistic local tests.
- [ ] If protection passes, verify deletion propagation and guarded recovery, including a remote-only task, exact preserved content and phone follow-up. Record failure/uncertainty separately. No cleanup beyond the exact agreed disposal scope.
- [ ] Add regression fixtures/tests for observed server behavior, update matrix/as-is docs and commit evidence separately from any permission enablement.

## Task 8: Full validation, review and handoff

- [ ] Run `cargo test --workspace`, `python3 -m unittest discover -s tools/tests -t .`, `python3 tools/check_links.py`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` and `git diff --check`. Build only in the existing disk-backed target directory.
- [ ] Check the packaged helper runs without source files and reports a mismatched Python dependency cleanly. Privacy-scan all new fixtures and inspect staged content.
- [ ] Request an independent whole-branch review using `superpowers:requesting-code-review`; address concrete findings with focused commits and targeted reruns. Record precise enabled/refused capability scope and unresolved device limitations here.
- [ ] Offer final terminal diff review. Since focused commits are authorized, use `git diff <base> HEAD | hunk patch` (or installed fallback) and say the review covers committed changes. No push, PR, release or installed-binary update is included.

## Execution ledger

- 2026-11-12: rename/recolor and deletion were later admitted as accepted best-effort writes; see [the best-effort list writes plan](2026-11-12-best-effort-list-writes.md).

- 2026-11-08: user approved the product design after completion of the archive round and requested focused commits. Implementation is inline on the existing branch. Plan prepared from the installed vdirsyncer 0.21.0 API and existing Rust lock/journal/queue/TUI interfaces. Product code has not yet changed.
- Planning review: all design requirements map to the tasks above. Remaining deletion concurrency evidence is an explicit device checkpoint, not a presumed capability. The existing archive round does not authorize additional remote work.
- Task 1 RED: absent `lists` service prevented the archive tests from compiling. GREEN: nine archive tests prove exact unrelated-text/task-byte preservation, stale/wrong-store/symlink refusal, missing-config refusal, new quoted tables, config permissions and service preconditions. One expanded test initially omitted its temporary state directory and hit the sandbox; corrected the fixture, with no user-state write. UI path/reload integration is carried by Task 5.
- Task 2: embedded helper and versioned bounded transport are available internally; only probe/exact read-only resolution actions exist at this checkpoint. No new device access or remote mutation was performed. Ruling: traverse the adapter's discovery generator directly because its convenience dictionary silently overwrites duplicate identities; the regression now refuses them. Broader service/UI and write/recovery behaviors remain later tasks.
- Task 3: eight Rust operation tests and eight Python helper tests pass; affected journal (24), queue (34) and sync-run (72) targets pass. The helper retains both store locks after its launcher exits. A strict queue reader is used only for collection preflight, preserving ordinary queue reporting while refusing unknown pending targets. Action-specific reconciliation tests remain Tasks 4/6; no external action was performed.
- Task 4: 11 Rust remote-service tests and 13 Python helper tests pass, plus local metadata replay coverage. The Rust service drives the actual embedded helper source/protocol/locks/phases against an offline adapter/transport fixture; no NAS requests. Tests prove one create across timeout/reconciliation, no repeated edit when keeping server values, account-config digest refusal and write-policy refresh refusal. Full workspace initially exposed two historical operation-count assertions (127 → 130 for the three new rows); corrected and a fresh full run passes all 1,188 Rust tests across 50 targets. Python tools pass 62 tests; workspace clippy, formatting, links and whitespace checks pass.
- Ruling: the helper rejects dynamic `url.fetch` layouts before credential expansion because their changing output cannot bind the same account across separate operation phases. Other credential fetching and TLS settings remain delegated to vdirsyncer. Supporting dynamic account URLs would need a separate identity-binding contract.
- Ruling: a remote target that disappeared after an uncertain create/edit is not silently recreated or treated as a metadata retry. Explicit acceptance of remote absence must use Task 6's backup/retirement path before clearing the fence; carry that case into deletion/recovery tests.
- Task 3 follow-up: a new process regression proved that an orphan helper lacked its own timeout. Added a child-enforced deadline and durable uncertain state; it terminates its owned process group without depending on potentially stalled async/credential cleanup. The regression failed before the fix; 12 remote-service, nine operation, three helper and 13 Python helper tests now pass, with workspace clippy clean. No device action was performed.

- Task 5: ten list-key/worker/render tests pass, with existing keyboard, recovery, write, renderer and terminal suites passing. Archive reload keeps active input and selects Today when appropriate. Refused remote saves retain input; pending creates remain reachable without a local folder. Workspace clippy and docs links pass. Deletion remains a Task 6 refusal.
- Task 5: Ruling: remote helper phases hold the exclusive run lock and a shared vdir lock; local finalization remains exclusive — remote phases never write the vdir and readers must stay responsive — a cross-process regression proves writers are excluded while a shared reader proceeds. Any future helper vdir mutation must acquire the exclusive lock.
- Helper follow-up: commit `507c36b` preserves the caller working directory for relative vdirsyncer paths. Four helper tests pass, including the packaged-source smoke from `/tmp`.

- Task 6 partial checkpoint: raw preview/backup/export and TUI confirmation/browser are implemented. Python inventory tests were observed RED before implementation; 15 helper tests and five Rust backup tests now pass, including remote-only/malformed/unknown bytes, strong validators, changed inventories, failed publication, interrupted read-only capture, private permissions and hash checking. No DELETE implementation or network/device action was performed.
- Task 6: Ruling: stop the mutation/restore portion at the required collection-precondition checkpoint; implement no speculative DELETE mechanism — earlier private evidence reports no collection getetag, and neither sync-token nor ctag has demonstrated DELETE protection — lifecycle restore and accepting a vanished pending create/edit remain explicitly unavailable until that dependency is resolved. The cost is that issue 69 is not complete yet; raw backup export is already operable.
- Task 7 preparation: [bounded lifecycle protocol](../../reference/evidence.md), private exact IDs, three synthetic resources/hashes and isolated vdirsyncer configs are ready. No credentials were expanded, no global configuration changed and no network requests were made. Fresh approval remains required under AGENTS.md.

- Independent whole-branch checkpoint review (through `7b2ec5e`) found seven Important issues, no Critical or Minor issues. One fix pass addressed every finding with an observed failing regression and a focused commit:
  - `b7a37b5`: reject ambiguous mapped/dynamic pair expansion (`test_mapped_dynamic_layout_and_multiple_bindings_are_refused`).
  - `ac5843d`: distinguish failed/omitted metadata reads from explicit 404 absence (`test_unreadable_or_omitted_color_cannot_become_an_absence_receipt`).
  - `f29b24b`: permit collection getetag 404 while retaining strong child validators (`test_collection_etag_absence_does_not_weaken_resource_backup_validators`).
  - `bda5eb4`: refuse discovery caches inside the vdir, including symlink aliases, before credentials (`test_discovery_cache_inside_vdir_or_symlink_alias_is_refused_before_credentials`).
  - `26a1165`: validate saved state-dir binding for create, preview and reconciliation (`changed_saved_state_binding_refuses_create_preview_and_reconciliation`).
  - `79bbefa`: retry failed writer initialization after reconciliation, preserving healthy-store undo (`writer_opened_during_a_pending_list_operation_recovers_after_reconciliation`).
  - `2bb63b4`: correlate completions with their card; unrelated export failures cannot unlock it (`unrelated_export_or_list_completion_cannot_unlock_a_busy_card`).
- Checkpoint verification after the fix pass: all 1,210 Rust tests and 68 Python tool tests pass. Workspace clippy with warnings denied, formatting, docs links and whitespace checks pass. The packaged-source helper smoke passes from `/tmp`; the bounded private Rust driver was relinked against the corrected core and its no-network binding checks pass. No fresh NAS/iPhone actions have occurred.
- Review: Ruling: refuse mixed three-part aliases and dynamic collection shortcuts instead of attempting an incomplete vdirsyncer expansion — guessing can select a target outside the actual pair — this narrows supported collection configurations; explicit mappings or pure dynamic layouts remain available and the restriction is documented.
- Review scope rulings: destructive operations/restore/vanished-target retirement stay pending the explicit checkpoint (cost: issue incomplete); NAS/iPhone guarantees remain unverified until fresh approval/evidence (cost: shipping remote gates remain closed); macOS cross-process locking is unverified on this Linux machine (cost: no new macOS interoperability claim). Full-suite validation was rerun by the implementer after all review fixes rather than delegated (cost: no independent full-suite execution, but recorded commands pass). No deferred minor findings.

- PR review follow-up: reproduced a concurrent phone rename arriving between metadata preflight and PROPPATCH; the prototype overwrote it and readback reported success. No verified atomic metadata precondition is available. The shipping `list-metadata` row is therefore now enforced in every write mode; test mode cannot bypass it. `shipping_metadata_gate_requires_server_protection_even_in_test_mode` was observed RED (edit succeeded), then GREEN (refusal before helper/network/operation preparation). Existing prototype/reconciliation tests use an explicit synthetic matrix only with the offline transport. Updated compatibility notes, as-is docs, design and proposed device protocol; phone interoperability alone cannot promote this row. Full validation passes 1,211 Rust tests, 68 Python tests, workspace clippy, formatting, links and whitespace checks. The race is gated, not claimed solved by the prototype.

- Lifecycle round approved explicitly with yes on 2026-11-09. The bundled helper/Rust service created only the prepared lifecycle list and completed local discovery. Read-only raw backups verified the empty list, then the two seeded synthetic baseline tasks. A missing DTSTAMP in the original prepared fixtures was corrected before requesting phone verification; vdirsyncer ignores stamp-only sync changes, so its resource API uploaded the corrections with captured strong If-Match ETags. Final resource bytes match the sanitized fixtures exactly. Remote-only task, metadata mutation, LOCK/DELETE guard probes and recovery remain unexecuted. [Session record](../../reference/evidence.md); first phone confirmation is pending. User separately authorized opening the existing draft PR #72; evidence updates continue there.

- Lifecycle continuation after phone baseline confirmation: post-phone resources matched exactly. Server granted finite exclusive depth-infinity locks but omitted lockroot and returned property 404 for lockdiscovery. The approved name-only negative PROPPATCH succeeded without a token; locks were released and that candidate was stopped.
- Ruling: investigate the independently observed strong calendar-export ETag within the already approved version-test scope, without any more lock-based writes — metadata-lock failure does not establish export-version behavior — this adds only the allowlisted third task and its explicit resource-version test; it does not grant metadata-write authority.
- Export ETag changed after adding the remote-only task and after a captured one-second DTSTAMP update. A complete service backup captured all three remote resources, including the absent-local task. The approved stale If-Match DELETE nevertheless returned 204 and removed the exact target. Read-only exact-URL PROPFIND/GET confirmed absence; user confirmed phone disappearance. No DELETE retry or fresh unconditional deletion was sent. Only the private test sync configuration was sealed, and no recovery list was created.
- Verified the core raw-export API locally: all three remote-resource hashes match, two local task files are retained, and the source backup is unchanged. This is snapshot export evidence, not remote restore completion. Captures and anonymized guard results are in `fixtures/phase0/v1/issue69-lifecycle/`; [session results](../../reference/evidence.md) distinguish successful creation propagation from failed write protection.
- Safety regression: `metadata_interoperability_cannot_bypass_missing_or_failed_atomicity` was observed RED (fully promoted interoperability allowed a write), then GREEN after requiring the independent `list-metadata-atomicity` edge. Both modes reject missing/unverified/failed/unsupported atomicity before any helper invocation. Two failed atomicity edges record the live server findings. Only create Twydo-to-iPhone is promoted; reverse creation remains unverified. All 1,212 Rust tests, 68 Python tests, clippy, formatting, links and whitespace checks pass.
- Required policy checkpoint: both tested server-enforced protections failed. Metadata/deletion remain refused and Task 6's mutation/remote-restore work is unfinished. Continuing needs another proven mechanism or explicit user approval of best-effort list writes, whose recovery covers only the captured snapshot and cannot guarantee recovery of changes arriving afterward. No such relaxed policy has been assumed.

- PR #72 crash-recovery follow-up: reproduced a crash before the local metadata atomic rename leaving `.displayname.tdo-tmp` or `.color.tdo-tmp`, which blocked reconciliation. The end-to-end offline service regression was observed failing before the fix. The helper now recognizes only scratch files absent from the saved baseline whose bytes are a prefix of a planned metadata replacement; Rust validates the complete collection before removing those files under the exclusive lock and retrying. Empty/partial/full scratch writes recover without another MKCALENDAR. Pre-existing scratch files, incompatible contents, unrelated files and changed task bytes remain protected. Workspace tests pass 1,215 tests; Python tools pass 68 tests; clippy, formatting, documentation links and whitespace checks pass. No remote/device action or compatibility permission change was performed.
