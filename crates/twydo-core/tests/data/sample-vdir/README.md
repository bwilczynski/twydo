# sample-vdir

Synthetic vdir for tests. All content is made up. Reference "now" in tests is
**Wed 2026-11-04 10:00 America/New_York**. 3 collections, 16 tasks. Each collection has
`displayname` and `color` files. Timed values use `TZID=America/New_York`. Every task has
a unique UID (`sv-<collection>-<n>`) and loads with no warnings.

| Collection (displayname, color) | Tasks |
|---|---|
| `bills` (Bills, #EF4444) | 6 |
| `inbox` (Inbox, #3B82F6) | 5 |
| `personal` (Personal, #22C55E) | 5 |

## inbox

| File | Title | Due | Start | Covers |
|---|---|---|---|---|
| `renew-library.ics` | Renew library books | 2026-11-03 (date) | - | overdue, date-only |
| `buy-stamps.ics` | Buy stamps | 2026-11-02 (date) | - | completed (COMPLETED 2026-11-02T17:00Z) |
| `call-dentist.ics` | Call the dentist | 2026-11-04 15:00 | - | timed, due today |
| `pay-electricity.ics` | Pay electricity bill | 2026-11-05 (date) | - | one-time, date-only, tomorrow |
| `read-article.ics` | Read the saved article | 2026-11-15 (date) | 2026-11-10 (date) | future start date |

## bills

| File | Title | Due | Start | Covers |
|---|---|---|---|---|
| `water-bill.ics` | Pay water bill | 2026-11-06 09:00 | - | timed, VALARM (-PT15M relative) |
| `rent.ics` | Pay rent | 2026-11-07 (date) | - | monthly repeat (`FREQ=MONTHLY;BYMONTHDAY=7`) |
| `insurance.ics` | Renew car insurance | 2026-11-20 (date) | - | yearly repeat (`FREQ=YEARLY`) |
| `tax-return.ics` | Prepare tax return | - | - | project (`@tasktype:project`), no dates |
| `tax-receipts.ics` | Collect receipts | 2026-11-08 (date) | - | project child, one-time |
| `tax-forms.ics` | Review draft forms | 2026-11-12 (date) | - | project child, weekly repeat (`FREQ=WEEKLY`) |

Children `RELATED-TO:sv-bills-4` (the project).

## personal

| File | Title | Due | Start | Covers |
|---|---|---|---|---|
| `water-plants.ics` | Water plants | 2026-11-04 (date) | - | weekly repeat (`BYDAY=WE`), due today |
| `packing.ics` | Packing list | - | - | checklist (`@tasktype:checklist`), no dates |
| `packing-1.ics` | Passport | - | - | checklist item |
| `packing-2.ics` | Charger | - | - | checklist item |
| `packing-3.ics` | Umbrella | - | - | checklist item |

Items `RELATED-TO:sv-personal-2` (the checklist).

## Totals useful for assertions

- Open tasks: 15; completed: 1 (`buy-stamps`).
- With a due date: 11 (including the completed one). Without: `tax-return`, `packing`, and its 3 items (5).
- Overdue at reference now (due before 2026-11-04, open): 1 (`renew-library`).
- Due 2026-11-04: `call-dentist` (15:00) and `water-plants`.
- Due 2026-11-05: `pay-electricity`.
- Repeating: 4 (`rent`, `insurance`, `tax-forms`, `water-plants`).
- With an alarm: 1 (`water-bill`).
- Future start (after 2026-11-04): 1 (`read-article`, starts 2026-11-10).
- Projects: 1 (2 children). Checklists: 1 (3 items).
