# Item relocation candidate

The three `reparent` rows (`item-to-project`, `item-to-checklist`, `item-outdent`)
are unverified in the shipping matrix, including the iPhone baseline required by
test mode. No device round has been performed for this implementation.

## Evidence and proposed encoding

The [family baseline](plan6-baseline.md) establishes standard `RELATED-TO` and JSON
`parentuid` representations for existing families, and whole-family list moves
recreate resource identities. Those captures do **not** establish the identity
policy of moving a single existing item between parents or out to a list.

The local candidate recreates one open plain leaf with a stable new resource UID,
removes the old resource, and changes both parent links together. A distinct parent
JSON `uid` is used for `parentuid`, while `RELATED-TO` uses its resource UID. On
detach both parent links disappear. JSON self `uid` follows the resource; foreign
`uid` and `RUID` survive. Attach retains the member-move RUID convention; detach
rewrites self RUID as a top-level list move does. These are candidate policies,
not newly observed encodings.

Project membership preserves scheduling. Checklist inbound, or checklist outdent,
can clear only date-only due and matching mirrored start fields. Own start/time,
recurrence and alarms requiring cleanup are refused. Inbound date loss asks for
confirmation in the TUI. Unknown properties and unrelated resources survive.
Parents with children, nested families, closed leaves/parents, conflicting links,
ambiguous identities and same-parent moves are excluded. Use ordering for the last.

The journal persists both parent member sets, unchanged parent/sibling bytes,
and old/new leaf identities through queueing, recovery and undo/redo. Candidate
matrices in synthetic tests exercise these paths without promoting shipping rows.

## Minimum future verification proposal

Requires fresh explicit approval and an isolated synthetic account/list; it is not
a request to repeat earlier phone rounds.

1. Move one dated standalone task into a project, then move its item to another
   project. Capture resource UID, internal UID, RUID, both parent links and rank
   before/after; check unrelated family bytes.
2. Move one simply dated task into a checklist, confirming date loss, then outdent
   an undated checklist item. Capture date/start metadata and both removed links.
3. Exercise Twydo candidate output and reverse-direction reload on those exact
   shapes, including distinct resource/internal parent identities and a foreign
   series reference. Check the phone displays one item at the appended rank and
   leaves the source family intact.

Do not enable all three rows from one example if its identity/date mechanism does
not cover the others. No production data, NAS, phone or global write mode was
changed by local implementation tests.
