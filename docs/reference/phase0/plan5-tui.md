# TUI editing against the iPhone (Plan 5 Task 8)

The TUI's write keys, checked against the isolated test account (Synology Calendar on the NAS,
2Do on the iPhone). Same two test lists as Plans 3 and 4 (`Twydo test`, `Twydo test 2`),
single-letter synthetic titles. Session on 2026-11-02.

## Setup

- Release build of the `plan-5-tui-editing` worktree, run as
  `XDG_CONFIG_HOME=~/twydo-phase0/xdg <worktree>/target/release/tdo`.
- Test config as in [plan3-sync.md](plan3-sync.md): `[writes] mode = "test"`, `[sync] after_edit = false`
  (every sync by hand with `:sync`).
- Both lists started empty. The TUI cannot create tasks yet, so the phone created them.

## Rounds

| Round | Where | Actions | Result |
|---|---|---|---|
| 1 | iPhone | in `Twydo test`: P one-time due tomorrow; Q monthly on a day ≤ 28; R one-time, no due date; sync | pass |
| 2 | TUI | P `x` then `u`; Q `]` then `t`; R `m` → `Twydo test 2`; P `i`: title P2, due `fri 5pm`, two-line note in `$EDITOR`, `Ctrl-s`; Q `dd` then `u`; `:sync` | pass |
| 3 | iPhone | sync; check P2 open, due Fri 17:00, two-line note; Q due today; R in `Twydo test 2`; add a third note line to P2; sync | pass |
| 4 | TUI | `:sync`; P2's details show all three note lines | pass |

## Findings

- **Multi-line notes** written by Twydo (escaped `\n` in `DESCRIPTION`) read correctly on the
  iPhone, and the phone's edit of them reads back correctly in Twydo. This was the one write
  shape the TUI added beyond what `tdo op` had already verified.
- **The iPhone needed two refreshes** to show all of round 2's changes; the first pull brought
  only some. Seen on the 2Do side only; nothing was lost. Same family as the transient
  "invalid task reference" in plan3-sync.md. Not acted on.
- **TUI:** the status line kept `sync done` indefinitely. Fixed on the branch: success messages
  now clear after five seconds; errors stay until `Esc`.

No matrix row changes: every operation used here was already verified by Plan 4.
