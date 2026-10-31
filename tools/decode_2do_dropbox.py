#!/usr/bin/env python3
"""Decode a 2Do Dropbox sync folder into JSON.

Each file in tod/ cat/ col/ loc/ cal/ is gzip(NSKeyedArchiver binary plist) whose
root object "data" is a flat dictionary (values are mostly strings, alarms are a
nested array of dictionaries).

Usage:
  decode_2do_dropbox.py <2Do folder> <inventory.2do.json>     # decode everything to JSON
  decode_2do_dropbox.py --roundtrip <2Do folder>    # verify archive() re-encodes identically

Folders: tod (tasks), col (lists), cat (tags), cal (list groups), loc (locations).
Format details: docs/reference/2do/05-migrating-your-data.md and 06-sync.md §2.7.

The input folder and the JSON output contain personal task data: keep both outside
this repository (see .gitignore).
"""
import gzip
import json
import plistlib
import sys
from pathlib import Path

FOLDERS = {"tod": "tasks", "cat": "tags", "col": "lists", "loc": "locations", "cal": "list_groups"}


def unarchive(blob: bytes):
    pl = plistlib.loads(blob)
    objects = pl["$objects"]

    def resolve(v):
        if isinstance(v, plistlib.UID):
            return resolve(objects[v.data])
        if isinstance(v, dict):
            if "NS.keys" in v and "NS.objects" in v:
                return {resolve(k): resolve(o) for k, o in zip(v["NS.keys"], v["NS.objects"])}
            if "NS.objects" in v:
                return [resolve(o) for o in v["NS.objects"]]
            if "NS.string" in v:
                return v["NS.string"]
            if "NS.data" in v:
                return v["NS.data"].hex()
            return {k: resolve(x) for k, x in v.items() if k != "$class"}
        if isinstance(v, list):
            return [resolve(x) for x in v]
        if v == "$null":
            return None
        if isinstance(v, bytes):
            return v.hex()
        return v

    return resolve(pl["$top"]["data"])


def archive(record: dict) -> bytes:
    """Encode one record the way 2Do writes it: gzip(NSKeyedArchiver bplist).

    Values must be strings, except lists of dicts (e.g. task "alarms").
    Equal strings share one $objects slot, as Apple's archiver does.
    """
    objects = ["$null"]
    index = {}

    def ref_string(s):
        if s not in index:
            index[s] = len(objects)
            objects.append(s)
        return plistlib.UID(index[s])

    def add(value):
        if isinstance(value, str):
            return ref_string(value)
        slot = len(objects)
        objects.append(None)  # reserve, so containers precede their contents
        if isinstance(value, dict):
            keys = [ref_string(k) for k in value]
            vals = [add(v) for v in value.values()]
            objects[slot] = {"NS.keys": keys, "NS.objects": vals, "$class": add_class("NSMutableDictionary", "NSDictionary")}
        elif isinstance(value, list):
            objects[slot] = {"NS.objects": [add(v) for v in value], "$class": add_class("NSMutableArray", "NSArray")}
        else:
            raise TypeError(f"unsupported value {value!r}")
        return plistlib.UID(slot)

    classes = {}

    def add_class(name, parent):
        if name not in classes:
            classes[name] = len(objects)
            objects.append({"$classname": name, "$classes": [name, parent, "NSObject"]})
        return plistlib.UID(classes[name])

    root = add({k: v for k, v in record.items() if not k.startswith("_")})
    pl = {"$version": 100000, "$archiver": "NSKeyedArchiver", "$top": {"data": root}, "$objects": objects}
    return gzip.compress(plistlib.dumps(pl, fmt=plistlib.FMT_BINARY), mtime=0)


def roundtrip(root: str):
    """Decode every file, re-encode it, decode again and compare."""
    bad = 0
    total = 0
    for folder in FOLDERS:
        for f in sorted((Path(root) / folder).glob("*")):
            if f.name.startswith("."):
                continue
            total += 1
            original = unarchive(gzip.decompress(f.read_bytes()))
            again = unarchive(gzip.decompress(archive(original)))
            if again != original:
                bad += 1
                print("mismatch:", f)
    print(f"round-trip: {total - bad}/{total} identical")


def main(root: str, out: str):
    result = {name: [] for name in FOLDERS.values()}
    errors = []
    for folder, name in FOLDERS.items():
        for f in sorted((Path(root) / folder).glob("*")):
            if f.name.startswith("."):
                continue
            try:
                raw = f.read_bytes()
                data = gzip.decompress(raw) if raw[:2] == b"\x1f\x8b" else raw
                rec = unarchive(data)
                rec["_file"] = f"{folder}/{f.name}"
                result[name].append(rec)
            except Exception as e:  # keep going, report at the end
                errors.append(f"{folder}/{f.name}: {e}")
    Path(out).write_text(json.dumps(result, indent=1, ensure_ascii=False, sort_keys=True))
    print({k: len(v) for k, v in result.items()}, "errors:", len(errors))
    for e in errors[:10]:
        print("  ", e)


if __name__ == "__main__":
    if sys.argv[1] == "--roundtrip":
        roundtrip(sys.argv[2])
    else:
        main(sys.argv[1], sys.argv[2])
