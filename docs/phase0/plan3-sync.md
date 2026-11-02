# Sync and conflicts against Synology and the iPhone (Plan 3 Task 13)

Twydo's sync runner (`tdo sync`), conflict hook and conflict resolution, checked against the isolated
test account: Synology Calendar on the NAS, 2Do on the iPhone. Same two test lists as Plan 4
(`Twydo test`, `Twydo test 2`), single-letter synthetic titles, rounds of at most six actions.

## Setup (🤖, test account only)

- `~/.config/vdirsyncer/config`, pair `twydo`: `conflict_resolution = ["command", "<abs path>/tdo", "conflicts", "hook"]`.
  The absolute path points at the worktree's release build, because `tdo` is not on `PATH`.
- Twydo test config `~/twydo-phase0/xdg/twydo/config.toml` gains:

  ```toml
  [sync]
  vdirsyncer_config = "~/.config/vdirsyncer/config"
  command = "~/twydo-phase0/vdirsyncer-tee.sh"   # runs vdirsyncer, appends its stderr to ~/twydo-phase0/reports/vdirsyncer.log
  after_edit = false                             # rounds 1–6 sync by hand; round 7 turns it on
  ```

- The wrapper exists only to capture vdirsyncer's raw output for parser fixtures.
- Shell helpers (tdo only sees the test config):

  ```sh
  T=<worktree>/target/release/tdo
  tdo() { XDG_CONFIG_HOME=~/twydo-phase0/xdg "$T" "$@"; }
  L1=<collection dir of "Twydo test">
  L2=<collection dir of "Twydo test 2">
  ```

- Both lists start empty.

## Rounds

"Sync on the phone" means: open 2Do, pull to refresh, wait until it settles.

| Round | Who | Actions | Expected |
|---|---|---|---|
| 01-edit-vs-edit | 🤖 `tdo op add --list $L1 --title A --due 2026-12-10`, `tdo sync` · 👤 sync, see A · 👤 edit A's title to `A2`, sync · 🤖 `tdo op edit <A> --notes t`, `tdo sync` | `tdo sync` exits 1; `tdo conflicts` shows one `changed here and on another device` |
| 01b-resolve | 🤖 `tdo conflicts resolve <id> --keep remote`, `tdo sync` · 👤 sync | vdir holds `A2` with no notes; the phone shows `A2`; the conflict is resolved |
| 01c-keep-local | 👤 edit A2's notes to `p`, sync · 🤖 `tdo op edit <A> --title A3`, `tdo sync`, `tdo conflicts resolve <id> --keep local`, `tdo sync` · 👤 sync | the phone shows `A3` and has no `p` notes; check that 2Do reads the LF-ended task (R4) |
| 02-concurrent-completion | 👤 create `M` in Twydo test, due 2026-12-15, repeat monthly, sync · 🤖 `tdo sync` · 👤 complete M, sync · 🤖 `tdo op complete <M>`, `tdo sync` · 👤 sync | Record what happens to the live task and the history copies (no fix planned). `tdo conflicts` shows the live task conflict |
| 03-twydo-delete-vs-phone-edit | 🤖 `tdo op add --list $L1 --title D`, `tdo sync` · 👤 sync, edit D's notes `n`, sync · 🤖 `tdo op delete <D>`, `tdo sync` | `local-delete-remote-edit` conflict; `--keep local` then `tdo sync` → D gone on the phone |
| 04-phone-delete-vs-twydo-edit | 🤖 `tdo op add --list $L1 --title E`, `tdo sync` · 👤 sync, delete E, sync · 🤖 `tdo op edit <E> --notes x`, `tdo sync` | `remote-delete-local-edit` conflict; `--keep remote` → E gone from the vdir; the phone still has no E |
| 05-emptied | 🤖 `tdo op add --list $L2 --title F`, `tdo sync` · 👤 sync, delete F, sync · 🤖 `tdo sync` | exit 1, `emptied-remote` hint, F still in the vdir; `tdo sync --force-delete "Twydo test 2"` → F gone. Then 🤖 move the last task of Twydo test to Twydo test 2 and `tdo sync` → forced automatically, exit 0 |
| 06-offline | 🤖 point `vdirsyncer_config` at a copy with a wrong host, `tdo sync` twice, `tdo status` · restore, `tdo sync` | failure + `retry HH:MM` in `tdo status` and the TUI; the second run backs off with `--background`; restored → `synced` |
| 07-timer | 🤖 `after_edit = true`, install the timer (test-setup environment) · 👤 edit a task, rename `Twydo test 2` to `Twydo test 2b` · wait about 2 minutes, with the TUI closed | the edit and the new display name reach the vdir; `tdo status` shows a recent success |
| 08-cleanup | 👤 rename the list back · 🤖 delete leftovers with `tdo op delete`, `tdo sync` | both lists empty; no unresolved conflicts |

Throughout: note any "invalid task reference" error the iPhone shows after a sync that deleted items.

## Results

Session 2026-11-02, iPhone 2Do 3.91, Synology Calendar, vdirsyncer 0.21.0 on Linux.

- **Setup:** the first `tdo sync` failed because `pass` could not decrypt the account password (gpg
  pinentry timed out, about 3 minutes). Twydo reported it as a transient failure with a retry time;
  after unlocking gpg the baseline sync took 6 s. The background timer will fail the same way when
  the gpg agent's cache expires.
- **01 edit vs edit:** a Twydo notes edit plus a phone title edit gave one `both-changed` conflict;
  other items kept syncing. `--keep remote` put the phone's version in the vdir (LF line endings, R4).
- **01c keep local:** a phone notes edit plus a Twydo title edit; `--keep local` sent Twydo's
  version to the server LF-ended. **2Do on the iPhone showed it correctly and could edit it again**,
  and the next `tdo sync` pulled that edit. R4 holds: LF-ended tasks are fine for 2Do and Synology.
- **02 concurrent completion (monthly M, due 2026-12-15):** the phone completed M and synced, then
  Twydo completed it before pulling. vdirsyncer reported `...same content on both sides.` for the live
  task (Twydo's copy-and-advance matched 2Do's except for timestamps vdirsyncer ignores), so there was
  **no conflict**. Both history copies (different UIDs) synced both ways. Result on both sides: live M
  due 2027-01-15, monthly, once; **two completed copies of the 2026-12-15 occurrence**; no 2Do sync
  error. Nothing is lost, but the completion is recorded twice. Not fixed in Plan 3 (spec §9 asks that
  retries not duplicate history; this is two real completions, not a retry). Possible follow-up:
  detect two history copies of the same series and occurrence after a sync and offer to delete one.
- **03 Twydo delete vs phone edit:** the phone edited D's notes and synced; Twydo deleted D. vdirsyncer
  restored the phone's version locally and exited 0 (F6 confirmed on CalDAV: `Copying (uploading) item
  <uid> to twydo_local/<coll>`), and Twydo recorded a `local-delete-remote-edit` conflict. `--keep local`
  deleted it locally and the next sync deleted it on the server (`warning: Deleting an item with no
  etag.`). D disappeared on the phone. **The first refresh after that sync showed 2Do's sync error
  again** ("invalid task reference" class, as in Plan 4); the retry succeeded. This happens with every
  delete vdirsyncer sends to Synology, not only Twydo's resolutions. Twydo cannot fix it; it is cosmetic
  and settles on the next refresh.
- **04 phone delete vs Twydo edit:** the phone deleted E and synced; Twydo edited E. vdirsyncer
  re-uploaded Twydo's edit to the server and exited 0 (F6, `Copying (uploading) item <uid> to
  twydo_remote/<coll>`), and Twydo recorded a `remote-delete-local-edit` conflict. `--keep remote`
  removed E locally and the next sync deleted the re-uploaded copy on the server. E stayed gone on the
  phone; no 2Do error this time.
- **05 emptied lists:** the phone deleted F, the only task of Twydo test 2. `tdo sync` exited 1 with
  `list "Twydo test 2" was emptied on the server; to apply that here run: tdo sync --force-delete
  "Twydo test 2"`; F stayed in the vdir. The explicit `--force-delete` removed it locally (exit 0).
  Then Twydo created G there and moved it to Twydo test: vdirsyncer stopped on the local-emptied list,
  Twydo forced it in the same run (`applied the emptied list "Twydo test 2" on the server`, exit 0).
  The phone showed Twydo test 2 empty and G in Twydo test; no 2Do error.
- **06 offline:** a vdirsyncer config with a changed URL is rejected by vdirsyncer itself (`critical:
  Detected change in config file, please run vdirsyncer discover`), reported by Twydo as a transient
  failure. A real network failure was simulated with a dead HTTPS proxy: one `error: Unknown error
  occurred for twydo/<coll>: Cannot connect to host …` per list, `tdo status` showed `failures: 2`,
  the retry time and the error; `--background` skipped while backing off; with the network back a
  manual `tdo sync` succeeded and reset the failure count. **Bug:** `metasync error` showed the first
  stderr line (`Metasyncing twydo/home`) instead of the first error line.
- **07 timer:** with the systemd user timer installed (test-setup environment) and the TUI closed, a
  phone notes edit reached the vdir within one timer run (about a minute); renaming Twydo test 2 to
  Twydo test 2b reached `displayname` one run later through metasync (`Copying displayname to
  twydo_local/<coll>`), matching 2Do's known one-refresh lag.
- **08 cleanup:** Twydo deleted the five remaining tasks with `after_edit = true`. Each `tdo op` waited a
  few seconds for the background sync the previous one had started (writers wait for sync, as
  designed). Deleting the last task of Twydo test was forced automatically by a background run. End
  state: both lists empty on the phone, 18 journal entries all confirmed, no pending changes, no
  conflicts. The timer was then disabled again (it runs a worktree build).

### Conclusions

- Every planned case behaved as designed against Synology and iPhone 2Do: edit-vs-edit conflicts with
  both choices, both delete-vs-edit directions (which vdirsyncer alone would settle silently), emptied
  lists from either side, offline backoff and recovery, the background timer and metasync.
- **R4 is confirmed safe:** 2Do reads and edits a task that came back LF-ended after a conflict choice.
- **Concurrent completion** of a repeating task gives no conflict and two history copies (round 02).
  Recorded, not fixed.
- **2Do's "invalid task reference" error** still appears on the first refresh after some vdirsyncer
  deletes (round 03) and clears on retry. Not a Twydo issue.
- **gpg:** a background sync fails while the gpg agent's cache is locked; set a long cache TTL for the
  account's `pass` entry before relying on the timer.
- **Bug found and fixed:** the metasync error in `tdo status` showed the first stderr line, not the
  first error line (round 06); fixed in `fix(sync): report metasync's first error line`.
- Anonymised vdirsyncer output from this session is in `crates/twydo-core/tests/data/vdirsyncer/caldav-*.txt`.
