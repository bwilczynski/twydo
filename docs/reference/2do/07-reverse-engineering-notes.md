# 2Do for macOS — reverse-engineering notes

Target: `/Applications/2Do.app`, version 2026.4.6 (build 202640600),
bundle id `com.guidedways.TodoMac`, by Beehive Innovations.
Analysis: REA + Ghidra 12.1.4 on the thinned arm64 slice
(sha256 `f6b949f3ed69bb668fb372c3eca2aacdfaf61532e6122ae38b6549fcae6c59fa`),
plus `otool`/`strings` for the 92 MB `TodoDAOFramework`.
Everything here is static analysis; the app was not run.

Legend: **[obs]** seen directly in code or data · **[inf]** inferred · **[?]** unresolved

## 1. Bundle layout

| Component | What it is |
|---|---|
| `MacOS/2Do` | Universal (arm64 + x86_64) main app. ObjC app delegate (`TDMAppDelegate*.m`) + Swift UI (`TodoWC`, `TDM*VC`). Exported symbols stripped; ObjC/Swift metadata intact (~1,080 class names, ~160 `TDM*`). [obs] |
| `Frameworks/TodoDAOFramework` | 92 MB. Data model, SQLite schema, all sync engines, automation executor, embedded Rust (Automerge). [obs] |
| 30 more frameworks | In-house Beehive UI kits, plus GRDB, Dropbox SDK, AppAuth/GTMAppAuth, DAVKit, CocoaLumberjack, SwiftyChrono, ZipArchive, ShortcutRecorder, PTHotKey. [obs] |
| `Library/LoginItems/…TodoMac.Helper.app` | Background helper with no Dock icon (`LSUIElement`), talks to the app over XPC. Runs launch-at-login and the global hotkey listener. [obs/inf] |
| `PlugIns/Share.appex`, `WidgetsExtension.appex` | Share extension and WidgetKit widgets. [obs] |
| `Metadata.appintents`, `2Do.sdef` | App Intents / Shortcuts and AppleScript dictionary. [obs] |

Third-party telemetry: Firebase Analytics + Crashlytics (Fabric key in Info.plist). [obs]
`NSAllowsArbitraryLoads = true` (App Transport Security disabled, needed for user-supplied CalDAV/WebDAV servers). [obs]

## 2. URL schemes and IPC surface

- URL schemes: `x-2do`, `twodo` (`twodo://x-callback-url/showtask?uid=…`, `showcollection?collection_id=…`), `twodolog`, `twodologshow`, `twodo-oauth2`, Dropbox (`db-<2Do app key>`). [obs]
- Services menu: "New 2Do: Selection as Title/Note" (`quickEntry`). [obs]
- Bonjour: `_2do-list._tcp` (list sharing over Wi-Fi), `_2do-sync._tcp` (change hints between devices). [obs]
- Handoff activity types: `com.guidedways.2Do.{SceneState,SelectedList,TaskEditing,NewTask}`. [obs]
- **Automation socket** for MCP (section 4).

## 3. Data and sync

SQLite tables (from `CREATE TABLE` literals in TodoDAOFramework) [obs]:
`tasks, tasksections, tags, taggroups, alarms, locations, images, audionotes,
taskattachments, calendars, calgroups, emailaccounts, emailscaptured, secureurls,
syncdata, syncmethod, unsyncedtasks, dbversion`, and for shared lists:
`sharedlists, sharedlistmirrors, sharedfeedentry, sharedlocalmetadata,
sharedobjectidentitymap, sharedpendingchange, sharedrecentconnecteddevice,
sharedsubscriptionstate, sharedtaskmergestate`.

Sync backends (one `*SyncSettings` class each) [obs]: CloudKit (iCloud), CalDAV,
WebDAV, Dropbox, Reminders (EventKit), Todoist, Toodledo, local filesystem, local, in-memory.
`SyncEngine` / `SyncEngineCoordinator` run in a separate process managed by
`TDMSyncProcessHost` (includes a server-trust prompt, `TDMSyncProcessTrustRequest`). [obs]

**Shared-list merging uses Automerge 0.7.2** (Rust CRDT), compiled into TodoDAOFramework
with `uniffi_core 0.28.2` bindings (built by a different developer's Rust nightly-2025-11-18 toolchain). [obs]
The `sharedtaskmergestate` / `sharedpendingchange` tables are the persisted side of that. [inf]

## 4. Claude / MCP automation socket (decompiled)

2Do advertises "Let Claude work with 2Do directly … through MCP". The app side is
`_Do.TodoAutomationSocketServer` (Swift); command execution lives in
`TodoDAOFramework.TodoAutomationMachineExecutor`.

| Step | Function (arm64 slice) | Finding |
|---|---|---|
| Start | `TDMAppDelegate::startAutomationSocketServer` @ `0x100479324` | Lazily creates the server singleton, logs, calls path + listen. Skips if the app lifecycle can't accept requests. [obs] |
| Path | `FUN_1004a9af0` → `FUN_1004a9dd4` | Base = `Preferences._sharedAppGroupContainerURL` (`~/Library/Group Containers/EKT6323JY3.com.guidedways/`). Candidate 1: `Automation/2do-mcp.sock`. Candidate 2 ends in `mcp/…`. First path whose UTF-8 length is < 104 bytes (`sun_path` limit) wins; otherwise error "2Do automation socket path is too long." Swift small-string immediates decoded: `0x6974616d6f747541…` = "Automation", `0x2e70636d2d6f6432…` = "2do-mcp.sock". [obs] |
| Listen | `FUN_1004aa790` | Removes any stale file, `socket(AF_UNIX, SOCK_STREAM, 0)`, `fcntl(O_NONBLOCK)`, then bind/listen, a GCD read source for `accept`, and a cancel handler that closes the fd. [obs] |
| Bind | `FUN_1004ad268` | `sockaddr_un` (`sun_len=0x6a`, `AF_UNIX`), `bind`, then `chmod(path, 0600)` (`FUN_1004ad070`), `listen(fd, 128)`. [obs] |
| Read | `FUN_1004a8fa8` | Reads in 8 KB chunks with a millisecond deadline and a max frame size. Errors: empty / incomplete / oversized frame. [obs] |
| Framing | `FUN_1004a87fc` | Searches the buffer for byte `0x0A` and cuts out the bytes through the newline as one request. **Newline-delimited JSON.** [obs] |
| Response | — | JSON `{"ok":true,…}` / `{"ok":false,"error":{"code","message"}}`; size-capped ("exceeded the native socket frame limit"). [obs] |

Access control [obs]:
- Socket file mode `0600`, so only the same macOS user can connect. There is no auth token in the protocol as far as I found. [inf]
- Refused while 2Do is locked: "Unlock 2Do before running AppleScript or MCP automation."
- Refused during shutdown; a request admission timeout applies ("was busy and could not start the automation request in time").
- The app records `ClaudeMCPLastObservedRequestDate` and shows "Yes (Last request: …)" in settings.

Command names recovered [obs unless noted]. Recovered from Swift small-string immediates and
string sections; **list may be incomplete**. Names ≤15 chars that the compiler loaded another way could be missing. [?]

- Lists/navigation: `list_lists`, `list_list_groups`, `list_smart_lists`, `list_tag_groups`, `list_locations`,
  `select_smart_list`, `select_list_group`, `open_task`
- List settings: `get_list_settings`, `update_list_settings` (fields incl. `sort_field`, `sort_order`, `show_in_done`, `focus_enabled`, `focus_filter_string`)
- Destinations: `get_default_task_destination`
- Tasks: `query_tasks`, `complete_task`, `uncomplete_task`, `delete_task`, `duplicate_task`, `move_task`, `update_task_action`
- Sections: `create_section`, `reorder_sections`
- Probably present but not recovered: task create/update (types `TodoAutomationTaskCreationReminderPolicy`, `TodoAutomationParentTaskDestination`, `TodoAutomationRecurrenceSnapshot` exist). [inf]

The same executor also backs AppleScript (`TodoAutomationScriptCommand`) and App Intents
(`TodoAppIntentAutomationAdapter`), with Darwin notifications signalling mutations between processes. [obs/inf]

The MCP client extension itself is not installed on this machine, so the JSON request
field names (e.g. `command` vs `method`) are not confirmed. [?]

## 5. Licensing (noted, not analysed)

App Store receipt validation (TPInAppReceipt, SwiftyStoreKit), plus a direct-sale license path
(`BusyLicensingDirect`, `AquaticPrimeLicenseValidator`) and a trial anchor stored in the keychain.
Deliberately not investigated further.

## 6. Open questions

- Exact JSON request schema and full command list: decompile `TodoAutomationMachineExecutor` in
  TodoDAOFramework (a large Ghidra import, budget 20+ min), or capture traffic from a real MCP client.
- Second socket-path candidate (`…/mcp/…`) base directory.
- Helper ↔ app XPC protocol.

## 7. CalDAV static review (2026-11-01)

REA 6.8.0 + Ghidra 12.1.4, installed macOS 2Do 2026.4.6, thinned arm64 binaries.
No application execution, production account access, NAS access or iPhone access was performed.
Addresses below belong to the named framework slice, not the main executable.

| Binary | ARM64 SHA-256 | Analysis result |
|---|---|---|
| `MacOS/2Do` | `f6b949f3ed69bb668fb372c3eca2aacdfaf61532e6122ae38b6549fcae6c59fa` | Default analysis completed; CalDAV procedure search identifies imports from TodoDAOFramework. |
| `TodoDAOFramework` | `4ddb12773ee02fd8acbd2d7644009ff376274670af12a49e0b6d34ef85cbc725` | Whole-program analysis timed out after 1,500,000 ms; native dispatch metadata inspected separately. |
| `DAVKit` | `c4e520ab8c6fd9b9d333ba5ed9951e02f5285dfd1014bc1bedda808bd0eee492` | Default analysis completed; reader and writer function dossiers recovered. |

Recovered entry points **[obs]**:

| Binary | Method | Address | Evidence |
|---|---|---|---|
| DAVKit | `CalDAVKitTask::convertToVTODOUsingExistingCalendarData:usingProductID:` | `0x3e1bc` | Decompiled body and assembly; completion/date/recurrence serialization. |
| DAVKit | `CalDAVKitTask::parseAsVTODO` | `0x409e4` | Decompiled body and assembly; completion precedence, recurrence and alarm parsing. |
| TodoDAOFramework | `TodoTask::setMetaFromString:isForCalDAV:withConnection:` | `0xeee818` | Native Objective-C dispatch metadata only; body not recovered. |
| TodoDAOFramework | `TodoTask::setMetaFromDictionary:isForCalDAV:withConnection:` | `0xef15e0` | Native Objective-C dispatch metadata only; body not recovered. |

See [06 §1.7](06-sync.md#static-readerwriter-findings-2026-11-01) for findings and the
completion-precedence discrepancy with Twydo. Method names alone do not establish metadata
semantics. JSON metadata precedence, recurring copy-and-advance and provider conflict policy
were not confirmed by this session.

**Analysis limits:** TodoDAOFramework produced unsupported Objective-C type-encoding errors,
invalid flow targets and decompiler address-space errors before reaching its startup deadline.
No out-of-memory condition was confirmed. Its incomplete whole-program analysis supplies no
behavioral equivalence claim. Native metadata inspection also reports partial decoding, so it
is not an exhaustive class or method inventory.

**Local evidence exports** (temporary artifacts, not committed; may disappear):
`/tmp/twydo-2do-main-rea.json`, `/tmp/twydo-dao-evidence.json`,
`/tmp/twydo-davkit-evidence.json`, `/tmp/twydo-davkit-rea.json`, and extracted pseudocode
`/tmp/twydo-davkit-0x3e1bc.c`, `/tmp/twydo-davkit-0x409e4.c`.
DAVKit reader dossier: `ev_bd9a15cb498a0de32ec733b9fa476d0e1729a1a806cbb60b49607f1536eeb01e`;
writer dossier: `ev_763ca1b3d686ddebecb70214ebf1589f964792ac56187e56ad5e3960ff91b84d`.
These are static observations, not original source or runtime traces.

**Timeout reproduction:** there are two independent deadlines. The MCP client initially
timed out after 300 seconds; Codex's REA registration was changed to
`tool_timeout_sec = 1800` and reloaded. REA's separate Ghidra startup default was 330,000 ms;
`update_analysis_settings` with `ghidra.startup_timeout_ms = 1500000` allowed the main
executable to finish, but TodoDAOFramework still timed out. Runtime settings apply to the
next startup and do not survive an REA restart. Increasing only the client timeout does
not increase the provider startup allowance.

## Reproduce

```sh
lipo /Applications/2Do.app/Contents/MacOS/2Do -thin arm64 -output 2Do-arm64
# REA: update_analysis_settings ghidra.startup_timeout_ms=1800000 (default 330 s times out)
# REA: open_binary 2Do-arm64 provider_id=ghidra, then procedure_pseudo_code on the addresses above
```
