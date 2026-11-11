#!/usr/bin/env python3
"""Capture selected resources into a complete operation evidence state, or privacy-scan staging.

capture <vdir> <out-root> --operation move --case family --state before
  --collection list1 --collection list2 --resource list1/parent.ics
  --resource list1/child.ics --device iphone --origin device
  --claim "Move preserves fields" --test path/to/test.rs::test_name --action "Observed action"
scan <directory>

Enumerate every family member explicitly. Empty declared collections represent absence.
Raw staging stays outside git; fill provenance/versions and obtain synthetic-content
confirmation before promotion. No previous-state lookup, diffs, network or source writes.
"""
import argparse
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

if not __package__:
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from tools.evidence import OPERATIONS, inside

ALLOWED_FILES = {"displayname", "color", "case.toml", ".gitkeep"}
FORBIDDEN_PROPS = {"ORGANIZER", "ATTENDEE", "X-APPLE-ABUID", "X-APPLE-STRUCTURED-LOCATION",
                   "LOCATION", "URL"}
EMAIL_RE = re.compile(r"\w[\w.+-]*@[\w-]+(?:\.[\w-]+)*")
URL_RE = re.compile(r"[A-Za-z][A-Za-z0-9+.-]*://([^/\s\"'<>?#]*)")
PROP_RE = re.compile(r"^([A-Za-z0-9-]+)[;:]")
SAFE_HOSTS = ("example.com", "example.org")
CONFIRM_PROPS = {"SUMMARY", "DESCRIPTION", "COMMENT", "CATEGORIES"}
REMINDER = "Confirm every line above is synthetic before committing."


def toml_str(value: str) -> str:
    out = []
    for ch in value:
        if ch == "\\":
            out.append("\\\\")
        elif ch == '"':
            out.append('\\"')
        elif ord(ch) < 0x20 or ord(ch) == 0x7F:
            out.append({"\n": "\\n", "\t": "\\t", "\r": "\\r"}.get(ch, "\\u%04X" % ord(ch)))
        else:
            out.append(ch)
    return '"' + "".join(out) + '"'


def cmd_capture(args) -> int:
    # Validate the whole selection before creating anything. Staging is local and
    # explicitly bounded; the helper never discovers or contacts a server.
    try:
        import tomllib
        for name in (args.case, args.state):
            if not re.fullmatch(r"[a-z][a-z0-9-]*", name):
                raise ValueError(f"invalid case/state name: {name!r}")
        source = Path(args.vdir).resolve()
        if not source.is_dir():
            raise ValueError(f"not a vdir: {source}")
        case = inside(Path(args.out_root), f"{args.operation}/{args.case}")
        state = inside(case, args.state)
        if source.is_relative_to(case.resolve()) or case.resolve().is_relative_to(source):
            raise ValueError("source and capture destination must be separate")
        if state.exists():
            raise ValueError(f"state already exists: {state}")
        collections = sorted(set(args.collection))
        for collection in collections:
            if not re.fullmatch(r"[A-Za-z0-9_-]+", collection):
                raise ValueError(f"invalid collection: {collection!r}")
            if not inside(source, collection).is_dir():
                raise ValueError(f"missing collection: {collection}")
        members = sorted(set(args.resource or []))
        captured = {}
        for member in members:
            path = inside(source, member)
            parts = Path(member).parts
            if len(parts) != 2 or parts[0] not in collections:
                raise ValueError(f"resource outside declared collections: {member}")
            if path.suffix != ".ics" and path.name not in ("displayname", "color"):
                raise ValueError(f"not a vdir resource: {member}")
            if path.is_symlink():
                raise ValueError(f"symlink resource: {member}")
            captured[member] = path.read_bytes()
        manifest = case / "case.toml"
        if manifest.exists():
            metadata = tomllib.loads(manifest.read_text())
            if metadata["claim"] != args.claim or metadata["origin"] != args.origin:
                raise ValueError("existing case claim/origin differs")
            prefix = manifest.read_text()
        else:
            prefix = (f"claim = {toml_str(args.claim)}\norigin = {toml_str(args.origin)}\n"
                      f"observed = {toml_str(args.action)}\n"
                      f"tests = [{', '.join(toml_str(t) for t in args.test)}]\n"
                      '[provenance]\nsource_revision = "unknown"\nsource_path = "external staged capture"\n'
                      '[environment]\nclient_version = "unknown"\nserver_version = "unknown"\n')
        import json
        record = (f"\n[states.{toml_str(args.state)}]\npath = {toml_str(args.state)}\n"
                  f"collections = {json.dumps(collections)}\nmembers = {json.dumps(members)}\n"
                  f"action = {toml_str(args.action)}\ndevice = {toml_str(args.device)}\n"
                  f"captured_at = {toml_str(datetime.now(timezone.utc).isoformat())}\n")
        for collection in collections:
            inside(state, collection).mkdir(parents=True, exist_ok=True)
        for member, contents in captured.items():
            inside(state, member).write_bytes(contents)
        for collection in collections:
            if not any(Path(m).parts[0] == collection for m in members):
                inside(state, collection + "/.gitkeep").write_bytes(b"")
        manifest.write_text(prefix + record, encoding="utf-8")
    except (ValueError, OSError, KeyError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    print(f"captured {len(captured)} selected resources to {state}")
    print("Fill unknown provenance/versions, scan, and confirm synthetic content before promotion.")
    return 0


def logical_lines(text: str, unfold: bool):
    """Yield (physical line number where the logical line starts, content)."""
    result = []
    for n, line in enumerate(text.splitlines(), 1):
        if unfold and result and line[:1] in (" ", "\t"):
            result[-1][1] += line[1:]
        else:
            result.append([n, line])
    return [(n, s) for n, s in result]


def host_ok(host: str) -> bool:
    host = host.rsplit("@", 1)[-1].split(":")[0].lower()
    return any(host == h or host.endswith("." + h) for h in SAFE_HOSTS)


def cmd_scan(args) -> int:
    root = Path(args.dir)
    if not root.is_dir():
        print(f"error: not a directory: {root}", file=sys.stderr)
        return 2
    paths = sorted(p for p in root.rglob("*") if p.is_file())
    if not paths:
        print("error: no files to scan", file=sys.stderr)
        return 2
    findings, summaries = [], {}
    for path in paths:
        rel = path.relative_to(root)
        if path.name not in ALLOWED_FILES and path.suffix != ".ics":
            findings.append(f"{rel}:1: file type not allowed in fixtures")
            continue
        is_ics = path.suffix == ".ics"
        text = path.read_text(encoding="utf-8", errors="replace")
        if path.name == ".gitkeep" and path.stat().st_size:
            findings.append(f"{rel}:1: nonempty collection marker")
        if path.name == "case.toml":
            summaries.setdefault(("CASE", text.strip()), None)
        if path.name == "displayname":
            summaries.setdefault(("DISPLAYNAME", text.strip()), None)
        for n, line in logical_lines(text, unfold=is_ics):
            m = PROP_RE.match(line)
            name = m.group(1).upper() if m else ""
            if name in FORBIDDEN_PROPS:
                findings.append(f"{rel}:{n}: forbidden property {name}")
            if EMAIL_RE.search(line):
                findings.append(f"{rel}:{n}: email address")
            for um in URL_RE.finditer(line):
                if not host_ok(um.group(1)):
                    findings.append(f"{rel}:{n}: URL outside example.com/example.org")
            if "@actionvalue:" in line:
                findings.append(f"{rel}:{n}: @actionvalue: metadata")
            if is_ics and (name in CONFIRM_PROPS or name.startswith("X-")):
                summaries.setdefault((name, line.split(":", 1)[1] if ":" in line else ""), None)
    for f in findings:
        print(f)
    for name, value in sorted(summaries):
        print(f"{name}: {value}")
    print(REMINDER)
    return 1 if findings else 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("capture")
    c.add_argument("vdir")
    c.add_argument("out_root")
    c.add_argument("--operation", required=True, choices=sorted(OPERATIONS))
    c.add_argument("--case", required=True)
    c.add_argument("--state", required=True)
    c.add_argument("--collection", required=True, action="append")
    c.add_argument("--resource", action="append")
    c.add_argument("--device", required=True, choices=["mac", "iphone", "twydo", "server"])
    c.add_argument("--origin", required=True, choices=["device", "synthetic", "protocol"])
    c.add_argument("--claim", required=True)
    c.add_argument("--test", required=True, action="append")
    c.add_argument("--action", required=True)
    s = sub.add_parser("scan")
    s.add_argument("dir")
    args = ap.parse_args(argv)
    return cmd_capture(args) if args.cmd == "capture" else cmd_scan(args)


if __name__ == "__main__":
    sys.exit(main())
