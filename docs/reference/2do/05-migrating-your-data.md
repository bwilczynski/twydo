# 05 — Getting existing 2Do data into the port

**Use the Dropbox sync folder.** It's fully decoded: `tools/decode_2do_dropbox.py` reads
a local copy of the 2Do Dropbox folder (`Apps/2Do`, name localized per Dropbox language) into JSON.
Decoding was checked against a local dataset; its contents and inventory are not included. [obs]
All concrete dates, timestamps, and example configurations below are synthetic.

```sh
python3 tools/decode_2do_dropbox.py /path/outside/repo/2Do /path/outside/repo/inventory.2do.json
```

Backups and CSV/ICS exports (bottom of this page) remain fallbacks.

## Folder layout [obs]

| Folder | Entity (`entity` field) | One file per | Port target |
|---|---|---|---|
| `tod/` | `task` (alarms embedded) | task | `task`, `alert` |
| `col/` | `calendar` | list, smart list, Starred/Scheduled/Inbox | `list`, `smart_list` |
| `cal/` | `calgroup` | list group | `list_group` |
| `cat/` | `tag` | tag | `tag` |
| `loc/` | `location` | location | (out of scope) |

File names encode IDs (from SQL in the app and confirmed on disk):
`o_u_k_<listUID>_p_<parentTaskUID>_i_<uid>_d_.<ext>`. Empty segments are omitted values
(`o_u_k__p__i_<uid>_d_.cat`). List-group files use fixed IDs: `2DoCalGroupFocus`,
`2DoCalGroupInbox`, `2docalgrouplists`, … [obs]

## File format [obs]

`gzip( NSKeyedArchiver binary plist )`. The archive root key `data` is a **flat dictionary of
strings** (numbers as `"0"`, `"1.000000"`). A task's `alarms` is an array of dictionaries.
Python's `plistlib` reads it; the decoder resolves the `$objects`/UID indirection.

**Timestamps are Unix epoch seconds as strings** (`"946728000.000000"`). Dates without a time
are stored twice: `due = "2000-01-01"` (floating date) and `duedateabs` = that day at **12:00 UTC**.

## Task fields (`tod/`)

| Key | Meaning | Example values (synthetic) |
|---|---|---|
| `uid` | task id | 32 hex |
| `caluid` | list uid | |
| `parent` | parent project/checklist uid | empty for top-level |
| `title`, `notes` | text | |
| `tasktype` | **0 task, 1 checklist, 2 project** [obs, code + data] | 0, 2 |
| `due` | due date `YYYY-MM-DD`, empty = none | |
| `duedateabs` | due date as epoch (noon UTC) | |
| `duetime` | **HHMM integer** (`930` = 9:30); `999999` or `-1` = no time; `0` = 00:00 (midnight). 2Do itself treats `0` as a time: on a CalDAV upload it writes those tasks as `DUE:<date>T000000`, and `999999` ones as `DUE;VALUE=DATE` [obs code / obs data: phase-0 upload] | 0, 999999 |
| `startdatetime` | start date/time, epoch; key absent = no start | |
| `iscomp` | completed | 0/1 |
| `compdate` / `compdatetime` | completion, human string / epoch | |
| `createdate` | created, epoch | |
| `datestamp`, `datestampnorm` | modified, epoch (norm = timezone-normalised) | |
| `priority` | 0–3 | 0–3 |
| `starred` | 0/1 | |
| `taskduration` | minutes | |
| `displayorder` | manual order | |
| `sortfield`, `sortorder` | project's own sorting | |
| `expanded` | UI state | |
| `repeattype` | `recurrenceType`: 0 none; 1–127 weekday bitmask; 256 daily, 257 weekly, 258 monthly, 259 yearly; 260–266 Nth Mon…Sun; 267 last day of month; 268 Nth weekday; 269 Nth weekend day. Full table in [01 §6](01-features.md) [obs] | 0, 258, 259 |
| `repeatval` | `recurrenceValue`: interval N, or ordinal 1–5 (other = last) | 0, 1 |
| `recurrence` | `recurrenceFrom`: **2 = repeat from completion**, else from due date. Also set on non-repeating tasks; use `repeattype != 0` for "repeats" [obs] | 0, 1 |
| `recurrenceendtype`, `recurrenceendrepeats(orig)` | 0 never, 1 on date, 2 after N (remaining / original count) [obs] | 0 |
| `tags` | Optional. Same serialized string as the DB column: entries split by `+$$\|$$+`, fields by `~\|$$@$$\|~`, field 0 = name (see [02](02-data-model.md)). Sync key name `tags` [inf] | — |
| `startdaydelay` | start offset for repeats | 0 |
| `ruid` | on a **completed occurrence**: uid of the live repeating task it came from | |
| `lastuid` | link to the previous instance [inf] | |
| `acttype` / `url` | action (−1 none) | −1 |
| `deleted` | tombstone; skip if `1` | 0 |
| `status` | sync status, ignore | 1, 3 |
| `alarms[]` | `uid, taskuid, action` (`CalAlarmActionSound`…), `sound` ("Basso"), `isauto`, `relduetime`, `reltrig`, `lreltrig`, `relduetrig`, `issnooze`, `deleted` | |

Tasks without assigned tags may omit the tag key; the serialized format is known from
the code (row `tags` above).

### How repeating tasks look on disk [obs]

Completing a repeating task **creates a completed copy** (`iscomp=1`, `repeattype=0`,
`ruid=<live uid>`) and **moves the live task's due date forward** by the rule. For example,
a synthetic monthly project on the 10th can have completed copies due 2000-01-10 and
2000-02-10, with the live task due 2000-03-10. Import the copies as completed history
(the Done list) and the live task as the repeating one.

## List fields (`col/`)

`uid, title, color ("r,g,b" floats 0–1), parentuid (group id), position, sortfield, sortorder,
canshowintoday, canshowinall, canshowindone, isarchived, hidden, expanded`, flags for the special
lists `isinboxcal, istodaycal, isallcal, isscheduled, donecal, starred`, and for smart lists
`issmartcalendar, smartsearch, smartrangefrom, smartrangeto, smartsearchusesoundex,
excludecompleted, combinelist, smartrangeexcludesstart`. [obs]

- A synthetic "Upcoming" smart-list example: `smartsearch="*"`, `smartrangefrom=7`, `smartrangeto=1`,
  `excludecompleted=1`. This reads as "next 7 (count) days (unit 1)". [inf]
- `sortfield`: `1` Due Date (default), `11` Start Date (Scheduled), `13` Creation (Inbox); full code table in [01 §12](01-features.md). [obs]
- Today/All/Done have no files (they're built in); Starred, Scheduled and Inbox do.

## List groups (`cal/`) and tags (`cat/`)

- Group: `uid, name, disporder, collapsed`. Built-in groups include FOCUS, COLLECT (Inbox), LISTS, SMART LISTS.
  User-made groups (Personal, Work…) get generated uids. [inf]
- Tag: `uid, name, type, isheld (paused), disporder, pauid/panam (tag group, "TAGS_WITHOUT_A_GROUP"), meta`.

## Fallbacks

- **Backup** (2Do › Settings › Backups): snapshots named `.2DoBackupArchive-/Package-/Snapshot-`,
  likely containing the SQLite DB from [02-data-model.md](02-data-model.md). [inf]
- **CSV** (File › Export › Export as CSV): titles, lists, dates, notes; columns [?].
- **ICS** (File › Export › Export as Calendar File): `VTODO`s with `RRULE`, `DUE`, `DTSTART`,
  `RELATED-TO`, and `X-2DOAPP-*` extensions. Useful to cross-check repeat rules.

## Two-way sync later (optional)

The port could read *and write* this Dropbox format (gzip + keyed-archive plist, same
file names), which would keep the Mac/iOS apps in sync with Linux. Risky: 2Do's Dropbox sync
also uses ZIP folder snapshots, conflict and revision logic. A self-hosted CalDAV server is the
safer two-way option.
