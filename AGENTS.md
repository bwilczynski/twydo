# Agent handoff

Continue building Twydo (`tdo`), a terminal task app sharing CalDAV tasks with 2Do.

## Read in this order

1. [Main design](docs/superpowers/specs/2026-10-31-terminal-tasks-design.md): fixed user
   decisions and preservation, compatibility, conflict, recovery and notification contracts.
2. [Current documentation](docs/README.md): update the matching page when behavior changes;
   `python3 tools/check_links.py` must pass.
3. [Evidence guide](docs/reference/evidence.md): operation cases, known failures, unresolved
   questions and device consent boundaries. Consult [2Do format references](docs/reference/2do/README.md)
   for exact encodings; observed/inferred/unknown labels remain significant. The spec wins.

## Current work and roadmap

The [operation-evidence plan](docs/superpowers/plans/2026-11-11-operation-evidence.md)
implements the approved [operation-contract design](docs/superpowers/specs/2026-11-11-operation-evidence-design.md).
Continue its first unfinished task and execution ledger; do not start another plan over it.
This is breaking `tdo` version `0.2.0`: custom matrices use version 2. Fixtures live under
`fixtures/<operation>/<case>/`, with complete named states and case metadata. Do not revive
numbered session snapshots, inherited states, stored diffs or the old global rule/edge gate.

The phase-0 gate passed and was approved on 2026-11-01. Writable task operations, TUI editing,
flat families/recurrence, ordering, list management, the durable write queue and daemon plans
1–4 are implemented. The daemon owns configuration, data, sync and scheduling; TUI/CLI clients
use its Unix socket. Tests must isolate both `XDG_CONFIG_HOME` and `TWYDO_SOCKET`. Stop only
an isolated test daemon with `tdo daemon stop`. A custom write-policy matrix is supplied to
`tdo daemon --matrix`; per-report overrides do not alter the daemon's write policy.

After this refactor: write-queue plan 3 (rebasing inside the daemon), then Plan 2 (notifier).
Later features include indent/outdent, yank/paste, list management, Project View and natural
language input. Read the current plan and as-is documentation before choosing new work.

## Device and data boundaries

No new phone/server rounds are authorized by this handoff. Plan 8 batch 1, RF/RA, item deletion
and both item-reordering/insertion rounds are complete; do not repeat them. Saved phone ranks
require **Manually**. Preserve the synthetic lists/families and private backups/session state
outside git. Do not change accounts, global write mode or other data. Any further device scope
needs a bounded unresolved question and fresh approval; ask before production 2Do, NAS or iPhone
access. Existing captures and local preservation tests should be reused first.

## Ground rules

- **No personal data in this repository.** Never commit real task titles, list names, notes, dates
  of real obligations, people's names, addresses, account identifiers, credentials, CalDAV
  URLs/usernames, or raw exports (Dropbox `2Do/` folder, decoded JSON, `.ics` from the user's
  production calendars). Test fixtures must come from the **isolated test account** with synthetic
  content, or be anonymised before committing. `.gitignore` blocks the common cases; check
  `git diff --staged` anyway.
- The spec refers to acceptance cases **P1, P2, L1** by label. Their real names live outside the
  repo in `../rea-private/acceptance-cases.md` (on the user's machine only). You may read it locally
  for acceptance testing; never copy its contents into the repo.
- **Commit messages and PR titles must pass commitlint** (`.commitlintrc.json`,
  `@commitlint/config-conventional`); PR validation checks every commit and the PR title. Use
  `type(scope): subject` with a lowercase type such as `feat`, `fix`, `docs`, `test`, `refactor`,
  `ci` or `chore`. The subject must not start with a capital letter, even for a key or proper
  name: write `feat(tui): toggle parents with Enter`, not `feat(tui): Enter toggles parents`.
  No trailing full stop, header at most 100 characters, a blank line before the body, and body
  lines at most 100 characters. Before pushing, run
  `npx --yes -p @commitlint/cli -p @commitlint/config-conventional commitlint --from=origin/main --to=HEAD`
  and pipe the PR title through the same command (`printf '%s\n' "<title>" | npx … commitlint`).
- Code lives in this repository as a Cargo workspace at the root:
  - `crates/twydo-core/`: core library (data model, storage, views)
  - `crates/tdo/`: the `tdo` binary (CLI and TUI)
  - `compat/`: the phase-0 compatibility matrix (`matrix.toml`)
  - `contrib/`: example configuration for external tools (vdirsyncer, systemd, launchd)
  - `tools/`: helper scripts (tests in `tools/tests/`)
  - `docs/`: the as-is pages at its root, `docs/demo/` (VHS demo), `docs/reference/` (2Do reference
    and the phase-0 checklist and session logs), `docs/superpowers/` (spec and plans)
- The spec's "decisions made by the user" (§4) are fixed; "decisions made by me" (§5) are proposals
  you may refine with justification, except where §11 says they cannot be overridden (compatibility
  gate, preservation, conflict, recovery and notification contracts).
