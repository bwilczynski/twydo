#!/usr/bin/env python3
"""Seed a test collection with copies of captured fixture tasks.

Usage:
  seed_fixture.py <fixture-step-dir> <collection-dir> [--only S1,S2,...] [--rename OLD=NEW,...]
                  [--dry-run]

Copies the *.ics files of a captured step (written by 2Do, synthetic titles) into a vdirsyncer
collection folder, so that the next `vdirsyncer sync` uploads them and the phone pulls them
instead of someone typing them in. Each copied task gets a fresh UID, and every reference to an
old UID among the copied tasks (RELATED-TO, parentuid and RUID in <2Do Meta>) is rewritten.
CREATED, DTSTAMP and LAST-MODIFIED are set to now; nothing else changes.

--only picks tasks by exact SUMMARY; a child whose parent is not picked is refused.
--rename changes SUMMARY values of the copies (e.g. P=Q,p1=q1), so one family can be seeded twice.
"""
import argparse
import re
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path

STAMP_PROPS = ("CREATED", "DTSTAMP", "LAST-MODIFIED")


def unfold(text: str) -> list:
    out = []
    for line in text.split("\n"):
        if line.startswith((" ", "\t")) and out:
            out[-1] += line[1:]
        elif line:
            out.append(line)
    return out


def fold(line: str) -> str:
    """Fold at 75 octets (RFC 5545 3.1), never inside a UTF-8 sequence."""
    data = line.encode()
    parts, limit = [], 75
    while len(data) > limit:
        cut = limit
        while cut > 0 and (data[cut] & 0xC0) == 0x80:
            cut -= 1
        parts.append(data[:cut])
        data = data[cut:]
        limit = 74  # continuation lines start with a space
    parts.append(data)
    return "\n ".join(p.decode() for p in parts)


def prop(lines: list, name: str):
    for line in lines:
        m = re.match(r"^([A-Za-z0-9-]+)(?:;[^:]*)?:(.*)$", line)
        if m and m.group(1).upper() == name:
            return m.group(2)
    return None


def parse_pairs(value: str) -> dict:
    pairs = {}
    for item in filter(None, (value or "").split(",")):
        old, sep, new = item.partition("=")
        if not sep or not old or not new:
            raise SystemExit(f"bad --rename item: {item!r}")
        pairs[old] = new
    return pairs


def seed(src: Path, dest: Path, only=None, rename=None, now=None, new_uid=None, dry_run=False):
    rename = rename or {}
    now = now or datetime.now(timezone.utc)
    new_uid = new_uid or (lambda: uuid.uuid4().hex)
    stamp = now.strftime("%Y%m%dT%H%M%SZ")

    tasks = []
    for p in sorted(src.glob("*.ics")):
        raw = p.read_bytes().decode()
        lines = unfold(raw.replace("\r\n", "\n"))
        uid, summary = prop(lines, "UID"), prop(lines, "SUMMARY")
        if uid is None:
            raise SystemExit(f"{p.name}: no UID")
        tasks.append({"lines": lines, "uid": uid, "summary": summary,
                      "eol": "\r\n" if "\r\n" in raw else "\n",
                      "parent": prop(lines, "RELATED-TO")})
    if only is not None:
        missing = set(only) - {t["summary"] for t in tasks}
        if missing:
            raise SystemExit(f"no task with SUMMARY {', '.join(sorted(missing))}")
        tasks = [t for t in tasks if t["summary"] in only]
    picked = {t["uid"] for t in tasks}
    for t in tasks:
        if t["parent"] and t["parent"] not in picked:
            raise SystemExit(f"{t['summary']}: its parent is not picked")

    mapping = {t["uid"]: new_uid() for t in tasks}
    # Longest first, so a UID that is a prefix of another cannot clobber it.
    olds = sorted(mapping, key=len, reverse=True)
    written = []
    for t in tasks:
        out = []
        for line in t["lines"]:
            name = re.match(r"^([A-Za-z0-9-]+)", line).group(1).upper()
            if name in STAMP_PROPS:
                line = f"{name}:{stamp}"
            elif name == "SUMMARY" and t["summary"] in rename:
                line = line[: len(line) - len(t["summary"])] + rename[t["summary"]]
            else:
                for old in olds:
                    line = line.replace(old, mapping[old])
            out.append(fold(line).replace("\n", t["eol"]))
        target = dest / f"{mapping[t['uid']]}.ics"
        if target.exists():
            raise SystemExit(f"{target} exists")
        if not dry_run:
            target.write_bytes((t["eol"].join(out) + t["eol"]).encode())
        written.append((rename.get(t["summary"], t["summary"]), mapping[t["uid"]]))
    return written


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("src", type=Path)
    ap.add_argument("dest", type=Path)
    ap.add_argument("--only")
    ap.add_argument("--rename")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args(argv)
    if not a.dest.is_dir():
        raise SystemExit(f"{a.dest} is not a directory")
    only = a.only.split(",") if a.only else None
    for summary, uid in seed(a.src, a.dest, only, parse_pairs(a.rename), dry_run=a.dry_run):
        print(f"{summary}\t{uid}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
