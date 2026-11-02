# Reference material

This folder keeps the raw material Twydo was built from. It is kept as it was written, so parts of
it describe the state at the time. For how Twydo works today, read the pages in [`docs/`](../README.md);
where they and this folder disagree, `docs/` wins.

| Folder | What it holds |
|---|---|
| [`2do/`](2do/README.md) | 2Do's behaviour and formats as observed: features and rules, data model, look and feel, keyboard shortcuts, the Dropbox sync format, and CalDAV sync with 2Do's `X-2DOAPP-*` metadata |
| [`phase0/`](phase0/README.md) | The phase-0 compatibility gate checklist and the logs of every session run against a test iPhone (Plans 3–6 and the gap session). The verdicts they recorded live in [`compat/matrix.toml`](../../compat/matrix.toml) |
| [`../superpowers/`](../superpowers/) | The design spec and the implementation plans |

## Tools used in those sessions

All in [`tools/`](../../tools/), Python 3 with no dependencies; tests are in `tools/tests/`.

| Script | What it does |
|---|---|
| [`decode_2do_dropbox.py`](../../tools/decode_2do_dropbox.py) | Reads and writes 2Do's Dropbox sync files; used to inventory data before migrating to CalDAV |
| [`capture_fixture.py`](../../tools/capture_fixture.py) | Snapshots a test collection into `fixtures/phase0/` as a numbered step with a diff against the previous one; `scan` checks fixtures for personal data before they are committed |
| [`seed_fixture.py`](../../tools/seed_fixture.py) | Seeds a test collection with tasks from captured fixtures, instead of typing them on the phone |
| [`reconcile.py`](../../tools/reconcile.py) | Compares a decoded 2Do inventory with `tdo export-json` after a migration, to show nothing was lost |
