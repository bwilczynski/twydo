# 02 — Data model

2Do keeps everything in one SQLite database. The `CREATE TABLE` statements below are 2Do's
schema as observed [obs]. Comments are mine. **In-scope** columns are marked ★;
the rest are sync-, sharing- or legacy-specific and can be dropped in the port.

Conventions [obs]: every row has `primid` (local integer PK) and `uid` (stable string ID, used
for all cross-references). Dates are stored as `DOUBLE`; the Dropbox sync files use **Unix
epoch seconds** and date-only values at 12:00 UTC (see [05](05-migrating-your-data.md)), and the
database very likely does the same [inf]. The sync files are the decoded, verified view of
this model; prefer them for import.
`isdeleted` is a soft delete (tombstone for sync); `syncstatus` / `modduringsync` /
`changed_properties` / `modguid` / `webdav_rev` / `*_uid` are sync bookkeeping.

## tasks

```sql
CREATE TABLE IF NOT EXISTS tasks (
  primid INTEGER PRIMARY KEY NOT NULL,
  uid VARCHAR(255) NOT NULL,                 -- ★ stable id
  calendaruid VARCHAR(255) NOT NULL,         -- ★ list uid (calendars.uid)
  url TEXT,
  title TEXT NOT NULL,                       -- ★
  notes TEXT,                                -- ★ markdown
  datestamp DOUBLE NOT NULL DEFAULT 0,       -- ★ last modified
  duedate DOUBLE DEFAULT 0,                  -- ★ 0 = none
  priority INTEGER NOT NULL DEFAULT 0,       -- ★ 0 none,1 low,2 med,3 high [inf]
  iscompleted INTEGER NOT NULL DEFAULT 0,    -- ★
  completeddate INTEGER DEFAULT 0,           -- ★
  isdeleted INTEGER NOT NULL DEFAULT 0,
  canberestored INTEGER NOT NULL DEFAULT 0,
  wassent INTEGER NOT NULL DEFAULT 0,
  wasreceived INTEGER NOT NULL DEFAULT 0,
  haveregistered INTEGER NOT NULL DEFAULT 0,
  syncstatus INTEGER NOT NULL DEFAULT 1,
  tags TEXT,                                 -- ★ serialized tag list, see "Tag string format" below [obs]
  actionvalue TEXT,                          -- action payload (phone/email/url)
  actiontype INTEGER NOT NULL DEFAULT -1,    -- -1 none
  displayorder INTEGER NOT NULL DEFAULT 0,   -- ★ manual sort order
  recurrence INTEGER NOT NULL DEFAULT 0,     -- ★ recurrenceFrom: 2 = from completion, else from due [obs]
  repeatvalue INTEGER NOT NULL DEFAULT 0,    -- ★ recurrenceValue: interval N or ordinal 1–5 [obs]
  repeattype INTEGER NOT NULL DEFAULT 0,     -- ★ recurrenceType: 0 none, see 01 §6 table [obs]
  parent VARCHAR(255),                       -- ★ parent task uid (project/checklist)
  tasktype INTEGER NOT NULL DEFAULT 0,       -- ★ 0 task, 1 checklist, 2 project [obs]
  imagepath TEXT,
  alarmcount INTEGER NOT NULL DEFAULT 0,
  toodledo_uid VARCHAR(255),
  outlook_uid VARCHAR(255),
  startdate DOUBLE DEFAULT 0,                -- ★ 0 = none
  duetime DOUBLE DEFAULT 999999,             -- ★ HHMM integer (930 = 9:30, 1430 = 14:30, 0–2359, 0 = 00:00); 999999 or -1 = no time [obs]
  archived INTEGER DEFAULT 0,
  isbirthday INTEGER DEFAULT 0,
  lastuid VARCHAR(255),                      -- previous occurrence of a repeating task [inf]
  sortfield INTEGER DEFAULT 0,               -- ★ project's own sort field
  sortorder INTEGER DEFAULT 0,               -- ★ project's own sort direction
  toodledo_parentuid VARCHAR(255),
  isanniversary INTEGER NOT NULL DEFAULT 0,
  hasaudionote INTEGER NOT NULL DEFAULT 0,
  hasimage INTEGER NOT NULL DEFAULT 0,
  starred INTEGER NOT NULL DEFAULT 0,        -- ★
  locations TEXT,
  mobileme_uid VARCHAR(255),
  mobileme_etag VARCHAR(255),
  mobileme_starchanged INTEGER NOT NULL DEFAULT 0,
  recurrenceendtype INTEGER NOT NULL DEFAULT 0,      -- ★ 0 never, 1 on date, 2 after N [obs]
  recurrenceendrepeats INTEGER NOT NULL DEFAULT 0,   -- ★ remaining repeats
  recurrenceenddate DOUBLE NOT NULL DEFAULT 0,       -- ★
  recurrenceendrepeatsorig INTEGER NOT NULL DEFAULT 0,
  toodledo_location VARCHAR(255),
  startdaydelay INTEGER NOT NULL DEFAULT 0,  -- ★ start offset for repeats [inf]
  creationstamp DOUBLE NOT NULL DEFAULT 0,   -- ★ created at
  modduringsync INTEGER NOT NULL DEFAULT 0,
  entitieschanged VARCHAR(255),
  locationalerttype INTEGER NOT NULL DEFAULT 0,
  iscollapsed INTEGER NOT NULL DEFAULT 0,    -- ★ UI state
  isexpanded INTEGER NOT NULL DEFAULT 0,     -- ★ UI state
  ruid VARCHAR(200),
  modstamptitle DOUBLE NOT NULL DEFAULT 0,   -- per-field modified stamps (merge)
  webdav_rev VARCHAR(255),
  modstamptags DOUBLE NOT NULL DEFAULT 0,
  modstampnotes DOUBLE NOT NULL DEFAULT 0,
  modstampdue DOUBLE NOT NULL DEFAULT 0,
  modstampstart DOUBLE NOT NULL DEFAULT 0,
  modstamppriority DOUBLE NOT NULL DEFAULT 0,
  modstampaudio DOUBLE NOT NULL DEFAULT 0,
  modstamppicture DOUBLE NOT NULL DEFAULT 0,
  modstampalarm DOUBLE NOT NULL DEFAULT 0,
  duration INTEGER NOT NULL DEFAULT 0,       -- ★ minutes [inf]
  modguid VARCHAR(255),
  changed_properties TEXT NOT NULL DEFAULT '',
  needsIndexing INT NOT NULL DEFAULT 0,
  timezoneid VARCHAR(100)                    -- ★ floating vs zoned dates
)
```

Later schema versions also add `tasksectionuid`-style columns (section membership is synced). [inf: sync log
"section membership changed"]. Check the backup's real `tasks` columns.

## calendars (= lists, smart lists and built-in focus lists)

One table holds regular lists, smart lists and the special lists. [obs]

```sql
CREATE TABLE IF NOT EXISTS calendars (
  primid INTEGER PRIMARY KEY NOT NULL,
  redcolor DOUBLE DEFAULT 0,                 -- ★ list colour (0..1)
  greencolor DOUBLE DEFAULT 0,               -- ★
  bluecolor DOUBLE DEFAULT 0,                -- ★
  notes TEXT,
  oldtitle TEXT,
  title TEXT,                                -- ★ list name
  type VARCHAR (100),
  uid VARCHAR(255) NOT NULL,                 -- ★
  iseditable INTEGER DEFAULT 1,
  colorid INTEGER DEFAULT 0,                 -- ★ palette index
  displayorder INTEGER DEFAULT 0,            -- ★ sidebar order
  canshowinall INTEGER NOT NULL DEFAULT 1,   -- ★ include in All
  canshowintoday INTEGER NOT NULL DEFAULT 1, -- ★ include in Today
  tasksortfield INTEGER DEFAULT 1,           -- ★ sort code: 0 Status,1 Due,2 Note,4 Priority,5 List,6 Title,7 Manual,9 Tag,10 Location,11 Start,13 Creation,14 Duration,15 Modified [obs]
  tasksortorder INTEGER DEFAULT 0,           -- ★ asc/desc
  isdeleted INTEGER NOT NULL DEFAULT 0,
  canberestored INTEGER NOT NULL DEFAULT 0,
  datestamp DOUBLE NOT NULL DEFAULT 0,
  syncstatus INTEGER NOT NULL DEFAULT 1,
  ishidden INTEGER NOT NULL DEFAULT 0,       -- ★
  isshowingall INTEGER NOT NULL DEFAULT 1,
  password INTEGER NOT NULL DEFAULT 0,       -- list passcode lock
  autotweet INTEGER NOT NULL DEFAULT 0,
  smartsearch VARCHAR(255),                  -- ★ smart list query string
  smartsearchsoundex INTEGER NOT NULL DEFAULT 0, -- ★ fuzzy match
  showindone INTEGER NOT NULL DEFAULT 1,     -- ★
  toodledo_uid VARCHAR(255),
  outlook_uid VARCHAR(255),
  isspecialfolder INTEGER NOT NULL DEFAULT 0,
  isbirthday INTEGER NOT NULL DEFAULT 0,
  smartrangefrom DOUBLE NOT NULL DEFAULT 0,  -- ★ smart list date range
  smartrangeto DOUBLE NOT NULL DEFAULT 0,    -- ★
  notsyncing INTEGER NOT NULL DEFAULT 0,
  isanniversary INTEGER NOT NULL DEFAULT 0,
  starred INTEGER NOT NULL DEFAULT 0,        -- ★ = Starred focus list
  donecal INTEGER NOT NULL DEFAULT 0,        -- ★ = Done focus list
  calendartab INTEGER NOT NULL DEFAULT 0,
  smartcaluid VARCHAR(255),
  mobileme_uid VARCHAR(255),
  mobileme_etag VARCHAR(255),
  mobileme_publishurl VARCHAR(255),
  mobileme_isshared INTEGER NOT NULL DEFAULT 0,
  isscheduled INTEGER NOT NULL DEFAULT 0,    -- ★ = Scheduled focus list
  modduringsync INTEGER NOT NULL DEFAULT 0,
  isallcal INTEGER NOT NULL DEFAULT 0,       -- ★ = All focus list
  istodaycal INTEGER NOT NULL DEFAULT 0,     -- ★ = Today focus list
  excludecompleted INTEGER NOT NULL DEFAULT 0, -- ★ smart list option
  combineresults INTEGER NOT NULL DEFAULT 0,   -- ★ smart list option
  focusfilterstring VARCHAR(255),            -- ★ per-list focus filter
  parentuid VARCHAR(100),                    -- ★ list group uid
  parentname VARCHAR(250),                   -- ★ list group name (denormalised)
  isexpanded INTEGER NOT NULL DEFAULT 0,     -- ★ sidebar state
  isarchived INTEGER NOT NULL DEFAULT 0,     -- ★
  webdav_rev VARCHAR(255),
  isinboxcal INTEGER NOT NULL DEFAULT 0,     -- ★ = Inbox
  caldav_aftersync VARCHAR(255),
  caldav_synctoken VARCHAR(255),
  caldav_usessynctoken INTEGER NOT NULL DEFAULT 0,
  caldav_familyshared INTEGER NOT NULL DEFAULT 0,
  caldav_imagemanageduid VARCHAR(255),
  caldav_audiomanageduid VARCHAR(255),
  modguid VARCHAR(255),
  smartrangeexcludesstart INTEGER NOT NULL DEFAULT 0,
  changed_properties TEXT NOT NULL DEFAULT '',
  mobileme_issharedowner INTEGER NOT NULL DEFAULT 0
)
```

## calgroups (list groups)

```sql
CREATE TABLE IF NOT EXISTS calgroups (primid INTEGER PRIMARY KEY NOT NULL, uid VARCHAR(100),
  groupname VARCHAR(255) NOT NULL, displayorder INTEGER NOT NULL DEFAULT 0,
  isdeleted INTEGER NOT NULL DEFAULT 0, syncstatus INTEGER NOT NULL DEFAULT 1,
  modduringsync INTEGER NOT NULL DEFAULT 0)
```

## tasksections

```sql
CREATE TABLE IF NOT EXISTS tasksections (
  primid INTEGER PRIMARY KEY NOT NULL,
  uid VARCHAR(255) NOT NULL,                 -- ★
  calendaruid VARCHAR(255) NOT NULL,         -- ★ owning list
  sectionname VARCHAR(255) NOT NULL DEFAULT '', -- ★
  sectiondescription TEXT,
  isremindersimportsection INTEGER NOT NULL DEFAULT 0,
  displayorder INTEGER NOT NULL DEFAULT 0,   -- ★
  isdeleted INTEGER NOT NULL DEFAULT 0,
  syncstatus INTEGER NOT NULL DEFAULT 1,
  modguid VARCHAR(255),
  changed_properties TEXT NOT NULL DEFAULT '',
  datestamp DOUBLE NOT NULL DEFAULT 0,
  remoteuid VARCHAR(255), synchref TEXT, remotetoken VARCHAR(255),
  remotemetadata TEXT, needsparentcheck INTEGER NOT NULL DEFAULT 0,
  remotecoltoken VARCHAR(255)
)
```

## tags / taggroups

```sql
CREATE TABLE IF NOT EXISTS tags (
  primid INTEGER PRIMARY KEY NOT NULL,
  tag VARCHAR(255) NOT NULL,                 -- ★ name
  type INTEGER NOT NULL DEFAULT 0,
  displayorder INTEGER NOT NULL DEFAULT 0,   -- ★
  meta VARCHAR(255),
  isdeleted INTEGER NOT NULL DEFAULT 0,
  outlook_uid VARCHAR(255),
  syncstatus INTEGER NOT NULL DEFAULT 1,
  modduringsync INTEGER NOT NULL DEFAULT 0,
  uid VARCHAR(100),                          -- ★
  parentuid VARCHAR(100),                    -- ★ tag group uid
  parentname VARCHAR(250),
  webdav_rev VARCHAR(255),
  modguid VARCHAR(255),
  isheld INTEGER NOT NULL DEFAULT 0,         -- ★ paused tag [inf]
  changed_properties TEXT NOT NULL DEFAULT ''
)
CREATE TABLE IF NOT EXISTS taggroups (primid INTEGER PRIMARY KEY NOT NULL, uid VARCHAR(100),
  groupname VARCHAR(255) NOT NULL, displayorder INTEGER NOT NULL DEFAULT 0,
  isdeleted INTEGER NOT NULL DEFAULT 0, syncstatus INTEGER NOT NULL DEFAULT 1,
  modduringsync INTEGER NOT NULL DEFAULT 0)
```

### Tag string format (`tasks.tags`) [obs]

Tasks embed their tags in **one string**, not a join table:

```
tag1 +$$|$$+ tag2 +$$|$$+ …
each tag = title ~|$$@$$|~ 0 ~|$$@$$|~ metaData ~|$$@$$|~ primid ~|$$@$$|~ uid ~|$$@$$|~ tagGroupUid ~|$$@$$|~ tagGroupName ~|$$@$$|~ isHeld(1/0)
```

Format string `%@%@0%@%@%@%ld%@%@%@%@%@%@%@%@`, entry separator `+$$|$$+`, field separator `~|$$@$$|~`.
Missing metadata or group values are empty. Tags titled `format(s)`, `?` or `~`, empty titles and duplicates (by catalog name key) are skipped. To read it: split on `+$$|$$+`, then on `~|$$@$$|~`, and take field 0 as the tag name.

### Other sentinels [obs]
- `6406192800` is used as a "no date" placeholder in date comparisons; values below `120.0` count as unset.
- In date SQL, `parenttasktype = 1` (checklist) makes child items use the parent's due/start dates, which confirms **checklist = 1, project = 2**.

## alarms

```sql
CREATE TABLE IF NOT EXISTS alarms (
  primid INTEGER PRIMARY KEY NOT NULL,
  uid VARCHAR(255) NOT NULL,                 -- ★
  taskuid VARCHAR(255) NOT NULL,             -- ★
  action TEXT NOT NULL,                      -- ★ display / sound / email / procedure (iCal VALARM style)
  sound TEXT,                                -- ★
  emailaddress VARCHAR (150),
  url TEXT,
  relativetrigger DOUBLE DEFAULT 0,          -- ★ seconds offset [inf]
  datestamp DOUBLE NOT NULL DEFAULT 0,
  absolutetrigger DOUBLE DEFAULT 0,          -- ★ absolute time
  isdeleted INTEGER NOT NULL DEFAULT 0,
  canberestored INTEGER NOT NULL DEFAULT 0,
  syncstatus INTEGER NOT NULL DEFAULT 1,
  toodledo_uid VARCHAR(255),
  outlook_uid VARCHAR(255),
  isbirthday INTEGER NOT NULL DEFAULT 0,
  isanniversary INTEGER NOT NULL DEFAULT 0,
  snooze INTEGER NOT NULL DEFAULT 0,         -- ★
  modduringsync INTEGER NOT NULL DEFAULT 0,
  isauto INTEGER NOT NULL DEFAULT 0,         -- ★ default alert
  relduetime INTEGER NOT NULL DEFAULT 0,     -- ★ relative to due time vs due day [inf]
  modguid VARCHAR(255)
)
```

Alarm action names: `CalAlarmActionDisplay`, `CalAlarmActionSound`, `CalAlarmActionEmail`, `CalAlarmActionProcedure`. [obs]

## Other tables (out of scope)

`locations`, `images`, `audionotes`, `taskattachments`, `emailaccounts`, `emailscaptured`,
`secureurls`, `syncdata`, `syncmethod`, `unsyncedtasks`, `dbversion`, all `shared*` tables
(iCloud/LAN sharing, Automerge CRDT state).

## Suggested port schema (simplified)

```
list(id, name, color, group_id, sort_field, sort_dir, focus_filter, in_all, in_today, archived, is_inbox, position)
list_group(id, name, position, expanded)
smart_list(id, name, color, query, range_from, range_to, exclude_completed, position)
section(id, list_id, name, position, collapsed)
task(id, list_id, parent_id, section_id, type{task,project,checklist}, title, notes,
     due_date, due_time?, start_at?, duration_min, priority 0..3, starred,
     rrule?, repeat_from_completion, repeat_end{never,count,until},
     completed_at?, created_at, modified_at, position, collapsed)
tag(id, name, group_id, paused, position) ; task_tag(task_id, tag_id)
alert(id, task_id, kind{absolute,relative}, at?, offset_sec?, snooze_until?)
```

Store due dates as **local floating dates** (date + optional time without timezone), like 2Do's
default, so "due Tuesday" stays Tuesday when you travel. [inf from `timezoneid` being optional]
