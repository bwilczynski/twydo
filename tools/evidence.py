"""Focused evidence manifests. Validation is repository-only, never runtime policy."""
from pathlib import Path
import re
import tomllib

OPERATIONS = frozenset((
    "create", "edit-title", "edit-notes", "set-due", "set-start", "set-repeat",
    "convert", "complete", "reopen", "move", "delete", "reorder", "insert",
    "reparent", "list-create", "list-metadata", "list-delete",
))
ORIGINS = {"device", "synthetic", "protocol"}
CASE_FIELDS = {"claim", "origin", "observed", "tests", "provenance", "environment", "states", "supports"}


def inside(base: Path, value: str) -> Path:
    """Resolve only relative paths whose full resolution stays below base."""
    path = Path(value)
    if path.is_absolute() or ".." in path.parts or not value:
        raise ValueError(f"unsafe path: {value!r}")
    result = base / path
    if not result.resolve().is_relative_to(base.resolve()):
        raise ValueError(f"path escapes through a symlink: {value!r}")
    return result


def load_case(root: Path, case_id: str) -> dict:
    parts = Path(case_id).parts
    if len(parts) != 2 or parts[0] not in OPERATIONS:
        raise ValueError(f"unknown operation/case path: {case_id!r}")
    path = inside(root / "fixtures", case_id) / "case.toml"
    return tomllib.loads(path.read_text())


def validate(root: Path) -> list[str]:
    errors = []
    cases = {}
    owned_files = set()
    for manifest in sorted((root / "fixtures").glob("*/*/case.toml")):
        case_id = manifest.parent.relative_to(root / "fixtures").as_posix()
        try:
            case = load_case(root, case_id)
            cases[case_id] = case
            unknown = set(case) - CASE_FIELDS
            if unknown:
                errors.append(f"{case_id}: unknown fields: {sorted(unknown)}")
            for key in ("claim", "observed"):
                if not isinstance(case.get(key), str) or not case[key].strip():
                    errors.append(f"{case_id}: missing {key}")
            if case.get("origin") not in ORIGINS:
                errors.append(f"{case_id}: unknown origin")
            provenance = case.get("provenance", {})
            for key in ("source_revision", "source_path"):
                if not isinstance(provenance.get(key), str) or not provenance[key]:
                    errors.append(f"{case_id}: missing provenance.{key}; use unknown when unavailable")
            tests = case.get("tests", [])
            if not tests:
                errors.append(f"{case_id}: missing regression test references")
            for test in tests:
                path, _, name = test.partition("::")
                target = inside(root, path)
                if not target.is_file() or (name and not re.search(
                        r"\b(?:fn|def)\s+" + re.escape(name) + r"\s*\(", target.read_text())):
                    errors.append(f"{case_id}: missing test {test}")
            owned = {manifest.resolve()}
            state_paths = set()
            for name, state in case.get("states", {}).items():
                unknown = set(state) - {"path", "collections", "members", "action", "device", "captured_at"}
                if unknown:
                    errors.append(f"{case_id}/{name}: unknown state fields: {sorted(unknown)}")
                base = inside(manifest.parent, state["path"])
                if base.resolve() in state_paths:
                    errors.append(f"{case_id}/{name}: duplicate state path")
                state_paths.add(base.resolve())
                collections = state.get("collections", [])
                members = state.get("members", [])
                if len(members) != len(set(members)) or len(collections) != len(set(collections)):
                    errors.append(f"{case_id}/{name}: duplicate membership")
                for collection in collections:
                    if not inside(base, collection).is_dir():
                        errors.append(f"{case_id}/{name}: missing collection {collection}")
                for member in members:
                    path = inside(base, member)
                    if not path.is_file():
                        errors.append(f"{case_id}/{name}: missing resource {member}")
                    if not any(Path(member).is_relative_to(Path(c)) for c in collections):
                        errors.append(f"{case_id}/{name}: resource outside declared collections: {member}")
                    if path.resolve() in owned:
                        errors.append(f"{case_id}/{name}: resource owned twice: {member}")
                    owned.add(path.resolve())
            for path in manifest.parent.rglob("*"):
                if path.is_symlink():
                    inside(manifest.parent, path.relative_to(manifest.parent).as_posix())
                if path.name == ".gitkeep" and path.is_file() and path.read_bytes() == b"":
                    # Git needs a zero-byte marker to retain a declared empty
                    # collection. It is bookkeeping, never a task resource.
                    for state in case.get("states", {}).values():
                        base = inside(manifest.parent, state["path"])
                        if any(path.parent == inside(base, c) and not any(
                                Path(member).is_relative_to(Path(c))
                                for member in state.get("members", []))
                               for c in state.get("collections", [])):
                            owned.add(path.resolve())
                if path.is_file() and path.resolve() not in owned:
                    errors.append(f"{case_id}: unowned resource {path.relative_to(manifest.parent)}")
            owned_files.update(owned)
        except (OSError, ValueError, KeyError, TypeError) as error:
            errors.append(f"{case_id}: {error}")

    for path in (root / "fixtures").rglob("*"):
        if path.is_symlink():
            errors.append(f"{path.relative_to(root)}: symlink evidence is not self-contained")
        if path.is_file() and path.resolve() not in owned_files:
            errors.append(f"{path.relative_to(root)}: unowned evidence file")

    matrix_path = root / "compat/matrix.toml"
    if matrix_path.exists():
        try:
            matrix = tomllib.loads(matrix_path.read_text())
            vocabulary = (root / "crates/twydo-core/src/compat/contracts.rs").read_text()
            known = set(re.findall(r'\w+\s*=>\s*\w+,\s*"([a-z][a-z0-9.-]+)";', vocabulary))
            seen = set()
            for contract in matrix.get("contract", []):
                cid = contract["id"]
                operation = cid.partition(".")[0]
                if cid not in known or operation not in OPERATIONS or cid in seen:
                    errors.append(f"unknown or duplicate contract ID: {cid}")
                seen.add(cid)
                refs = contract.get("evidence", [])
                device = False
                protocol = False
                for ref in refs:
                    case = cases.get(ref)
                    if case is None:
                        errors.append(f"{cid}: missing evidence {ref}")
                        continue
                    if not ref.startswith(operation + "/") and cid not in case.get("supports", []):
                        errors.append(f"{cid}: cross-operation evidence {ref} needs an explicit supports claim")
                    device |= case.get("origin") == "device"
                    protocol |= case.get("origin") == "protocol"
                if contract.get("atomicity") == "pass" and not protocol:
                    errors.append(f"{cid}: atomicity needs a protocol observation")
                if any(contract.get(c) == "pass" for c in
                       ("iphone_baseline", "twydo_to_iphone", "iphone_to_twydo")) and not device:
                    errors.append(f"{cid}: passing phone requirement needs a device observation")
            # A case may own parser/model assertions without supplying admission proof.
            # It is still owned through its checked regression references.
        except (OSError, ValueError, KeyError, TypeError) as error:
            errors.append(f"matrix: {error}")
    return errors
