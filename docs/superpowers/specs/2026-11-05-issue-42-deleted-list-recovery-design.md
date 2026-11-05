# Remotely deleted lists and recovery (issue #42)

Date: 2026-11-05 · Status: approved (2026-11-05)

## Agreed direction

Keep this a manual workflow for one user. A sync failure identifies the affected list;
the user confirms that they deleted it on another device. No automatic remote-deletion
detection, Python helper, Python-runtime discovery, or custom network integration.

## Command flow

1. For a list-specific Not Found error, display the list's local display name, collection
   id, and exact command: `:forget-list <collection-id>`. Say "server returned Not Found";
   do not claim that the whole list was deleted. Include every affected list, even when
   another collection failed in the same run. Keep the normal error detail.
2. `:forget-list <list>` accepts an existing collection id or unique display name
   (the whole remaining argument, including spaces). Show its name, task count, and
   whether it has unsynced work. Ask the user to confirm that they deleted it remotely.
3. On confirmation, preserve the complete local collection outside the synchronized vdir,
   including original task bytes, list metadata, and relevant journal/conflict evidence.
   Then remove the active local copy and refresh vdirsyncer's cached list set using its
   ordinary `discover <pair>` command. Use the existing command, configuration,
   timeout, and locking machinery; never automatically answer yes to creation prompts.
4. Resume ordinary sync after discovery succeeds. If discovery fails, keep the backup and
   report that list discovery still needs attention; retry discovery before another sync.
   Explicit collection entries may need the user's config adjusted. Do not automatically
   edit their config, recreate the deleted server list, or restore its local copy.
5. `:recover` opens the preserved-list browser: backup id, original name, date, task count,
   and recovery status. Enter previews tasks and notes; `r` chooses a destination list.
6. `:recover <backup-id> <destination-list>` opens the recovery preview directly. The
   destination is the entire remaining argument, so list names can contain spaces.
   Accept an existing collection id or unique display name, with no implicit destination.
7. Confirm the backup, destination, task count, and that the local snapshot is being
   restored. Restore the task set with one durable journalled operation and ordinary
   after-edit sync. Retain the backup after recovery. A retry creates no duplicates.

## Preservation and recovery constraints

Keep the backup as inspectable iCalendar plus a versioned manifest under the per-vdir state
directory, with private permissions. No automatic expiry or backup-delete command.

Use the run/vdir locks. Durably publish the whole backup before removing active data.
Resume interrupted archiving and recovery without losing files or creating duplicates.
Refuse symlinks and escaping paths. Preserve unsupported resources instead of discarding them.

Retain unsynced intent as archived, never claim it reached the server. Entries spanning
the retired list and an active list must retain the active list's pending work. Preserve
both sides of existing conflicts in the backup; recovery explicitly restores the local
snapshot, without silently resolving those historical conflicts or writing to the old list.
Make that scope visible in preview. Undo must not recreate the retired collection.

Reuse existing verified move/copy and family handling for recovery into another collection.
Validate the whole set and destination first; refuse unsupported shapes or collisions without
partial writes. Do not change compatibility verdicts. Preserve family links, history,
notes, dates, alarms and unknown properties. Support undo/redo and stale-data checks for
the recovery write. Record its operation and identity mapping for repeat-safe retries.

## Approaches and scope

The earlier automatic discovery/Python-adapter proposal is withdrawn at the user's request.
Deleting local files without a backup would lose unsynced edits. Automatically recreating
the remote list would reverse the user's deletion. The selected approach is explicit local
retirement, ordinary vdirsyncer rediscovery, and optional recovery to an existing list.

Implement the TUI commands, useful sync error guidance, the Rust preservation/recovery
support, and matching as-is docs. CLI parity is optional rather than a separate required
feature. No new collection-management system or per-task archived-conflict editor.

Test with synthetic local data and real vdirsyncer filesystem pairs where applicable:
Not Found messages and names, cancelled confirmation, backup failures, pending edits,
multi-list journal entries, conflicts, failed/retried discovery, empty collection sets,
family recovery, destination collisions, repeated recovery, interruption, undo/redo, and
continued sync of other lists. Run project tests, lint and documentation link checks.

No production NAS, phone, account or data access is authorized.
