#!/usr/bin/env python3
"""Phase-0 fixture capture and privacy scan.

Usage:
  capture_fixture.py capture <collection-dir> <out-root> --scenario <id> --step <NN-slug>
                     --device mac|iphone|twydo --action "<text>"
  capture_fixture.py scan <dir>

capture snapshots *.ics, displayname and color from a vdirsyncer collection folder into
<out-root>/<scenario>/<step>/, writes step.toml and changes.diff (against the lexically
greatest earlier step of the scenario). The source folder is only read.
scan checks staged fixtures for personal data; exit 1 if anything is flagged.
Never commit fixtures before scan passes and a human confirms the content is synthetic.
"""
import argparse
import difflib
import re
import shutil
import sys
from datetime import datetime, timezone
from pathlib import Path

ALLOWED_FILES = {"displayname", "color", "step.toml", "changes.diff"}
FORBIDDEN_PROPS = {"ORGANIZER", "ATTENDEE", "X-APPLE-ABUID", "X-APPLE-STRUCTURED-LOCATION",
                   "LOCATION", "URL"}
EMAIL_RE = re.compile(r"[\w.+-]+@[\w-]+(?:\.[\w-]+)+")
URL_RE = re.compile(r"[A-Za-z][A-Za-z0-9+.-]*://([^/\s\"'<>?#]*)")
PROP_RE = re.compile(r"^([A-Za-z0-9-]+)[;:]")
SAFE_HOSTS = ("example.com", "example.org")
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


def collect(src: Path) -> dict:
    files = {}
    for p in sorted(src.iterdir()):
        if p.is_file() and (p.suffix == ".ics" or p.name in ("displayname", "color")):
            files[p.name] = p.read_bytes()
    return files


def previous_step(scenario_dir: Path, step: str):
    if not scenario_dir.is_dir():
        return None
    names = sorted(d.name for d in scenario_dir.iterdir() if d.is_dir() and d.name < step)
    return scenario_dir / names[-1] if names else None


def build_diff(prev: dict, new: dict) -> str:
    out = []
    for name in sorted(set(prev) | set(new)):
        if name not in prev:
            out.append(f"added: {name}\n")
        elif name not in new:
            out.append(f"removed: {name}\n")
        elif prev[name] != new[name]:
            a = prev[name].decode("utf-8", "replace").splitlines(keepends=True)
            b = new[name].decode("utf-8", "replace").splitlines(keepends=True)
            for line in difflib.unified_diff(a, b, f"a/{name}", f"b/{name}"):
                out.append(line if line.endswith("\n") else line + "\n")
    return "".join(out)


def cmd_capture(args) -> int:
    src = Path(args.collection)
    new = collect(src)
    scenario_dir = Path(args.out_root) / args.scenario
    step_dir = scenario_dir / args.step
    prev_dir = previous_step(scenario_dir, args.step)
    prev = collect(prev_dir) if prev_dir else {}
    step_dir.mkdir(parents=True, exist_ok=True)
    for name, data in new.items():
        (step_dir / name).write_bytes(data)
    captured = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    (step_dir / "step.toml").write_text(
        f"scenario = {toml_str(args.scenario)}\nstep = {toml_str(args.step)}\n"
        f"device = {toml_str(args.device)}\naction = {toml_str(args.action)}\n"
        f"captured_at = {toml_str(captured)}\n")
    (step_dir / "changes.diff").write_text(build_diff(prev, new))
    print(f"captured {len(new)} file(s) to {step_dir}")
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
    findings, summaries = [], {}
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        rel = path.relative_to(root)
        if path.name not in ALLOWED_FILES and path.suffix != ".ics":
            findings.append(f"{rel}:1: file type not allowed in fixtures")
            continue
        is_ics = path.suffix == ".ics"
        is_diff = path.name == "changes.diff"
        text = path.read_text(encoding="utf-8", errors="replace")
        for n, line in logical_lines(text, unfold=is_ics):
            if is_diff and line[:1] in "+- " and not line.startswith(("+++", "---")):
                line = line[1:]
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
            if is_ics and name in ("SUMMARY", "DESCRIPTION"):
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
    c.add_argument("collection")
    c.add_argument("out_root")
    c.add_argument("--scenario", required=True)
    c.add_argument("--step", required=True)
    c.add_argument("--device", required=True, choices=["mac", "iphone", "twydo"])
    c.add_argument("--action", required=True)
    s = sub.add_parser("scan")
    s.add_argument("dir")
    args = ap.parse_args(argv)
    return cmd_capture(args) if args.cmd == "capture" else cmd_scan(args)


if __name__ == "__main__":
    sys.exit(main())
