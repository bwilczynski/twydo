# Phase-0 gate checklist

This is the checklist for the compatibility gate (spec section 10, step 0). Plan 4 (writable core)
cannot start until the gate outputs listed at the end exist.

- 👤 = you (a human action on your NAS, Mac or iPhone). 🤖 = the agent, using the tools in this repo.
- **Stop and ask** before anything that touches production 2Do data, the production NAS calendar or
  the production iPhone. Everything below uses a test account, a test 2Do setup and synthetic data.
- Keep every output **outside the repository**. This checklist uses `~/twydo-phase0/`:

```sh
export P0=~/twydo-phase0
mkdir -p "$P0"/{backup,inventory,vdir,staging,reports}
```

- Use **synthetic titles only** (for example `Test monthly 15th`). Never put real tasks, people,
  places or URLs into the test account.
- Run the 🤖 commands from the repository root, so `cargo run -p tdo` and `compat/matrix.toml` resolve.

## Gate outputs (checked at the end)

1. Committed fixtures in `fixtures/phase0/v1/`.
2. A filled `compat/matrix.toml`, including the `[environment]` versions.
3. A recorded vdirsyncer conflict behaviour, delete-versus-edit included.
4. The metadata precedence decision (spec section 6): which representation wins when two
   representations of the same fact disagree.

## Step 1. Back up and inventory

1. 👤 In 2Do on the Mac: Settings > Backups, make a backup. Copy the Dropbox folder `Apps/2Do` to
   `$P0/backup/2Do/` (a copy, never move it).
2. 🤖 Make the inventory:

   ```sh
   python3 tools/decode_2do_dropbox.py "$P0/backup/2Do" "$P0/inventory/inventory.2do.json"
   ```

   Output: `$P0/inventory/inventory.2do.json` (counts and errors print to the terminal). Contains
   personal data, so it never enters the repository.

## Step 2. Isolated CalDAV test account on the NAS

1. 👤 On the Synology NAS, create a **dedicated test user** and an empty calendar/task collection for
   it. Do not reuse your production account or calendar. Note the CalDAV URL (it goes only into
   `~/.config/vdirsyncer/config`).
2. 👤 Confirm Synology Calendar accepts `VTODO`: create a task from 2Do against this account (step 3
   must be done first) and see it arrive on the server. If it does not, run Radicale in a container
   instead and use that URL.
3. 👤 Record the server name and version (and the Calendar package version) in `compat/matrix.toml`
   `[environment]` (`server`, `server_version`).

Outputs: test account, empty collection, server name/version.

## Step 3. Restorable 2Do test setup

1. 👤 Prepare a setup where switching providers cannot replace production data: a **separate macOS
   user** (or a restored backup) running 2Do, and an **iPhone 2Do profile not connected to production
   data** (a device or profile with no Dropbox/production sync).
2. 👤 Record the app versions: Mac 2Do, iPhone 2Do, iOS and macOS. Put them in `compat/matrix.toml`
   `[environment]` (`mac_2do`, `iphone_2do`, `ios`, `macos`).
3. 👤 Check that the production Dropbox sync is **off** in the test setup before continuing.

Outputs: test setup, environment versions.

## Step 4. Upload to the test account and pull with vdirsyncer

1. 👤 In the test 2Do on the Mac, restore the backed-up data. Add the test CalDAV account and push
   the data with 2Do's upload flow. Use *Replace Server Data* **only** on the empty test collection.
2. 🤖 Install vdirsyncer and set it up outside the repo:

   ```sh
   mkdir -p ~/.config/vdirsyncer
   cp contrib/vdirsyncer/config.example ~/.config/vdirsyncer/config
   $EDITOR ~/.config/vdirsyncer/config   # URL, USERNAME, password.fetch command, vdir path = $P0/vdir/
   vdirsyncer --version                  # record in compat/matrix.toml [environment] vdirsyncer
   vdirsyncer discover twydo
   vdirsyncer sync twydo
   vdirsyncer metasync twydo
   ```

   Output: `$P0/vdir/<collection>/` with `*.ics`, `displayname`, `color`.

## Step 5. First inventory and gate report

🤖 Read-only runs against the test vdir:

```sh
cargo run -q -p tdo -- --vdir "$P0/vdir" export-json > "$P0/reports/export.json"
cargo run -q -p tdo -- --vdir "$P0/vdir" check --verbose > "$P0/reports/check.txt"; echo "check exit: $?"
python3 tools/reconcile.py --tz America/New_York "$P0/inventory/inventory.2do.json" "$P0/reports/export.json" > "$P0/reports/reconcile.txt"
```

- Use your real IANA zone in `--tz`. Add `--config ~/.config/twydo/config.toml` if list archive
  preferences are configured. Add `--show-titles` only for local reading, never into a committed file.
- `tdo check` reports which active repeat-rule classes are verified in `compat/matrix.toml`. At this
  point most are `unverified`, which is expected.
- Output: `$P0/reports/{export.json,check.txt,reconcile.txt}` (first gate report). Explain any
  special-list count differences instead of expecting each Dropbox list to become a collection.
- Acceptance cases P1, P2 and L1 are checked from your private acceptance file, not from this repo.

## Step 6. Fixture scenarios (👤 + 🤖)

Do these on the **test setup only**, with synthetic titles. After **every** step, sync, then capture.

```sh
export COLL="$P0/vdir/<collection>"      # the test collection directory
vdirsyncer sync twydo && vdirsyncer metasync twydo
python3 tools/capture_fixture.py capture "$COLL" "$P0/staging" \
  --scenario <scenario-id> --step <NN-slug> --device mac|iphone|twydo --action "<what was done>"
```

- `--device` is where the action happened. Use `twydo` only for edits made by Twydo (none exist in
  Plan 1, so the `twydo_*` matrix columns stay `unverified` for now).
- Step names sort lexically: `01-create`, `02-complete`, and so on. `changes.diff` compares against
  the previous step of the same scenario.
- Use a fresh collection state or distinct titles per scenario, so diffs stay readable.
- Naming: scenario id = the matrix `case` or `id`, lower-case with hyphens.

In the sequences below, "Mac" and "phone" mean the test 2Do on each device, "sync" means wait for
2Do to sync and then run the vdirsyncer sync above, and "capture" means the capture command with the
next `NN-slug`. Each operation scenario covers the six matrix ops `create, edit, complete, reopen,
move, delete` for its `case`; record the results in the `mac_to_iphone` / `iphone_to_mac` columns.

### 6a. Operation scenarios (one per `[[operation]]` case)

1. **one-time** (`one-time`). Create `Test one-time` on Mac, sync, capture. Phone: sync, check it
   arrived, edit the title, sync, capture. Phone: complete, sync, capture. Mac: reopen, sync, capture.
   Mac: move it to a second test list, sync, capture (the move shows as a new collection folder; capture
   both collections). Phone: delete, sync, capture. Then repeat create/edit with the roles swapped
   (create on phone, edit on Mac).
2. **monthly** (`monthly`). Create `Test monthly` repeating monthly on Mac, sync, capture. Phone: edit
   notes, sync, capture. Phone: complete (note whether it spawns a copy or advances the date), sync,
   capture. Mac: reopen/undo, sync, capture. Move to the second list, sync, capture. Delete on Mac,
   sync, capture.
3. **yearly** (`yearly`). Same sequence as monthly with `Test yearly` repeating yearly.
4. **from-completion** (`from-completion`). Create `Test from-completion` repeating "from completion"
   (every 1 week) with a due date in the past, sync, capture. Edit, complete on the phone, reopen, move,
   delete as in scenario 2, with a sync and a capture after each.
5. **recurring-project** (`recurring-project`). Create project `Test recurring project`, repeating
   monthly, with two children that have their own dates, sync, capture. Edit a child on the phone, sync,
   capture. Complete the project on Mac with one child unfinished (note the confirmation), sync, capture.
   Reopen, move the project, delete it; sync and capture after each.
6. **checklist** (`checklist`). Create `Test checklist` with three items, sync, capture. Edit one item
   on the phone, sync, capture. Complete one item, then the parent, sync and capture after each.
   Reopen, move, delete; sync and capture after each.
7. **project-recurring-child** (`project-recurring-child`). Create non-recurring project `Test project`
   with a child `Test child` that repeats weekly, sync, capture. Edit the child, complete the child on the
   phone, complete the project on Mac, reopen, move, delete; sync and capture after each.
8. **date-only** (`date-only`). Create `Test date-only` due a date with no time, sync, capture. Edit the
   date on the phone, complete, reopen, move, delete; sync and capture after each.
9. **timed** (`timed`). Create `Test timed` due a date with a time (note the zone in `--action`), sync,
   capture. Edit the time on the phone, complete, reopen, move, delete; sync and capture after each.
10. **start-date** (`start-date`). Create `Test start-date` with a start and a due date, sync, capture.
    Edit the start date on the phone, complete, reopen, move, delete; sync and capture after each.
11. **alarms** (`alarms`). Create `Test alarms` with one relative alarm, one absolute alarm and a second
    relative alarm, sync, capture. Edit an alarm on the phone, complete, reopen, move, delete; sync and
    capture after each.

### 6b. Edge cases (one per `[[edge_case]]` id)

12. **monthly-15th-three-months-late** (`monthly-15th-three-months-late`). Create on Mac `Test 15th late`
    repeating monthly, due the 15th of a month three months in the past, sync, capture. Complete on the
    phone, sync, capture (advanced once, or missed dates skipped? history retained?). Complete again,
    sync, capture. Edit on Mac, sync, capture.
13. **monthly-31st** (`monthly-31st`). Create `Test 31st` monthly due the 31st (start in a 31-day month).
    Complete it repeatedly across short months (capture after each) to see skip versus clamp and whether
    the original anchor is preserved.
14. **yearly-feb-29** (`yearly-feb-29`). Create `Test feb 29` yearly due 29 February (leap year). Complete,
    sync, capture; complete again to cross a non-leap year, capture.
15. **missing-fifth-weekday** (`missing-fifth-weekday`). Create `Test fifth` monthly on the fifth
    Wednesday (or the last Wednesday if 2Do offers it, and record which). Complete across months without a
    fifth occurrence, capture after each.
16. **from-completion-anchor** (`from-completion-anchor`). Create `Test anchor` from-completion, date-only,
    and a second `Test anchor timed` with a time. Complete each late on the phone, then on Mac, with a
    sync and capture after each (completion versus start anchor, date-only versus timed, timezone).
17. **end-after-n** (`end-after-n`). Create `Test end n` repeating weekly, ending after 3 occurrences.
    Complete it four times, capture after each (remaining versus original count; final occurrence state).
18. **end-on-date** (`end-on-date`). Create `Test end date` repeating weekly, ending on a date two weeks
    out. Complete until and past the end date, capture after each (inclusive boundary; final state).
19. **parent-mixed-children** (`parent-mixed-children`). Create project `Test mixed` repeating monthly with
    two completed and two open children. Complete the parent, sync, capture (confirmation, copied child
    identities and links, reset and date-shift rules).
20. **project-child-own-recurrence** (`project-child-own-recurrence`). Create recurring project
    `Test own rec` with one child repeating weekly. Complete the child, then the parent, sync and capture
    after each (no double advancement).
21. **start-due-alarm-offsets** (`start-due-alarm-offsets`). Create `Test offsets` repeating weekly with a
    start date, a due date, a relative alarm and an absolute alarm. Complete once, sync, capture (which
    dates and triggers shift, which stay).
22. **reopen-undo-before-after-sync** (`reopen-undo-before-after-sync`). On a recurring task, complete on
    Mac and undo **before** syncing, then sync, capture. Complete again, sync, capture, then reopen
    **after** the sync, sync, capture. Compare history, live dates, child state and repeat counts.

### 6c. Repeat-rule classes (one per `[[rule]]` class)

23. For each class in `compat/matrix.toml` (`days`, `weeks`, `weekdays-set`, `months-day-1-28`,
    `months-day-29-31`, `months-last-day`, `months-nth-weekday`, `months-nth-workday`,
    `months-nth-weekend-day`, `years`, `years-feb-29`): create `Test rule <class>` on Mac with that rule,
    sync, capture (**preserve**: is the rule carried unchanged?). Complete it on the phone, sync, capture
    (**execute**). Edit the rule on the phone, sync, capture (**edit**). Repeat for each modifier row:
    `mod:from-completion`, `mod:count`, `mod:until`. `unrecognized` stays `unsupported`. Scenario id:
    `rule-<class>`.

### 6d. Other §6 fixture cases

24. **completed-history** (`completed-history`). Complete a recurring task three times on the phone, sync,
    capture after each: the completed copies must be visible as history in the vdir.
25. **unknown-properties** (`unknown-properties`). On Mac set every 2Do field you can on `Test unknown`
    (tags, flag/star, priority, notes, location text, URL, contact) with synthetic values. Sync, capture.
    The goal is to see `X-2DOAPP-*` and other properties we must preserve unchanged. Edit the title on the
    phone, sync, capture.
26. **metadata** (`metadata`). Rename the test list and change its colour on Mac, sync, metasync, capture.
    Do the same from the phone. Record which representation wins when the two disagree (the metadata
    precedence decision, spec section 6).

### 6e. Conflict scenarios (record vdirsyncer's default behaviour; spec sections 5 and 9)

Make the two edits **while offline or before either side syncs**, then sync Mac first, then the phone,
then run `vdirsyncer sync twydo` and capture the vdir **and** save its output
(`vdirsyncer sync twydo 2>&1 | tee "$P0/reports/conflict-<id>.txt"`). Use scenario ids `conflict-*`.

27. **conflict-concurrent-completion** (`conflict-concurrent-completion`). Create a recurring task and
    sync everywhere. Turn network off on both devices. Complete it on Mac and on the phone. Reconnect,
    sync Mac, then phone, then vdirsyncer. Record: error text, which side's version is in the vdir, whether
    the other side is lost, whether other items keep syncing.
28. **conflict-completion-vs-edit** (`conflict-completion-vs-edit`). Same set-up. On Mac complete the task;
    on the phone, edit its notes. Reconnect and sync, then record the same four facts.
29. **conflict-delete-vs-edit** (`conflict-delete-vs-edit`). Same set-up. On Mac delete the task; on the
    phone edit its title. Reconnect and sync, then record: does the deletion win, does the edit resurrect
    the task, is anything silently discarded, is an error shown? Run it once more with the roles swapped.

For each conflict, note in `compat/matrix.toml` `notes` (and in the committed fixtures' `step.toml`
actions) whether vdirsyncer's default meets the spec: both versions preserved and other resources still
syncing. If it does not, that is the expected input for Plan 3's conflict store.

## Step 7. Privacy scan and commit fixtures

1. 🤖 Scan the staging directory:

   ```sh
   python3 tools/capture_fixture.py scan "$P0/staging"
   ```

   It exits 1 if it flags personal data. It also prints every SUMMARY, DESCRIPTION, COMMENT and
   CATEGORIES value (the SUMMARY list) for review.
2. 👤 Read that SUMMARY list and confirm that every line is synthetic. Fix or delete anything that is
   not, and re-run the scan.
3. 🤖 Only after the scan passes and you confirm:

   ```sh
   mkdir -p fixtures/phase0/v1
   cp -R "$P0/staging/." fixtures/phase0/v1/
   python3 tools/capture_fixture.py scan fixtures/phase0/v1
   git diff --stat -- fixtures/phase0/v1
   git add fixtures/phase0/v1
   git diff --staged --stat
   git commit -m "test: phase-0 compatibility fixtures v1"
   ```

## Step 8. Fill in the matrix

1. 🤖 From the diffs and `changes.diff` files, fill `compat/matrix.toml`: `[environment]` versions, every
   `[[rule]]`, `[[operation]]` and `[[edge_case]]` verdict (`pass`, `fail`, `unsupported`), the `evidence`
   paths (fixture step directories) and `notes`. Record the conflict findings and the metadata
   precedence decision.
2. Cases that could not be resolved stay `unverified`, each with a note saying why.
3. 🤖 Re-run `cargo run -q -p tdo -- check --matrix compat/matrix.toml --vdir "$P0/vdir"` to confirm the
   matrix parses and see which of your active rule classes are now verified.
4. 👤 Review the matrix and the conflict notes. Approve before Plan 4 starts.
