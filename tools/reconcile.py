#!/usr/bin/env python3
"""Reconcile a decoded 2Do inventory with a migrated vdir export.

Usage:
  reconcile.py <inventory.2do.json> <export.json> [--show-titles] [--config config.toml] [--tz IANA]

  inventory.2do.json  output of tools/decode_2do_dropbox.py
  export.json         output of `tdo export-json` (schema v1)

WARNING: the output contains personal data (list names, UIDs, and task titles with
--show-titles). Keep it in the terminal or outside this repository; never commit it.

Times are compared in --tz (default: `timezone` from --config, else the machine's zone).

Prints a summary, per-list counts, the special lists that are not collections, and one
line per mismatch (`<uid>: <field>: source=<v> dest=<v>`). Writes no files.
Exit status: 0 when there are no mismatches and no unmatched tasks, otherwise 1.
"""
import argparse
import json
import re
import sys
import tomllib
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from zoneinfo import ZoneInfo

DAY_ORDER = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]
# 01-features.md section 6: Sun 1, Sat 2, Fri 4, Thu 8, Wed 16, Tue 32, Mon 64.
WEEKDAY_BITS = {"SU": 1, "SA": 2, "FR": 4, "TH": 8, "WE": 16, "TU": 32, "MO": 64}
SPECIAL_FLAGS = ("isinboxcal", "istodaycal", "isallcal", "isscheduled", "donecal", "starred")
NO_TIME = {"", "999999", "-1", "0"}


def _ordinal(val: int, top: int = 4) -> int:
    """Ordinal 1..top as is; anything else means last (-1).

    260-266 pass 1-5 through (5 = fifth). 268/269 ("ordinal, else last") pass 1-4; their
    ordinal 5 is ambiguous (unverified) and is handled by the comparison.
    """
    return val if 1 <= val <= top else -1


def repeat_to_rrule(repeattype, repeatval, endtype, remaining, enddate):
    """Expected RRULE for a 2Do rule (01-features.md section 6), or None for no/unknown rule."""
    interval = repeatval if repeatval and repeatval > 1 else 1
    iv = f";INTERVAL={interval}" if interval > 1 else ""
    if 1 <= repeattype <= 127:
        days = ",".join(d for d in DAY_ORDER if repeattype & WEEKDAY_BITS[d])
        rule = f"FREQ=WEEKLY;BYDAY={days}"
    elif 256 <= repeattype <= 259:
        rule = f"FREQ={['DAILY', 'WEEKLY', 'MONTHLY', 'YEARLY'][repeattype - 256]}{iv}"
    elif 260 <= repeattype <= 266:
        day = DAY_ORDER[repeattype - 260]
        rule = f"FREQ=MONTHLY;BYDAY={_ordinal(repeatval, 5)}{day}"
    elif repeattype == 267:
        rule = f"FREQ=MONTHLY{iv};BYMONTHDAY=-1"
    elif repeattype == 268:
        rule = f"FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS={_ordinal(repeatval)}"
    elif repeattype == 269:
        rule = f"FREQ=MONTHLY;BYDAY=SA,SU;BYSETPOS={_ordinal(repeatval)}"
    else:
        return None
    if endtype == 2 and remaining is not None:
        rule += f";COUNT={remaining}"
    elif endtype == 1 and enddate:
        rule += f";UNTIL={re.sub(r'\D', '', enddate)[:8]}"
    return rule


def canonical_rrule(s: str) -> str:
    """Order-independent form: uppercase keys, sorted parts, INTERVAL=1 dropped,
    UNTIL reduced to its date, weekly-on-all-7-days folded into DAILY."""
    parts = {}
    for item in s.split(";"):
        if not item.strip():
            continue
        k, _, v = item.partition("=")
        parts[k.strip().upper()] = v.strip().upper()
    if parts.get("INTERVAL") == "1":
        del parts["INTERVAL"]
    if "UNTIL" in parts:
        parts["UNTIL"] = parts["UNTIL"][:8]
    if "BYDAY" in parts:
        days = parts["BYDAY"].split(",")
        if all(d in DAY_ORDER for d in days):
            days = [d for d in DAY_ORDER if d in days]
        parts["BYDAY"] = ",".join(days)
        if parts.get("FREQ") == "WEEKLY" and days == DAY_ORDER and "INTERVAL" not in parts:
            parts["FREQ"] = "DAILY"
            del parts["BYDAY"]
    return ";".join(f"{k}={parts[k]}" for k in sorted(parts))


def split_end(canon: str):
    """(rule without COUNT/UNTIL, 'COUNT=3' / 'UNTIL=...' / '')."""
    base, end = [], []
    for p in canon.split(";"):
        (end if p.startswith(("COUNT=", "UNTIL=")) else base).append(p)
    return ";".join(base), ";".join(end)


def source_due_time(duetime):
    """HHMM string to 'HH:MM'; 999999, -1, 0 and empty mean no time."""
    v = (duetime or "").strip()
    if v in NO_TIME:
        return None
    try:
        n = int(v)
    except ValueError:
        raise ValueError(f"invalid duetime {v!r}") from None
    if not (0 <= n <= 2359 and n % 100 < 60):
        raise ValueError(f"invalid duetime {v!r}")
    return f"{n // 100:02d}:{n % 100:02d}"


def norm_title(s):
    return " ".join((s or "").casefold().split())


def _int(v, default=0):
    try:
        return int(float(v))
    except (TypeError, ValueError):
        return default


def _parse_dest(v, tz):
    """Exported value to (date, 'HH:MM' or None) in tz. Aware values are converted to tz;
    floating values are taken as wall time; date-only values have no time."""
    if not v:
        return None, None
    if "T" not in v:
        return v[:10], None
    d = datetime.fromisoformat(v)
    if d.tzinfo is not None:
        d = d.astimezone(tz)
    return d.date().isoformat(), d.strftime("%H:%M")


def dest_date(v, tz):
    return _parse_dest(v, tz)[0]


def dest_time(v, is_date, tz):
    return None if is_date else _parse_dest(v, tz)[1]


def local_tz(name=None):
    return ZoneInfo(name) if name else datetime.now().astimezone().tzinfo


def source_start(raw):
    """Epoch string to an aware UTC datetime, or None when absent."""
    if raw in (None, "", "0"):
        return None
    try:
        return datetime.fromtimestamp(float(raw), tz=timezone.utc)
    except (ValueError, OverflowError, OSError):
        return None


def start_matches(src, dst, dst_is_date, tz):
    """Compare a source start instant with the exported start value."""
    if src is None or dst is None:
        return src is None and dst is None
    if dst_is_date:
        return src.astimezone(tz).date().isoformat() == dst[:10]
    d = datetime.fromisoformat(dst)
    if d.tzinfo is None:
        return src.astimezone(tz).replace(tzinfo=None, microsecond=0) == d
    return int(src.timestamp()) == int(d.timestamp())


def special_kind(lst):
    return lst.get("issmartcalendar") == "1" or any(lst.get(f) == "1" for f in SPECIAL_FLAGS)


def expected_rule(t, alt_ordinal5=False):
    rt = _int(t.get("repeattype"))
    if rt == 0:
        return None
    end = _int(t.get("recurrenceendtype"))
    remaining = _int(t.get("recurrenceendrepeats"), None) if t.get("recurrenceendrepeats") not in (None, "") else None
    rule = repeat_to_rrule(rt, _int(t.get("repeatval")), end, remaining, enddate_of(t))
    if alt_ordinal5 and rule:
        rule = rule.replace("BYSETPOS=-1", "BYSETPOS=5")
    return rule


def enddate_of(t):
    raw = t.get("recurrenceenddate") or ""
    if re.fullmatch(r"\d{9,}(\.\d*)?", raw):
        return datetime.fromtimestamp(float(raw), tz=timezone.utc).date().isoformat()
    return raw


def compare(s, d, uid_map, tz, notes):
    """Yield (field, source_value, dest_value) for each difference; append to notes."""
    uid = s.get("uid")
    s_done = s.get("iscomp") == "1"
    d_done = d.get("status") == "completed"
    if s_done != d_done:
        yield "completed", s_done, d_done
    s_due, d_due = (s.get("due") or None), dest_date(d.get("due"), tz)
    if s_due != d_due:
        yield "due", s_due, d_due
    try:
        s_time = source_due_time(s.get("duetime")) if s_due else None
    except ValueError as e:
        notes.append(f"warning: {uid}: {e}; treated as no time")
        s_time = None
    d_time = dest_time(d.get("due"), d.get("due_is_date"), tz)
    if s_time != d_time:
        yield "due_time", s_time, d_time
    s_start = source_start(s.get("startdatetime"))
    if not start_matches(s_start, d.get("start"), d.get("start_is_date"), tz):
        yield "start", s_start.isoformat() if s_start else None, d.get("start")

    rt = _int(s.get("repeattype"))
    want = expected_rule(s)
    if rt != 0 and want is None:
        yield "repeat", f"unmapped repeattype {rt}", d.get("rrule")
    else:
        sb, se = split_end(canonical_rrule(want)) if want else ("", "")
        db, de = split_end(canonical_rrule(d["rrule"])) if d.get("rrule") else ("", "")
        if rt in (268, 269) and _int(s.get("repeatval")) == 5:
            notes.append(f"note: {uid}: ambiguous ordinal 5 (unverified); accepting BYSETPOS=5 or -1")
            alt = split_end(canonical_rrule(expected_rule(s, True)))[0]
            if db == alt:
                sb = db
        if sb != db:
            yield "repeat", want, d.get("rrule")
        if se != de:
            yield "repeat_end", se or None, de or None
    if rt != 0 and (s.get("recurrence") == "2") != bool(d.get("from_completion")):
        yield "from_completion", s.get("recurrence") == "2", bool(d.get("from_completion"))

    s_parent = s.get("parent") or None
    s_parent = uid_map.get(s_parent, s_parent)
    if s_parent != (d.get("parent") or None):
        yield "parent", s_parent, d.get("parent") or None
    n_src = sum(1 for a in s.get("alarms") or [] if a.get("deleted") != "1")
    n_dst = len(d.get("alarms") or [])
    if n_src != n_dst:
        yield "alarms", n_src, n_dst


def reconcile(inv, exp, config=None, show_titles=False, tz=None):
    tz = tz or local_tz()
    notes = []
    out = []
    lists = inv.get("lists", [])
    list_title = {l.get("uid"): l.get("title", "") for l in lists}
    real_lists = [l for l in lists if not special_kind(l)]
    special = [l for l in lists if special_kind(l)]
    col_name = {c["id"]: c.get("display_name") or c["id"] for c in exp.get("collections", [])}

    tasks = inv.get("tasks", [])
    deleted = [t for t in tasks if t.get("deleted") == "1"]
    src = [t for t in tasks if t.get("deleted") != "1"]
    completed = [t for t in src if t.get("iscomp") == "1"]
    children = [t for t in src if t.get("parent")]
    dst = exp.get("tasks", [])

    dst_by_uid = {d["uid"]: d for d in dst}
    pairs, rest = [], []
    for t in src:
        d = dst_by_uid.pop(t["uid"], None)
        (pairs if d else rest).append((t, d))
    by_uid = len(pairs)
    pool = defaultdict(list)
    for d in dst_by_uid.values():
        pool[(norm_title(col_name.get(d.get("collection"), "")), norm_title(d.get("summary")),
              dest_date(d.get("due"), tz))].append(d)
    unmatched_src = []
    fallback = 0
    for t, _ in rest:
        key = (norm_title(list_title.get(t.get("caluid"), "")), norm_title(t.get("title")),
               t.get("due") or None)
        if pool.get(key):
            pairs.append((t, pool[key].pop(0)))
            fallback += 1
        else:
            unmatched_src.append(t)
    unmatched_dst = [d for v in pool.values() for d in v]
    uid_map = {t["uid"]: d["uid"] for t, d in pairs}

    mism = []
    def name(t):
        return f" ({t.get('title', t.get('summary', ''))})" if show_titles else ""
    for t in unmatched_src:
        mism.append(f"{t['uid']}: presence: source=present dest=absent{name(t)}")
    for d in unmatched_dst:
        mism.append(f"{d['uid']}: presence: source=absent dest=present{name(d)}")
    for t, d in pairs:
        for field, a, b in compare(t, d, uid_map, tz, notes):
            mism.append(f"{t['uid']}: {field}: source={a} dest={b}{name(t)}")
    prefs = (config or {}).get("lists", {})
    by_name = {v: k for k, v in col_name.items()}
    for l in real_lists:
        s_arch = l.get("isarchived") == "1"
        cid = by_name.get(l.get("title", ""))
        suffix = f" ({l.get('title')})" if show_titles else ""
        if cid is None:
            if s_arch:
                mism.append(f"{l['uid']}: archived: source=1 dest=no matching collection{suffix}")
        elif config is not None:
            d_arch = bool(prefs.get(cid, {}).get("archived", False))
            if s_arch != d_arch:
                mism.append(f"{l['uid']}: archived: source={int(s_arch)} dest={int(d_arch)}{suffix}")

    out.append("== Summary ==")
    out.append(f"source live: {len(src) - len(completed)}")
    occ = [t for t in completed if t.get("ruid")]
    out.append(f"source completed: {len(completed)}")
    out.append(f"  completed (repeat occurrences, ruid set): {len(occ)}")
    out.append(f"  completed (other): {len(completed) - len(occ)}")
    out.append(f"source children: {len(children)}")
    out.append(f"source deleted-skipped: {len(deleted)}")
    out.append(f"destination tasks: {len(dst)}")
    out.append(f"matched-by-uid: {by_uid}")
    out.append(f"matched-by-fallback: {fallback}")
    out.append(f"unmatched source: {len(unmatched_src)}")
    out.append(f"unmatched destination: {len(unmatched_dst)}")
    out.append(f"mismatches: {len(mism)}")

    out.append("")
    out.append("== Per-list counts (source live/completed, destination tasks) ==")
    s_cnt = defaultdict(Counter)
    for t in src:
        s_cnt[list_title.get(t.get("caluid"), "(unknown list)")]["done" if t.get("iscomp") == "1" else "live"] += 1
    d_cnt = Counter(col_name.get(d.get("collection"), "(unknown collection)") for d in dst)
    for title in sorted(set(s_cnt) | set(d_cnt) | {l.get("title", "") for l in real_lists}):
        c = s_cnt.get(title, Counter())
        out.append(f"{title}: source {c['live']}/{c['done']} dest {d_cnt.get(title, 0)}")

    out.append("")
    out.append("== Special lists (not a collection; no vdir counterpart expected) ==")
    refs = Counter(t.get("caluid") for t in src)
    for l in sorted(special, key=lambda l: l.get("title", "")):
        why = ("smart list: saved search, no collection" if l.get("issmartcalendar") == "1"
               else "built-in view in Twydo; tasks live in their real lists")
        out.append(f"{l.get('title', '')} [{l.get('uid', '')}]: "
                   f"{refs.get(l.get('uid'), 0)} source tasks reference it; {why}")
    if not special:
        out.append("(none)")

    if notes:
        out.append("")
        out.append("== Notes and warnings ==")
        out.extend(notes)

    out.append("")
    out.append("== Mismatches ==")
    out.extend(mism or ["(none)"])
    return out, len(mism)


def main(argv=None):
    ap = argparse.ArgumentParser(description="Compare a 2Do inventory with a tdo export.")
    ap.add_argument("inventory")
    ap.add_argument("export")
    ap.add_argument("--show-titles", action="store_true")
    ap.add_argument("--config", help="Twydo config.toml (for list archived preferences)")
    ap.add_argument("--tz", help="IANA timezone for time comparison (default: config timezone, else local)")
    args = ap.parse_args(argv)
    inv = json.loads(Path(args.inventory).read_text())
    exp = json.loads(Path(args.export).read_text())
    if exp.get("schema") != 1:
        print(f"unsupported export schema {exp.get('schema')!r}", file=sys.stderr)
        return 2
    config = tomllib.loads(Path(args.config).read_text()) if args.config else None
    try:
        tz = local_tz(args.tz or (config or {}).get("timezone"))
    except Exception as e:
        print(f"invalid timezone: {e}", file=sys.stderr)
        return 2
    lines, n = reconcile(inv, exp, config, args.show_titles, tz)
    print("\n".join(lines))
    return 0 if n == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
