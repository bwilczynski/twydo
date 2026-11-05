# Issue #11: always-visible planned and completed work

Status: scope confirmed by the user, 2026-11-05: no controls; always on in qualifying views.

## Behavior

All and ordinary lists show future-start and completed tasks automatically. Future-start
rows use Future, taking precedence over their due-date bucket. Existing long-range due
rows still use Future. Completed rows use a Done section at the bottom, newest completion
first. Empty sections are omitted. Future and Done remain sections with title/creation
sorts; ordinary rows keep their current sorting and grouping.

Today, Scheduled, Done and Next 3 Days retain their current rules. All retains its
hidden/archived/excluded-from-All collection restrictions; explicitly opening a list
retains its existing access rules. The dedicated Done view's show_in_done flag does not
hide completions in their own list or an otherwise eligible All collection.

Each task appears once. Preserve parent expansion and nested item order within each of
three sections: ordinary active, future-start and completed. A child in a different section
from its parent appears separately with parent context; normal date buckets alone do not
split existing active families. Checklist items remain subject to parent expansion when
in their parent's section. Completed children never disappear solely because their open
parent is collapsed.

Sidebar totals count eligible unfinished tasks, including future-start tasks. Completed
rows never inflate unfinished totals or due counts. Keep the due-today-or-overdue numerator
and project-child/checklist-item distinction. The dedicated Done view keeps its completed
total. Search does not change sidebar counts.

No new shortcuts, status hints, visibility preferences, configuration or persistence.
No completion-age windows, auto-deletion, new Today settings, stars, paused tags, counter
settings or sorting controls. No task-resource writes or production/device round.

## Verification

Cover membership, Future/Done precedence, completed ordering, non-date sorts, empty
sections, collection exclusions, family sections, counts, TUI rendering and selection after
completion/reopen/reload. Run project lint, tests and documentation-link checks. Update
as-is docs and explicitly supersede the original spec's future-start hiding contract for
All and ordinary lists.

The user's workflow, rather than full parity, is the target. Background:
[2Do Focus Lists](https://www.2doapp.com/docs/macos/focus-lists/).
