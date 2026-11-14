# Testing

Run `mise run test` for the Rust workspace, Python helper tests and documentation links.
Run `mise run lint` for Clippy and formatting. Integration tests use synthetic fixtures,
temporary configuration/state directories and their own Unix sockets. The real-vdirsyncer
suite skips when vdirsyncer is unavailable; a skipped run does not verify that integration.

## Evidence cases

`fixtures/<operation>/<case>/case.toml` declares observations, provenance, tests and
complete named vdir states. `python3 tools/check_evidence.py` validates membership and
references; it runs through `mise run test`. Use small local builders for incidental
setup. Keep exact captured bytes only when a parser, transformation or device behavior
is the subject of the assertion. The [evidence guide](reference/evidence.md) covers
pruning and promotion. Historical corpus-size checks were replaced with actual claim
and byte-preservation checks; removed session files remain available in git.

## Where a test belongs

Test a behavior at the lowest layer that can observe its contract. Add a client integration
case when it protects routing, identity, presentation or event ordering across that boundary;
do not repeat the lower layer's entire fixture matrix through every client.

| Owner | Contract |
|---|---|
| `twydo-core/tests/ical_*`, `model_task`, `edit_*` | Parsing, exact resource transformation, recurrence and captured 2Do behavior |
| `twydo-core/tests/gate` | Whether a specific operation is admitted, and why |
| `twydo-core/tests/plan`, `writer`, `history`, `store*`, `journal`, `queue`, `lists_*` | Atomic application, stale dependencies, durable queue, inverse operations and crash recovery |
| `twydo-core/tests/describe` | Pure operation message wording and affected-child counts |
| `twydo-daemon/tests` | Sessions, request replay, revisions, events, jobs, scheduling, RPC reports and service ownership of the core |
| `tdo/tests/app_keys`, `form`, `ui_render` | Emitted requests, input retention, selection, key handling and rendered output |
| `tdo/tests/write_adapter`, `link`, `tui` | Client request parameters, daemon answers, reconnects and TUI event ordering |
| `tdo/tests/cli*`, `daemon_*`, `term` | Binary arguments, stdout/stderr, exit codes, service/process lifecycle and the actual terminal loop |
| `tdo/tests/sync_e2e` | Real vdirsyncer, its executable conflict hook and two filesystem storages |

An admission test and a store test for the same task are not automatically duplicates.
They protect different failures. Likewise, a simulated restart does not replace a process-kill
regression, and a fake sync command does not replace the real vdirsyncer integration.

## Daemon migration cleanup

The November 2026 cleanup keeps these contracts while removing repeated execution:

| Removed or reduced coverage | Retained owner |
|---|---|
| Separate writer undo/redo wording test | Writer `complete_then_undo_then_redo`; daemon `the_answer_carries_todays_message` |
| Repeated stale writer setup and wording | Writer `sync_while_form_open_is_stale_not_overwritten`; daemon `an_edit_of_a_task_changed_since_based_on_is_stale_with_its_current_rev`; client `write_adapter` and `refusal_text_names_the_task_by_the_clients_title` |
| Writer gate-reason comparison | Daemon `the_gate_refusal_is_blocked_with_the_gates_reason`; client adapter error forwarding |
| Writer queue timing, drain, refusal acknowledgement and cancellation scenarios | Daemon `queue`, `sync`, and `writes` suites; client adapter queued-result forwarding and TUI refusal display tests |
| Writer empty-session, restart and deleted-title undo cases | Daemon `undo_is_per_session`, `a_request_applied_before_a_restart_is_replayed_from_the_journal`, and `the_answer_carries_todays_message` |
| Writer startup over an unfinished list operation | Daemon `lists` startup/reconciliation tests, including restart, write and undo; core `read_only_preview_reconciliation_clears_its_fence_before_refusing` |
| Writer state-directory ownership and connecting during a long job | Daemon `lifecycle` and `read`, respectively |
| Writer single-task/family success messages | Pure core `describe` tests, including singular/plural and open/completed-child counts |
| TUI until completion, monthly checklist reset and history reopen matrices | Core `store::fixed_checklist_reset_and_until_end_match_preupload_capture`, `store_family::recurring_checklist_family_completion_matches_both_phone_occurrences_and_undo`, `store::ended_until_reopen_store_is_journalled_and_undoable`, `store_family::history_family_capability_matches_h7`; app key completion/confirmation and Done-history request tests |
| TUI from-completion interval/date edits and checklist schedule refusals | Core `gate` from-completion/checklist capability tests, `edit_fields::existing_from_completion_interval_edit_matches_phone`, and `store_family` checklist schedule tests; form repeat parsing, date-key requests and all-field input-retention tests |
| TUI family deletion/undo/reset and queued item-deletion matrices | Core `store_family` individual/checklist deletion, scope, history/reset and inverse tests; `store_queue::individual_item_deletes_queue_and_refuse_changed_family_bytes`; daemon queue tests; app selected-item/empty-parent checks and one completed-item delete/undo integration |
| CLI report semantic matrices and detailed export shape assertions | Daemon `reports`; CLI retains flags, formatting, exit codes, privacy and write argument mapping |
| Separate repeated shutdown stress scenario | Daemon `shutdown_does_not_wait_for_a_reload_stuck_behind_an_outside_lock`, which explicitly confirms the refresher is blocked |
| Separate panicking sync-job scenario | Daemon `a_panic_in_a_sync_job_fails_it_and_clears_the_sync_state`, including failed sync event and subsequent runner health |
| Three phase-0 fixture subset roundtrip scans | Core `ical_write::unedited_calendar_is_byte_equal`, recursively covering the whole corpus with the original subset-size guards |
| Gate deletion disk/undo assertions already exercised in store tests | Matching core `store_family` capability deletion tests; gate admission assertions and the distinct empty-weekly-project execution case remain |
| Core `store_queue::lost_wakeup_is_drained_by_the_recheck` (the second lock probe after a push, via `submit_with`) | Daemon `an_edit_saved_while_an_outside_process_holds_the_vdir_lock_is_queued_then_applied` and `an_edit_queued_while_the_drain_runs_is_drained_after_the_job`; the history's single probe stays covered by core `history` `redo_of_a_cancelled_step_with_the_lock_free_applies_it_and_reports_the_drain` |
| Core `store_requests::take_drained_returns_items_a_submit_drained` and `a_store_collects_nothing_drained_unless_asked` (drained-item collection in the per-session store) | Daemon `queue::the_drain_after_the_job_applies_and_reports_op_finished`, `queue::an_edit_queued_while_the_drain_runs_is_drained_after_the_job` and `sync::a_sync_that_drains_then_fails_finishes_each_drained_item` for drained items and their `op.finished`; core `history` `redo_of_a_cancelled_step_with_the_lock_free_applies_it_and_reports_the_drain` for the drain sink |

The remaining TUI writer cases are representative workflows: a complete/undo/redo roundtrip,
refused stale form, recovery job waiting, chained item creation, repeat card, new checklist
card, closed checklist text editing, completed-item deletion and moves. Recurrence variants
and exact file preservation remain covered in core instead of being re-executed through each
of these workflows. No compatibility captures or production behavior were changed.
