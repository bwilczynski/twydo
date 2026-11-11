# Reference material

For current behavior, read [the documentation](../README.md). This directory contains:

| Reference | Purpose |
|---|---|
| [2Do formats](2do/README.md) | Observed/inferred encodings, task semantics, metadata and sync behavior |
| [Compatibility evidence](evidence.md) | Focused operation cases, known failures, unresolved work and capture/promotion workflow |
| [Designs and plans](../superpowers/) | Design history and implementation decisions |

Python helpers in [tools](../../tools/) use the standard library. `capture_fixture.py`
captures explicitly selected resources into complete named states and scans staging for private
data; `check_evidence.py` validates cases and references. `seed_fixture.py` prepares synthetic
resources for separately approved test rounds. `decode_2do_dropbox.py` and `reconcile.py`
support private migration inventory/reconciliation; their real-data output stays outside git.
