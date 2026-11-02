#!/usr/bin/env python3
"""Generate the demo vdir: the synthetic sample vdir with its dates moved to "today".

Usage: demo_vdir.py OUT_DIR [--today YYYY-MM-DD]

The sample (crates/twydo-core/tests/data/sample-vdir, all content made up) is anchored at
2026-11-04. Every date is shifted by the days between that anchor and --today (default: the
local date), so the Overdue and Today groups stay populated whenever the demo is recorded.
A weekly BYDAY and a monthly BYMONTHDAY follow the shifted due date; a monthly due date that
lands after the 28th moves back to the 28th, so the repeat stays in a verified rule class.
"""

import argparse
import re
import shutil
import sys
from datetime import date, datetime, timedelta
from pathlib import Path

SAMPLE = Path(__file__).resolve().parent.parent / "crates/twydo-core/tests/data/sample-vdir"
ANCHOR = date(2026, 11, 4)
DATE_PROPS = {"DUE", "DTSTART", "COMPLETED", "CREATED", "DTSTAMP", "LAST-MODIFIED", "TRIGGER"}
VALUE = re.compile(r"^(\d{8})(T\d{6}Z?)?$")
WEEKDAYS = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]


def _name(line: str) -> str:
    return re.split(r"[;:]", line, maxsplit=1)[0].upper()


def _shift_value(line: str, days: int) -> str:
    head, value = line.rsplit(":", 1)
    m = VALUE.match(value)
    if not m:  # e.g. a relative TRIGGER:-PT15M
        return line
    day = datetime.strptime(m.group(1), "%Y%m%d").date() + timedelta(days=days)
    return f"{head}:{day:%Y%m%d}{m.group(2) or ''}"


def _due(lines: list[str]) -> date | None:
    for line in lines:
        if _name(line) == "DUE":
            return datetime.strptime(line.rsplit(":", 1)[1][:8], "%Y%m%d").date()
    return None


def _set_due_day(line: str, day: int) -> str:
    head, value = line.rsplit(":", 1)
    return f"{head}:{value[:6]}{day:02d}{value[8:]}"


def shift_ics(text: str, days: int) -> str:
    """Shift every date in one .ics text by `days`, keeping its line endings."""
    newline = "\r\n" if "\r\n" in text else "\n"
    lines = text.split(newline)
    for prev, line in zip(lines, lines[1:]):
        if line[:1] in (" ", "\t") and _name(prev) in DATE_PROPS:
            raise ValueError(f"folded date line not supported: {prev!r}")
    lines = [_shift_value(l, days) if _name(l) in DATE_PROPS else l for l in lines]

    due = _due(lines)
    for i, line in enumerate(lines):
        if _name(line) != "RRULE" or due is None:
            continue
        if "BYMONTHDAY=" in line:
            if due.day > 28:
                lines = [_set_due_day(l, 28) if _name(l) == "DUE" else l for l in lines]
                due = due.replace(day=28)
            lines[i] = re.sub(r"BYMONTHDAY=\d+", f"BYMONTHDAY={due.day}", line)
        elif "FREQ=WEEKLY" in line and "BYDAY=" in line:
            lines[i] = re.sub(r"BYDAY=[A-Z]{2}", f"BYDAY={WEEKDAYS[due.weekday()]}", line)
    return newline.join(lines)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("out_dir", type=Path)
    parser.add_argument("--today", type=date.fromisoformat, default=date.today())
    args = parser.parse_args(argv)

    days = (args.today - ANCHOR).days
    for src in sorted(SAMPLE.rglob("*")):
        if not src.is_file() or src.name == "README.md":
            continue
        dst = args.out_dir / src.relative_to(SAMPLE)
        dst.parent.mkdir(parents=True, exist_ok=True)
        if src.suffix == ".ics":
            text = src.read_bytes().decode("utf-8")
            dst.write_bytes(shift_ics(text, days).encode("utf-8"))
        else:
            shutil.copyfile(src, dst)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
