# 06 — Sync, and how to do two-way sync from the port

Part 1 explains how 2Do syncs. Part 2 explains how a Linux port could join the same
sync so the Mac and iPhone apps keep working alongside it.
Legend: **[obs]** seen in the binary or the user's data · **[inf]** inferred · **[?]** open.

---

# Part 1 — How 2Do syncs

## 1.1 Architecture [obs]

- Engine: `SyncEngine` / `SyncEngineCoordinator` in `TodoDAOFramework`, one **provider** per
  service, each with its own settings class: CloudKit (iCloud), CalDAV, WebDAV, Dropbox, Reminders
  (EventKit), Todoist, Toodledo, local folder (plus in-memory/local for tests).
  **Exactly one provider is active** (`syncmethod` table: `syncmethod`, `subtype`).
- The engine runs in a **separate helper process** (`TDMSyncProcessHost`, `SyncTerminationSession`),
  so the UI stays responsive.
- Self-signed or unknown TLS certificates ask the user (`SyncServerTrustChallenge`,
  `TDMSyncProcessTrustRequest`).
- Shared lists (iCloud/LAN) are a separate system: Automerge CRDT documents in the `shared*`
  tables. Not covered here.

## 1.2 Local change tracking [obs: SQL in the binary]

Every syncable row (tasks, calendars, calgroups, tags, taggroups, alarms, tasksections,
taskattachments, locations) carries:

| Column | Meaning |
|---|---|
| `syncstatus` | **1 = new / needs full upload** (default on insert; re-adding a list resets its rows to 1) · **2 = in sync** · **3 = modified since last sync** (editing a `2` row sets `3`; local deletes are set to `3`) |
| `isdeleted` | soft delete; the row stays as a tombstone until the delete has been pushed |
| `changed_properties` | which fields changed, as numeric property IDs appended by the SQL function `sqliteAppendToChangedPropertiesTracking` (e.g. `'30'` = image) |
| `modduringsync` | the row was edited while a sync was running, so it goes out next round |
| `modguid` | change identifier |
| `datestamp` / `datestampnorm` | last-modified time (normalised for timezone) |
| `webdav_rev` | provider revision (WebDAV ETag / Dropbox `rev`) of the last version seen |
| `synchref` | provider path or href of the object (e.g. `/tod/o_u_k_…_d_.tod`) |
| `remoteuid`, `remotetoken`, `remotemetadata` | provider identity (sections and newer objects) |
| `modstamp*` (tasks) | per-field timestamps: title, tags, notes, due, start, priority, audio, picture, alarm |

## 1.3 A sync run [obs log strings, order inf]

1. Optional **backup before sync** (setting).
2. **Pull**: ask the provider what changed since the saved checkpoint (Dropbox cursor, CalDAV
   sync-token / ctag+ETags, CloudKit change token). With no checkpoint, list everything.
3. **Apply remote changes** to rows that have no pending local changes.
4. **Conflict step** for rows changed on both sides (§1.4).
5. **Push**: upload rows with `syncstatus` 1 or 3, delete remote objects for local tombstones,
   store the returned revisions (`webdav_rev`, `synchref`).
6. Mark pushed rows `syncstatus = 2`, purge pushed tombstones, **save the new checkpoint at the
   end of the run** ("Dbx fetching latest cursor at the end of sync").

## 1.4 Conflicts [obs]

- **CloudKit:** whole-record, **newer wins**. "Save conflict: the local copy is newer, resending it
  rebased on the server record" / "the server copy is newer, taking it".
- **Dropbox / WebDAV / CalDAV:** a shared "normal conflict step":
  - remote change and no local pending change → apply remote;
  - both changed → conflict step (newer modification wins [inf], using `datestamp` and per-field `modstamp*` [inf]);
  - "stale remote write": the upload hit an outdated server copy → refresh, re-resolve, retry or keep remote;
  - "stale remote delete": a remote delete clashes with a local edit → resolved by keeping or deleting the refreshed item.
- **First sync / reset**, the user picks one:
  - **Merge** — "will try to match existing tasks and lists before adding anything new. If an item was created separately on both sides and cannot be matched confidently, both copies will be kept." Slow, one-time.
  - **Replace Local Data** — download everything, discard local.
  - **Replace Server (Dropbox/CalDAV) Data** — wipe the remote folder/collections and upload everything.

## 1.5 When it syncs [obs]

- On launch ("Sync on Launch"), manually (⌘S), and automatically after local edits ("Making a change on one device informs the rest of your devices to sync too").
- CloudKit push subscriptions trigger a sync (`[PUSH SYNC]`).
- **LAN hints:** devices on the same Wi-Fi announce changes over Bonjour `_2do-sync._tcp` (`SyncLANHintService`) so peers sync sooner.
- Some UI actions block automatic sync briefly (`URLActionAutomaticSyncBlocker`, `SyncAutomaticBlockerRegistry`).

## 1.6 Dropbox provider in detail [obs]

Everything lives in the app folder (`/Apps/2Do` on Dropbox; the folder name is localized).

**Folders and file names.** One file per object, at `/<folder>/o_u_k_<listUID>_p_<parentUID>_i_<uid>_d_.<ext>`.
Unused segments stay empty:

| Folder/ext | Object | Name pattern (from the user's data and SQL) |
|---|---|---|
| `tod` | task (alarms embedded) | `o_u_k_<listUID>_p_<parentTaskUID>_i_<taskUID>_d_.tod` |
| `col` | list / smart list / Starred / Scheduled / Inbox | `o_u_k_<listUID>_p__i__d_.col` |
| `cat` | tag | `o_u_k__p__i_<tagUID>_d_.cat` |
| `cal` | list group | `o_u_k__p__i_<groupUID>_d_.cal` (built-in groups use fixed ids `2DoCalGroupFocus`, `2DoCalGroupInbox`, `2docalgrouplists`, …) |
| `loc` | location | `o_u_k__p__i_<locationUID>_d_.loc` |

Newer object types (sections, section memberships, tag groups, image and voice attachments) use
their own folders and extensions: see §2.7 for the full table.
Because the list and parent are part of the file name, **moving a task to another list or project
is a rename** (delete the old path, write the new one) [inf].

**File body.** `gzip( NSKeyedArchiver binary plist )`. `$top.data` is an `NSMutableDictionary`
whose values are **strings** (numbers as `"0"`, `"1.000000"`, dates as Unix-epoch strings);
`alarms` is an `NSMutableArray` of dictionaries. Field reference: [05](05-migrating-your-data.md).

**Change detection.** Dropbox `list_folder` + `list_folder/continue` with a saved **cursor**
("Dbx queryDelta, using previous cursor", "delta sync asked for, but no saved cursor was found.
Listing everything instead"). The cursor is saved per sync policy at the end of a successful run
and tied to the verified Dropbox account. A cursor reset (`CursorResetSnapshot`) triggers an
authoritative sweep of the lists.

**Download.** Whole folder as a ZIP (`download_zip`) when possible, validated for size, content
hash, duplicate entries, path escapes and links. It falls back to individual file downloads if the
folder changes during validation or exceeds Dropbox's limits ("Dbx skipped folder zip because
Dropbox's entry cap includes the top folder"). Rate limiting is honoured ("Dropbox asked us to
slow down").

**Upload.** Batched uploads (`upload_session` + `finish_batch_v2`, "Dropbox batch upload reported
too many write operations") with **`WriteMode.overwrite`** (§2.7). The rev from each upload is stored in `webdav_rev`.

**Delete.** Real **file deletion** with no tombstone files on Dropbox: "Dbx delete files matching",
batch deletes (`delete_batch`), "Dbx delete all objs of type", folder deletes. Other devices see
the deletion as a `deleted` entry in `list_folder/continue`.

**Locking.** `DropboxSyncLock`: a `/<deviceID>_<timestamp>.tsl` file at the folder root, created
for the destructive *Replace Server Data* operation ("A Dropbox Replace Remote sync lock is active").
Details in §2.7.

**Attachments and sections** have their own change checks ("Dbx attachment file changed on
Dropbox…", "Dbx section membership changed…") that defer to the conflict step when there are
unpushed local edits.

## 1.7 CalDAV provider in detail [obs: DAVKit.framework + strings]

- Built on **libical** inside `DAVKit.framework`. HTTP methods: `PROPFIND`, `REPORT`, `PUT`,
  `DELETE`, `MKCALENDAR`, `MKCOL`, `PROPPATCH`, `MOVE`. Uses **sync-token** when the server supports
  it (`caldav_synctoken`, `caldav_usessynctoken` on each list), otherwise ETag comparison.
- **List = calendar collection** that supports `VTODO`. **Task = one `VTODO` resource.**
- Standard properties: `SUMMARY` (title), `DESCRIPTION` (notes), `DUE`, `DTSTART`, `PRIORITY`,
  `STATUS`/`COMPLETED`/`PERCENT-COMPLETE`, `CATEGORIES` (tags), `RELATED-TO` (parent),
  `RRULE` (repeat), `VALARM`+`TRIGGER` (alerts), `URL`, `CREATED`, `LAST-MODIFIED`, `LOCATION`.
- 2Do extensions: `X-2DOAPP-METADATA` (2Do-only fields such as start date semantics, star,
  task type, actions, repeat-from-completion) [obs: `<2Do Meta>` JSON, §2.6],
  `X-2DOAPP-RELDUETIME`, `X-2DOAPP-RELATIVE-PROJECTION`, `X-2DOAPP-ISSNOOZE`,
  `X-2DOAPP-LOCATION-ONLY`, `X-2DOAPP-ISBIRTH`, `X-2DOAPP-EMAIL`, and media
  `X-2DOAPP-IMAGE` / `-AUDIO` (+ `-SYNC`, `-DURATION`), plus Apple's `X-APPLE-STRUCTURED-LOCATION`,
  `X-APPLE-PROXIMITY`, `X-APPLE-RADIUS`, `X-APPLE-ABUID`.
- Unsupported recurrence parts are dropped with a log ("Ignoring unsupported CalDAV BYMONTHDAY=").
- Servers 2Do knows by name: iCloud (app-specific password), Fastmail, fruux (`dav.fruux.com`),
  generic CalDAV.

### Static reader/writer findings (2026-11-01)

**[obs: REA + Ghidra, macOS 2026.4.6 arm64 DAVKit]** The recovered methods are
`CalDAVKitTask::parseAsVTODO` (`0x409e4`) and
`convertToVTODOUsingExistingCalendarData:usingProductID:` (`0x3e1bc`). These are
static code paths, not observations of a running app. Binary identities and coverage limits
are recorded in [07 §7](07-reverse-engineering-notes.md#7-caldav-static-review-2026-11-01).

- **Completion import:** the reader first derives `isCompleted` from `STATUS`, then reads
  `COMPLETED`. If the latter parses to a non-null date, it forces `isCompleted = true`.
  Thus `STATUS:NEEDS-ACTION` plus a valid `COMPLETED` timestamp is completed at this layer.
  A present but unparseable `COMPLETED` value does not itself force completion.
- **Completion export:** the writer sets `STATUS` from `isCompleted`, sets
  `PERCENT-COMPLETE` to 100 for completed tasks, and resets an existing value of 100 to 0
  for open tasks. It writes or removes `COMPLETED` according to the task's completed date.
  This is serialization logic; it does not establish how recurrence completion creates history.
- **Dates:** the normal writer branch constructs `DTSTART` and `DUE` from the same due-date/time
  components. An `omitEqualTaskStartDate` flag bypasses normal `DTSTART` creation.
  **[?]** The caller policy for that flag was not recovered. The iPhone phase-0 observation in
  §2.6 remains valid for those captures; it should not be generalized to every macOS export.
- **Recurrence:** the writer builds `RRULE` through libical from frequency, count/until,
  interval and BY* strings. The reader parses `RRULE` and can mark additional recurrence
  constraints. This does not establish support for executing every parsed rule class.
- **Preservation:** the writer can parse existing calendar data before updating properties;
  it updates `DTSTAMP` and `LAST-MODIFIED`. This is not proof of byte-for-byte preservation.

**Twydo comparison at review time:** `model/task.rs::read_todo` and
`edit/complete.rs::state` give an explicit `STATUS` precedence over `COMPLETED`, unlike
the DAVKit reader above. Without `STATUS`, they also use property presence rather than a
successfully parsed completion date. This is a known interpretation discrepancy, not a
compatibility pass. No implementation change or matrix verdict change was made in this review.
Metadata precedence, recurring copy-and-advance and provider conflict policy remain unresolved
by this static review; existing fixture evidence retains its own scope.

---

# Part 2 — Two-way sync from the Linux port

## 2.1 Options

| Option | Effort | Risk | Notes |
|---|---|---|---|
| **A. Self-hosted CalDAV** (Radicale/Baïkal/Nextcloud) shared by 2Do and the port | medium | **low** | Open standard (RFC 4791/5545). 2Do supports it officially. The port talks CalDAV, and the Mac/iOS apps switch their sync method to CalDAV. Recommended. |
| **B. Join the Dropbox folder** (read/write 2Do's own files) | high | medium–high | Supports an existing Dropbox setup. Format fully decoded and the writer verified (§2.3), but you must mimic 2Do's sync semantics exactly. |
| C. iCloud/CloudKit | — | — | Not usable from Linux (Apple's private container). |
| D. Todoist / Toodledo as the hub | medium | medium | Possible, but adds a third service and loses 2Do-only fields unless metadata is kept. |

## 2.2 Port-side data you need for any option

Add these per syncable row in the port's own DB (mirrors 2Do):
`sync_state` (new / clean / dirty), `deleted` (tombstone), `remote_path` (Dropbox path or CalDAV href),
`remote_rev` (Dropbox `rev` / ETag), `modified_at`, and optionally per-field modified stamps for
title/notes/tags/due/start/priority if you want field-level merges. Store one checkpoint per
account (Dropbox cursor or CalDAV sync-token per collection).

## 2.3 Option B — Dropbox protocol for the port

**Reading** — `tools/decode_2do_dropbox.py` (`unarchive`).
**Writing** — same tool, `archive(record)`: produces `gzip(NSKeyedArchiver bplist)` with
`$version 100000`, `$top.data` → `NSMutableDictionary`, strings de-duplicated, alarms as
`NSMutableArray`. **Verified against a local dataset:** decoded records round-trip identically, and Apple's
`NSKeyedUnarchiver` decodes the output into an `NSMutableDictionary` with title and alarms intact.
Dataset contents and inventory are excluded; numeric examples below are synthetic.

**Record rules when writing:**
- every value is a string, numbers formatted like 2Do does (`"0"`, `"1"`, floats with 6 decimals: `"946728000.000000"`);
- keep **all keys you read**, including ones the port doesn't use (round-trip unknown fields);
- set `entity` (`task`, `calendar`, `tag`, `calgroup`, `location`) and `uid`;
- bump `datestamp` and `datestampnorm` to now (epoch) on every edit;
- tasks: `due` (`YYYY-MM-DD` or empty) **and** `duedateabs` (that day 12:00 UTC epoch), `duetime`
  (HHMM or `999999`), `startdatetime`, `iscomp`/`compdate`/`compdatetime`, repeat fields (01 §6);
- completing a repeating task = write a **completed copy** with a new uid, `repeattype = 0`,
  `ruid = <live uid>`, and **advance the live task's dates** (01 §6).

**Sync loop for the port:**

```
1. if no cursor: list_folder("/", recursive) → full import; save cursor
   else: list_folder/continue(cursor) → changed + deleted entries
2. download changed files (download or download_zip), decode, and for each:
      local row clean        → overwrite local, remote_rev = entry.rev
      local row dirty        → conflict: newer datestamp wins (2Do behaviour), loser discarded
   deleted entries         → delete local row (unless dirty → keep and re-upload, like "stale remote delete")
3. push local dirty rows:
      path = name from (entity, list, parent, uid)
      if path changed (moved task) → delete old path
      re-check list_folder/continue(cursor) just before uploading; if the file changed → resolve first
      upload with mode = update(remote_rev) (safer than 2Do, which uses overwrite, §2.7)
      on conflict → re-download, resolve, retry
   push local tombstones → delete_batch(paths)
4. remote_rev = returned rev; sync_state = clean; purge tombstones
5. cursor = list_folder/get_latest_cursor   (only after everything succeeded)
```

**Rules to stay compatible:**
- Never run "Replace Server" from the port, and never wipe the folder.
- Don't write while a 2Do device is doing *Replace Remote*: skip the run if any `*.tsl` lock file
  exists at the folder root (§2.7).
- Throttle on Dropbox `too_many_write_operations` / `rate_limited` and retry with backoff.
- Use Dropbox's **app-folder** OAuth scope with your own app key. 2Do's own app key
  belongs to 2Do; register your own app pointing at the same folder, or use a full-Dropbox token
  scoped to `/Apps/2Do`.
- Tags inside tasks use the serialized format from [02](02-data-model.md) [inf: key `tags`].

## 2.4 Option A — CalDAV for the port

- Library: `caldav` (Python) / `tsdav` (JS) / `libical` (C) / `icalendar` (Python) for VTODO.
- Each 2Do list ↔ a calendar collection with `supported-calendar-component-set` = `VTODO`.
- Map fields per §1.7; keep `X-2DOAPP-*` properties untouched when you rewrite a VTODO (round-trip
  them) so the Mac app doesn't lose start dates, stars, task types or actions.
- Change detection: `REPORT sync-collection` with the stored sync-token; otherwise compare
  `getetag` per href. Upload with `If-Match: <etag>` (412 → re-fetch, resolve, retry). Delete with
  `DELETE` + `If-Match`.
- Repeats as `RRULE` (2Do drops unsupported parts). Completion of repeating tasks: follow 2Do's
  copy-and-advance model if the Mac app also does it under CalDAV [inf].

## 2.5 Testing checklist

1. Copy the Dropbox folder, point the port at the **copy** (or a second Dropbox account) first.
2. Create/edit/complete/delete on each side and confirm the other side shows the same result.
3. Test: moving a task between lists, completing a repeating task, an edit on both sides
   (conflict), deleting on one side while editing on the other.
4. Keep 2Do's "Backup before performing Sync" on while testing.

## 2.6 2Do task metadata format

### CalDAV: `<2Do Meta>` JSON [obs: phase-0 upload from 2Do 3.91 iOS to Synology Calendar]

Over CalDAV, `X-2DOAPP-METADATA` is **not** the token line below. 2Do writes
`<2Do Meta>` + URL-encoded JSON object + `</2Do Meta>` + an escaped newline, e.g.
`X-2DOAPP-METADATA;SHARE-SCOPE=GLOBAL:<2Do Meta>%7B%22StartDate%22%3A1927843200%2C…%7D</2Do Meta>\n`.
Every uploaded task carried it. Keys seen:

| Key | Meaning |
|---|---|
| `StartDate` | start date, epoch seconds at **UTC midnight** of the start day; key absent = no start. Present on exactly the tasks with a `.tod` `startdatetime` |
| `TaskType` | `.tod` `tasktype`: 0 task, 1 checklist, 2 project |
| `IsStarred` | 0/1 |
| `RecurrenceFrom` | seen 0 and 1 only; assumed to follow `.tod` `recurrence` (2 = from completion) [inf] |
| `RecurrenceType`, `RecurrenceValue`, `RecurrenceEndType`, `RecurrenceEndRepetitions(Orig)` | 2Do's own repeat fields (`.tod` `repeattype` / `repeatval` / …) |
| `StartDayDelay`, `TaskDuration`, `DisplayOrder`, `isExpandedToShowChildProjects` | as in `.tod` |
| `RUID`, `uid`, `parentuid` | task / parent ids |
| `duetime` | present (value 0) only on tasks whose `.tod` `duetime` is 0 |

**`DTSTART` is not the start date.** 2Do sets `DTSTART` to a copy of `DUE` on every dated task
and omits it on undated ones; the real start lives only in `StartDate`. So when this JSON is
present, read the start from it and ignore `DTSTART`. A repeating task with a start date but no
due date is uploaded **without** `RRULE` (no anchor), so its repeat is lost on the server.

### Token line [obs: decompiled `TaskNoteMetaData.makeExtendedStringForNotes`]

2Do encodes the fields that standard formats can't hold as a line of `@key:value` tokens ending in
a newline. This is used for Todoist task descriptions; it was assumed for CalDAV too
[inf: `calDAVImportHas2DoMetadata`, `syncsTaskNoteMetaData`], but the upload above shows the JSON
form instead. Twydo reads both. Tokens, in emit order:

| Token | Value |
|---|---|
| `@taskid:` | task uid |
| `@parenttaskid:` | parent uid |
| `@tasktype:` | `checklist` or `project` (absent = task) |
| `@repeat:` | repeat rule (2Do's own encoding) |
| `@repeatfromcompletion` | flag, no value |
| `@timezone:` | timezone id (absent = floating) |
| `@duedateoccurrence:` | occurrence anchor for repeating due dates |
| `@duetime:` | `HH:MM` (`%02d:%02d`) |
| `@duration:` | minutes |
| `@starred` | flag |
| `@priority:` | `high`, `medium`, `low` |
| `@startdate:` | `YYYY-MM-DD` |
| `@starttime:` | `HH:MM` |
| `@startdateoccurrence:` | occurrence anchor for repeating start dates |
| `@tags:` | comma-separated (`, `) tag names |
| `@action:` / `@actionvalue:` | action type (`call`, `message`, `mail`, `visit`, `browse`, `google`) and its value |

**Rule for the port:** treat any `X-2DOAPP-*` property (and an `@…` metadata line or `<2Do Meta>` JSON) as opaque. Parse
the fields you need, but always write back what you read unchanged, except for fields you
deliberately edit. Then the 2Do iPhone app keeps start dates, task types and repeat-from-completion.

## 2.7 Dropbox details recovered from the code (not needed for CalDAV)

**Full folder set** [obs: folder enumeration at `0x4aec1c`/`0x4b393c` lists `col`, `sec`, `tod`, `cag`, `cat`; builders for `cal`, `loc`; attachment code uses `tsm`, `tmg`, `tvn`]:

| Folder / extension | Content | File name |
|---|---|---|
| `/col/*.col` | lists, smart lists, special lists | `o_u_k_<listUID>_p__i__d_.col` [obs] |
| `/cal/*.cal` | list groups | `o_u_k__p__i_<groupUID>_d_.cal` [obs] |
| `/tod/*.tod` | tasks (alarms embedded) | `o_u_k_<listUID>_p_<parentUID>_i_<taskUID>_d_.tod` [obs] |
| `/cat/*.cat` | tags | `o_u_k__p__i_<tagUID>_d_.cat` [obs from data] |
| `/cag/*.cag` | tag groups | same pattern as tags [inf] |
| `/loc/*.loc` | locations | `o_u_k__p__i_<locationUID>_d_.loc` [obs from data] |
| `/sec/` | sections ("section files", resolved per list identity) | `o_…` pattern like the others [inf] |
| `/sec/<name>.tsm` | **task-section membership** | name must not start with `o_` and must contain no `_` [obs]; name = membership/task uid [inf] |
| `/tod/<name>.tmg` | task **image** attachment | same name rules, stored next to tasks [obs]; name = task uid [inf] |
| `/tod/<name>.tvn` | task **voice note** (audio) | same rules [obs] |

So the old-style `o_u_k_…_d_.<ext>` files hold objects, while plain `<uid>.<ext>` files in the same
folders hold the newer per-object side data (memberships, media).

**Upload write mode** [obs]: the only SwiftyDropbox `WriteMode` case 2Do imports is `.overwrite`.
`update(rev)` and `add` are never referenced. Uploads go through batched `upload_session` +
`upload_session/finish_batch_v2` with `CommitInfo(path, mode: .overwrite)`. **Conflict safety comes
entirely from 2Do's own pre-upload checks** (cursor delta + stored rev, §1.4), not from Dropbox.
A port writing to the same folder must do the same: re-check the cursor right before uploading,
because Dropbox won't reject a stale overwrite.

**Sync lock** [obs: decompiled lock creator at `0x5aada8`]:
- A file at the **root of the 2Do folder**: `/<deviceID>_<unix time, "%.6f", en_US_POSIX>.tsl`
  (e.g. `/3F2A…-…_1793460013.123456.tsl`). `deviceID` is the app's `uniqueGlobalDeviceIdentifier`;
  creation fails without one.
- Its Dropbox file properties (`FileMetaData`) carry `meta = <random UUID>`, identifying the lock instance.
- Used during **Replace Remote / Replace Server Data** ("A Dropbox Replace Remote sync lock is active").
  A port must **not write while any `*.tsl` file exists at the folder root**, and should never create one itself.
