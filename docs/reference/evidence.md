# Compatibility evidence

The compatibility gate passed and was approved on 2026-11-01. Its useful results now
live in operation cases under [`fixtures/`](../../fixtures/), rather than numbered
session snapshots. [`compat/matrix.toml`](../../compat/matrix.toml) records the
operation contracts those observations support. Historical session logs, completed
checklists and unused captures were removed; git retains their original versions.

## Reading a case

Each `fixtures/<operation>/<case>/case.toml` states one claim, its observed scope,
source revision/path, environment provenance, regression tests and any contracts reusing it.
Named states contain complete small vdirs: collection directories, exact captured
resources and explicitly declared membership. An empty collection means absence;
there are no inherited states or implicit previous snapshots. Zero-byte `.gitkeep`
markers retain empty collection directories in git; they are not task resources.

Per-capture client versions stay explicitly unknown when the source does not record them;
the original baseline environment is available through the recorded source revision.
Captured resource bytes, UIDs and relationships are preserved. Migrated provenance
points to the source commit where old paths existed. `action` and `device` record
what produced each retained state. Result-only visual or protocol observations may
have no `.ics` state: an unchanged file cannot prove a screen display or a server's
conditional-write behavior.

Direct observations establish representative interoperability. Local exact-byte
tests establish preservation across other shapes; they do not count as additional
phone observations. For example, moving a family rewrites its identities and links
while preserving its recurrence. Moving it does not require permission to execute
that recurrence. Completion does require its specific advancement/reset/history
evidence. Known failures remain useful evidence for refusals.

Cases have one owning operation, and other contracts/tests reference them explicitly.
Hand-authored parser/unit fixtures stay beside their tests. Use small synthetic
builders for incidental setup; importing another historical snapshot is not the
default way to make a test task.

## Operation evidence

| Operation | Representative evidence | What distinguishes the contract |
|---|---|---|
| Title and notes | [Standalone lifecycle](../../fixtures/complete/standalone-lifecycle/case.toml), [checklist reset](../../fixtures/complete/checklist-reset/case.toml) | Text and stamps change; identity, family membership, schedules and unrelated bytes are preserved |
| Due/start edits | [Completion anchoring](../../fixtures/set-due/from-completion/case.toml), [item-date cleanup](../../fixtures/create/family-roundtrip/case.toml) | Native date representation, anchors and offsets; narrow recurring-checklist cleanup |
| Repeat changes | [Phone encodings](../../fixtures/set-repeat/phone-encodings/case.toml), [encoding round trips](../../fixtures/set-repeat/encoding-roundtrip/case.toml) | Interval-only edits differ from reshaping/removal; unsupported encodings stay refused |
| Creation/conversion | [Family round trips](../../fixtures/create/family-roundtrip/case.toml), [checklist reset](../../fixtures/complete/checklist-reset/case.toml) | Exact task type, links and recurrence metadata; candidate conversions remain refused |
| Completion/reopening | [Recurrence profiles](../../fixtures/complete/recurrence-profiles/case.toml), [family lifecycle](../../fixtures/complete/family-lifecycle/case.toml), [checklists and UNTIL](../../fixtures/complete/checklist-and-until/case.toml) | Ordinary completion, advancement, reset, history, ended series and detached history differ |
| Move | [Standalone destination](../../fixtures/move/standalone-destination/case.toml), [family round trips](../../fixtures/move/family-roundtrip/case.toml) | Whole resource/family identity maps; foreign series identities remain unchanged |
| Delete | [Family items](../../fixtures/delete/family-items/case.toml), [mixed families](../../fixtures/complete/family-lifecycle/case.toml) | Standalone, whole-family and individual-item removal; open, completed and last items |
| Insert/reorder | [Checklist append](../../fixtures/insert/checklist-items/case.toml), [project append](../../fixtures/insert/project-items/case.toml), [saved ranks](../../fixtures/reorder/family-items/case.toml) | DisplayOrder-only changes; phone must select **Manually** to display saved ranks |
| Collections | [Creation confirmation](../../fixtures/list-create/phone-confirmation/case.toml), [phone-created discovery](../../fixtures/list-create/phone-created-discovery/case.toml), [archive observation](../../fixtures/list-metadata/archive-observation/case.toml), [server preconditions](../../fixtures/list-metadata/server-preconditions/case.toml) | Archive is local; successful round trips alone do not establish atomic writes |

## Remaining restrictions and consent

The gate and [compatibility guide](../compatibility.md) define the exact enabled
shapes. This list records useful unresolved questions, not permission to run them:

- The tested iPhone 2Do misreads last-day, Nth-workday and Nth-weekend-day encodings.
  Twydo never writes them. A missing-fifth-weekday execution observation is still absent.
- Recurring-checklist completion remains plain monthly, date-only, day 1–28, without
  ends or completion anchoring. Ended-series reopening remains the bounded top-level
  date-only daily/plain-monthly UNTIL case; other endings use undo.
- The recurring-checklist interval candidate remains refused, including test mode.
  Other native scheduling, project/item and lifecycle combinations need their own
  unresolved question before any additional device scope is proposed.
- Project repeat changes, some conversions, independently recurring child creation
  and membership changes retain their unverified boundaries. Reparenting is a candidate.
- List creation passes both directions; a phone-created list was adopted by local-only
  discovery with its identity, name, color and tasks. Metadata edits remain refused
  in every mode: a tokenless PROPPATCH succeeded while the server reported an exclusive
  lock. An obsolete calendar-export ETag also failed to protect collection DELETE,
  despite changing with task membership/content. Product list deletion now ships as best-effort
  (user decision 2026-11-12; see the best-effort list-writes design); the evidence about the
  failed server guards stands.
- Phone archive/unarchive changed neither the captured task bytes/ETags nor the 14
  inspected collection properties. Archive remains local; no shared encoding is claimed.

Plan 8 batch 1 and the RF/RA phone rounds are complete. Item-deletion and both
item-reordering/insertion rounds are complete. Do not repeat their uploads or phone
actions. Do not delete the synthetic families/lists, change accounts or global write
mode, or touch other data. Private session state and backups remain outside git.
Before another device/server round, prepare a bounded checklist naming an unresolved
failure mechanism and obtain fresh approval. Ask before production 2Do, NAS or iPhone
actions; this refactor authorizes none.

## Gathering and promoting a case

Keep raw staging outside git. Use explicit collections and resource selection:

```sh
python3 tools/capture_fixture.py capture "$TEST_VDIR" "$STAGING" \
  --operation move --case flat-family --state before \
  --collection source --collection destination \
  --resource source/parent.ics --resource source/child.ics \
  --device iphone --origin device \
  --claim 'Moving the family preserves its schedule' \
  --test crates/twydo-core/tests/gate.rs::capability_move_gate_is_independent_of_schedule_execution \
  --action 'Record the observed action and result here'
```

Enumerate the complete relevant family explicitly. Declare an empty destination
collection to record absence. Include collection metadata only when it matters to
the claim. The helper reads the source, writes a separate named state, and never
looks up a previous state, contacts a server or changes source data. It refuses an
existing state and unsafe selections.

Fill unknown versions/provenance from actual records; leave genuinely unknown facts
explicitly unknown. Distinguish device observations, local synthetic cases and protocol
observations. Record which assertion proves the result and which fields are preserved.
Retain extra states only for a distinct transition or failure mechanism. Use ordinary
directory diffs on demand instead of storing generated patches.

Run `python3 tools/capture_fixture.py scan "$STAGING"` and inspect its free-text
listing. Human confirmation that new staged content is synthetic is still required
before promotion. Copy the curated case into its operation folder, reference it from
the appropriate contract, and run `python3 tools/check_evidence.py` plus the focused
tests. These checks also run through `mise run test`.

## Tests

Keep assertions at the lowest layer that observes the contract; see
[testing](../testing.md). Admission parity pins concrete requests across both modes,
writer tests compare captured transformations and preserved bytes, and store tests
protect stale dependencies, queue replay, inverse writes and crash recovery. Neither
an admission test nor a phone display observation replaces the other.

The corpus validator checks manifest references, resource membership, test references,
provenance and observation origin. It cannot independently verify a claimed human
observation. Review the recorded action and the actual assertions when promoting evidence.
