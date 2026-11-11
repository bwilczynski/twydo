#!/usr/bin/env python3
"""Validate focused evidence cases and compatibility references."""
import sys
from pathlib import Path

if __package__:
    from .evidence import validate
else:
    from evidence import validate


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    root = Path(argv[0]) if argv else Path(__file__).resolve().parent.parent
    errors = validate(root)
    for error in errors:
        print(error)
    return bool(errors)


if __name__ == "__main__":
    sys.exit(main())
