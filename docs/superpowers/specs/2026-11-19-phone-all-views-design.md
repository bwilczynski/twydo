# Phone app: every view, from a third tab

Date: 2026-11-19. Status: draft for review. Issue #145.

This extends the [phone companion MVP](2026-11-17-phone-companion-mvp-design.md) (its §10 lists
"Other views: All, Lists, a single list" as a later step). That design and the
[main spec](2026-10-31-terminal-tasks-design.md) govern everything not said here; the §11
contracts apply unchanged.

Points marked *(my call)* are proposals the builder may refine with a reason. Everything else is
the user's decision or follows from the existing code.

## 1. Intent

Today the phone shows only Inbox and Today, so any other task cannot be reached away from the
desk. The user wants to browse **all** tasks from the phone: a third tab at the bottom shows the
same list of views the TUI shows in its sidebar (Inbox, Today, All, Scheduled, Done, the lists
under their group headings, Next 3 Days). Tapping one opens that view's tasks.

## 2. Goals, non-goals, anti-goals

**Goals**

- A third tab, the **views list**, with exactly the TUI sidebar's entries, headings, order and
  counts.
- Tapping an entry opens that view **inside the third tab**, with a back button to the views
  list (user decision). This holds for Inbox and Today too: they open inside the third tab; the
  first two tabs are unchanged.
- An opened view behaves as Inbox and Today do now: same rows, sections, live refresh, complete
  with undo, detail sheet, capture with `+`.
- The TUI and the phone cannot disagree on the sidebar: one implementation in `twydo-core`.

**Non-goals**

- Editing, moving, deleting, reopening; list management (create, rename, recolour, archive).
- Folding parents, searching, sorting controls.
- Paging long views (Done can be long; see §7).
- Any change to the TUI's behaviour or to what it asks the daemon. The TUI keeps computing its
  sidebar from its own snapshot (user question, answered: it does not call the new method).

**Anti-goals (failure even if it works)**

- The phone computing membership, order or counts itself, or showing a sidebar the TUI would not.
- Hidden or archived lists appearing on the phone when the TUI's sidebar leaves them out.
- A display device gaining any write.
- Breaking existing clients: `views.get` with `inbox`/`today` keeps its exact result.

## 3. Constraints

- Thin client: the phone never reads the snapshot (MVP design §4).
- Protocol changes are additions to `schema/tdo.openrpc.json` and pass the released-schema test.
- The gateway forwards only allowlisted methods per role (MVP design §7.4).
- iOS Safari home-screen app; light and dark follow the phone.

## 4. Approaches considered

| | Approach | Trade-off | Outcome |
|---|---|---|---|
| 1 | **New method `views.list`** for the views list; `views.get` learns every view. Sidebar code moves into `twydo-core`. | One more method in the schema and allowlist; each method keeps one job. | **Chosen** (user). |
| 2 | Return the views list inside every `views.get` answer. | No new method, but every refresh of any view (and a Today-only display) carries the whole sidebar and its counts. | Rejected. |
| 3 | The phone builds the views list from raw data. | Breaks the thin-client rule; needs the snapshot on the phone. | Rejected. |

## 5. Core: one sidebar

`App::sidebar_entries` (`crates/tdo/src/app.rs`) moves to `twydo_core::views` as a pure
function, e.g. `sidebar(snap: &Snapshot) -> Vec<SidebarEntry>`, with `SidebarEntry` moving too.
The TUI's `view_name` (`crates/tdo/src/ui/mod.rs`) moves with it, so the daemon names views in
the TUI's words. The function already reads only the snapshot (collections, `config.list_prefs`,
`config.groups`, `inbox_list`), so the move is pure; no TUI state (cursor, focus) is involved.
The TUI calls the core function; its behaviour does not change. Existing TUI
tests on sidebar order keep passing unchanged.

## 6. Daemon

### 6.1 `views.list`

Read-only, any session. Computed on the published snapshot at the daemon's clock, as
`views.get` is.

**Params:** `{}`.

**Result** (names illustrative; the builder fixes them in the schema):

- `epoch`, `revision`, `now`, `today`, as in `views.get`.
- `entries`: in sidebar order, each either
  - a **heading** `{ "heading": "<label>" }` (a list group's name, `Lists`, `Smart lists`), or
  - a **view** `{ "view": <ViewRef>, "name", "color", "open_count", "due_count" }`, where
    `color` is the list's `#rrggbb` for a list (absent otherwise), `open_count` and `due_count`
    are `twydo_core::views::counts` (`total`, `due_or_overdue`) exactly as the TUI's sidebar
    shows them.

**The Inbox entry.** The sidebar emits Inbox as `View::List(<inbox_list id>)`. `views.list`
reports that entry as `{ "view": "inbox" }` *(my call)*, so the opened Inbox is the same view as
the Inbox tab: same title, capture, empty state and cache key. This is exact: the sidebar shows
the entry only when `inbox_list` resolves to a shown list, and `views.get` `inbox` resolves to
the same id in that case. Every other `View::List(id)` is `{ "view": "list", "list": id }`.

A test asserts that, for the same snapshot and time, `views.list` gives the same entries, order
and counts as the TUI's sidebar (with the Inbox entry mapped as above).

### 6.2 `views.get` for every view

`ViewName` gains `all`, `scheduled`, `done`, `next_3_days` and `list`. A `list` view carries the
collection id: params become `{ "view": "list", "list": "<id>" }` *(my call: a flat optional
field keeps the existing `{ "view": "today" }` shape unchanged; `list` is required with `list` and
refused otherwise)*. The `ViewRef` in `views.list` entries uses the same shape, so the phone
passes an entry's `view` straight back to `views.get`.

- A collection id that does not exist is refused with kind `invalid`
  ("no such list"). A hidden or archived list that still exists is served: the phone only
  reaches it through a stale views list, and showing it is harmless *(my call)*. Hidden and
  archived are display preferences, not access control: any paired device can already read
  every task through Today, and the TUI reaches such lists with `:list`. So no extra role check
  for `display`.
- `title`: the view's name as in §5.
- `capture`, matching the TUI's `o` on a top-level row: a `list` view captures into that list
  (whatever its hidden/archived state);
  Today into the default list with due today (unchanged); All, Scheduled, Done and Next 3 Days
  into the default list with no due date *(as the TUI does; a task added in Scheduled or Done
  therefore leaves the screen, and the banner's "Added … to <list>" says where it went)*.
- `open_count`: `counts(...).total`, as now.
- `inbox` keeps its current result exactly, including its Inbox resolution.

### 6.3 Events

No new event. The phone refetches `views.list` on `data.changed`, on becoming visible and on the
same timer it uses for views (MVP design §9.4).

## 7. Gateway

`views.list` joins the allowlist of both roles, `phone` and `display` (read-only). The
`views.get` params rewrite in `crates/twydo-gateway/src/bridge.rs` accepts the new values. No
other change: `op.submit` rules stay as they are (a phone `create` already names its collection).

Long views: the Done view returns every completion. No paging in this round *(my call)*; the
builder measures the payload on the synthetic fixtures and notes it in the docs. If it proves
too large on the phone, paging is a follow-up.

## 8. Phone app

### 8.1 Tabs

The tab bar gains a third tab, **Views** *(my call: the TUI's pane title)*, with a list icon.
The remembered tab (`twydo.tab` in local storage) accepts the new value.

### 8.2 The views list screen

```
 Views
 ┌──────────────────────────────┐
 │ ▢ Inbox                    3 │
 │ ▢ Today                 2 │ 5│
 │ ▢ All                  2 │10 │
 │ ▢ Scheduled                1 │
 │ ▢ Done                     2 │
 │ LISTS                        │
 │ ▌ Bills                    6 │
 │ ▌ Personal              1 │ 2│
 │ SMART LISTS                  │
 │ ▢ Next 3 Days                │
 └──────────────────────────────┘
```

- Large title "Views", headings as small-caps section headers, rows as tappable cells with a
  chevron. Lists show their colour bar; built-in views an icon (Inbox and Today reuse the tab
  icons) *(my call)*.
- Counts: the open count on the right; when `due_count` is above zero it is shown first, in the
  overdue colour, as the TUI's `due│total` *(my call: the TUI's information, iOS badge styling)*.
  No count when the total is zero.
- No `+` button on this screen (there is no view to capture into).

### 8.3 An opened view

- Pushed inside the Views tab with a back button labelled "Views" at the top left; the large
  title is the view's `title`. Everything below is the existing view screen: sections, rows,
  complete, undo banner, detail sheet, `+` with the view's `capture`, empty state, offline and
  sync warning.
- The open view is remembered across reloads (local storage, alongside the tab) *(my call)*.
  Switching to Inbox or Today and back returns to it. Tapping the Views tab while a view is open
  pops back to the views list (iOS convention) *(my call)*.
- If `views.get` refuses the view as `invalid` (the list was deleted), the app pops back to the
  views list and shows a banner "That list is gone" *(my call)*.
- Empty state icon for views without their own: a generic list icon.

### 8.4 Data layer

`data.ts` today keys its cached views by `"inbox" | "today"`. It changes to key by a view
reference (a string key derived from `ViewRef`, e.g. `list:<id>`), keeping the in-memory cache
the user chose in #143/#144, and gains a `views()` signal for the views list with its own
refresh. Completion and capture use the `based_on` of the view on screen, as now.

## 9. Documentation and tests

- `docs/protocol.md`: `views.list` and the new `views.get` values. `docs/gateway.md`: the third
  tab and the allowlist. `docs/architecture.md` if it names the sidebar's home.
- Core: unit test of `sidebar` (moved TUI tests). Daemon: `views.list` matches the TUI sidebar;
  `views.get` for each new view matches `build_view`; unknown list refused; `inbox`/`today`
  results unchanged. Gateway: allowlist for both roles. Web: unit tests for the views list,
  navigation, back, remembered view, deleted list; an e2e test opening a list and completing a
  task in it.

## 10. Left to the builder

- Exact schema names for `views.list`, `ViewRef` and entries.
- The list icon, cell styling and push animation, within the app's existing style.
- How the TUI's sidebar tests split between `twydo-core` and `tdo`.
