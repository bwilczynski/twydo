"""Twydo collection adapter. Bundled in the binary; requests/results use protocol 1.

Importing this module does not read config, fetch credentials or access the network.
"""
import asyncio
import base64
import contextlib
import importlib
import hashlib
import io
import json
import os
from pathlib import Path
import re
import signal
import sys
import tempfile
from urllib.parse import quote, unquote, urljoin, urlsplit
import xml.etree.ElementTree as ET

D = "{DAV:}"
C = "{urn:ietf:params:xml:ns:caldav}"
A = "{http://apple.com/ns/ical/}"
MAX_INPUT = 16 * 1024
MAX_DAV = 4 * 1024 * 1024


class Refused(Exception):
    def __init__(self, code="mapping"):
        self.code = code
        super().__init__(code)


def plain_id(value):
    return (isinstance(value, str) and 0 < len(value) <= 255
            and not value.startswith(".")
            and not any(c in value for c in "/\\%?#:\x00\r\n"))


def validate_request(request):
    if not isinstance(request, dict) or type(request.get("version")) is not int or request["version"] != 1:
        raise Refused("protocol")
    if request.get("action") not in ("probe", "resolve", "create", "edit", "preview-delete"):
        raise Refused("protocol")
    mode = request.get("mode", "execute")
    if type(request.get("timeout_secs", 300)) is not int or request.get("timeout_secs", 300) < 1:
        raise Refused("protocol")
    if mode not in ("execute", "refresh", "reconcile") or type(request.get("keep_server", False)) is not bool:
        raise Refused("protocol")
    if mode != "execute" and request.get("action") not in ("create", "edit"):
        raise Refused("protocol")
    if request.get("keep_server") and mode != "reconcile":
        raise Refused("protocol")
    for name in ("pair", "local_storage"):
        if not isinstance(request.get(name), str) or not re.fullmatch(r"[A-Za-z0-9_]+", request[name]):
            raise Refused("protocol")
    if not isinstance(request.get("vdir"), str) or not Path(request["vdir"]).is_absolute():
        raise Refused("protocol")
    if request.get("config") is not None and not isinstance(request["config"], str):
        raise Refused("protocol")
    if request.get("action") != "probe" and not plain_id(request.get("collection")):
        raise Refused("protocol")
    if request.get("action") == "preview-delete" and (not request.get("operation") or not request.get("state")):
        raise Refused("protocol")
    if request.get("action") in ("create", "edit"):
        if not request.get("operation") or not request.get("state"):
            raise Refused("protocol")
        name, color = request.get("name"), request.get("color")
        if name is not None and (not isinstance(name, str) or not name.strip() or len(name.encode()) > 1024 or any(ord(c) < 32 or ord(c) == 127 for c in name)):
            raise Refused("protocol")
        if color is not None and (not isinstance(color, str) or not re.fullmatch(r"#[0-9a-fA-F]{6}", color)):
            raise Refused("protocol")
        if request["action"] == "create" and (name is None or color is None):
            raise Refused("protocol")
        if request["action"] == "edit" and (not isinstance(request.get("expected"), dict) or (name is None and color is None)):
            raise Refused("protocol")
    return request


def atomic_json(path, value):
    path = Path(path)
    fd, temporary = tempfile.mkstemp(prefix=".collection-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(json.dumps(value, indent=2).encode())
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def local_snapshot(path):
    path = Path(path)
    if path.is_symlink():
        raise Refused("operation")
    if not path.exists():
        return None
    if not path.is_dir():
        raise Refused("operation")
    files = {}
    for entry in path.rglob("*"):
        if entry.is_symlink():
            raise Refused("operation")
        if entry.is_file():
            files[entry.relative_to(path).as_posix()] = list(entry.read_bytes())
        elif not entry.is_dir():
            raise Refused("operation")
    return files


@contextlib.contextmanager
def operation_guard(request):
    """The child owns both locks, so a dead TUI cannot release an active request's locks."""
    operation_path = request.get("operation")
    if operation_path is None:
        yield None
        return
    import fcntl
    state = Path(request.get("state") or "")
    path = Path(operation_path)
    if not state.is_absolute() or state.is_symlink() or path.is_symlink() or path.parent != state / "lists/operations":
        raise Refused("operation")
    if path.parent.is_symlink() or (state / "lists").is_symlink():
        raise Refused("operation")
    with contextlib.ExitStack() as stack:
        for lockpath, mode in ((state / "sync/run.lock", fcntl.LOCK_EX), (state / "vdir.lock", fcntl.LOCK_SH)):
            flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
            descriptor = os.open(lockpath, flags, 0o600)
            lock = stack.enter_context(os.fdopen(descriptor, "r+b"))
            # Remote phases never write the vdir. A shared lock excludes task writers
            # while keeping the TUI/notifier's consistent local readers responsive.
            fcntl.flock(lock, mode)
        record = json.loads(path.read_text())
        identity = (state / "lists/active").read_text().strip()
        vdir = Path(request["vdir"]).resolve(strict=True)
        def comparable(value):
            result = {k: v for k, v in value.items() if k not in ("mode", "keep_server") and v is not None}
            result["timeout_secs"] = value.get("timeout_secs", 300)
            return result
        mode = request.get("mode", "execute")
        phases = {"execute": ("prepared",), "refresh": ("remote-confirmed",),
                  "reconcile": ("remote-started", "remote-confirmed", "uncertain")}
        if (record.get("format") != 1 or record.get("id") != identity or path.name != identity + ".json"
                or comparable(record.get("request", {})) != comparable(request) or record.get("phase") not in phases[mode]
                or record.get("vdir") != str(vdir) or (state / "vdir").read_text().strip() != str(vdir)):
            raise Refused("operation")
        actual = local_snapshot(vdir / request["collection"])
        before, after = record.get("local_base"), record.get("local_after")
        if after is not None and mode == "refresh":
            matches = actual == after
        elif after is not None and mode == "reconcile":
            values = (actual or {}, before or {}, after)
            matches = all(values[0].get(k) in (values[1].get(k), values[2].get(k)) for k in set().union(*values))
        else:
            matches = actual == record.get("holds")
        if not plain_id(request.get("collection")) or not matches:
            raise Refused("operation")
        yield record


def load_adapter():
    try:
        vdirsyncer = importlib.import_module("vdirsyncer")
        if not re.fullmatch(r"0\.21\.\d+", vdirsyncer.__version__):
            raise Refused("dependency")
        config = importlib.import_module("vdirsyncer.cli.config")
        utils = importlib.import_module("vdirsyncer.cli.utils")
        discover = importlib.import_module("vdirsyncer.cli.discover")
        aiohttp = importlib.import_module("aiohttp")
        if not callable(getattr(config.Config, "from_filename_or_environment", None)) or not callable(getattr(utils, "storage_instance_from_config", None)):
            raise Refused("dependency")
        return config, utils, discover, aiohttp
    except (ImportError, AttributeError):
        raise Refused("dependency") from None


def remote_identity(pair, request):
    local_a = pair.name_a == request["local_storage"]
    if not local_a and pair.name_b != request["local_storage"]:
        raise Refused()
    local_id = request["collection"]
    choices = []
    dynamic = False
    if not isinstance(pair.collections, list):
        raise Refused()
    dynamic = any(item in ("from a", "from b") for item in pair.collections)
    # vdirsyncer deduplicates expansion by logical name, not the local identity.
    # Mixed aliases/shortcuts require a complete expansion, which this bounded
    # adapter does not support. Refuse before credential expansion instead of guessing.
    if dynamic and any(isinstance(item, list) for item in pair.collections):
        raise Refused()
    logical_seen, local_seen, remote_seen = set(), set(), set()
    for item in pair.collections:
        if item in ("from a", "from b"):
            continue
        if isinstance(item, str):
            logical, local, remote = item, item, item
        elif isinstance(item, list) and len(item) == 3:
            logical, local, remote = item[0], item[1 if local_a else 2], item[2 if local_a else 1]
        else:
            raise Refused()
        if not all(plain_id(value) for value in (logical, local, remote)) or logical in logical_seen or local in local_seen or remote in remote_seen:
            raise Refused()
        logical_seen.add(logical)
        local_seen.add(local)
        remote_seen.add(remote)
        if local == local_id:
            choices.append(remote)
    if len(choices) > 1:
        raise Refused()
    result = choices[0] if choices else local_id if dynamic else None
    if not plain_id(result):
        raise Refused()
    return result


def validate_target(base, target, identity):
    b, t = urlsplit(base), urlsplit(target)
    if (b.scheme not in ("https", "http") or (b.scheme, b.netloc) != (t.scheme, t.netloc)
            or t.query or t.fragment or t.username or not plain_id(identity)):
        raise Refused()
    path = unquote(t.path)
    parts = path.rstrip("/").split("/")
    if ("%" in path or "\\" in path or any(x in (".", "..") for x in parts)
            or len(parts) < 3 or parts[-1] != identity
            or re.search(r"%2f|%5c", t.path, re.I)):
        raise Refused()


async def dav_request(session, method, path="", **kwargs):
    response = await session.request(method, path, allow_redirects=False, **kwargs)
    target = urljoin(session.url, path) if path else session.url
    if str(response.url).rstrip("/") != target.rstrip("/") or 300 <= response.status < 400:
        raise Refused("dav")
    chunks, size = [], 0
    while True:
        chunk = await response.content.read(min(65536, MAX_DAV + 1 - size))
        size += len(chunk)
        if size > MAX_DAV:
            raise Refused("dav")
        if not chunk:
            break
        chunks.append(chunk)
    return response, b"".join(chunks)


def successful_properties(raw, target):
    try:
        root = ET.fromstring(raw)
        rows = root.findall(D + "response")
        if root.tag != D + "multistatus" or len(rows) != 1:
            raise Refused("dav")
        href = rows[0].findtext(D + "href")
        if not href or urljoin(target, href).rstrip("/") != target.rstrip("/"):
            raise Refused("dav")
        result = {}
        for ps in rows[0].findall(D + "propstat"):
            if (ps.findtext(D + "status") or "").split()[1:2] != ["200"]:
                continue
            prop = ps.find(D + "prop")
            if prop is None:
                raise Refused("dav")
            for value in prop:
                if value.tag in result:
                    raise Refused("dav")
                result[value.tag] = value
        return result
    except ET.ParseError:
        raise Refused("dav") from None


async def read_properties(session):
    body = b'''<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/">
      <d:prop><d:displayname/><d:resourcetype/><c:supported-calendar-component-set/>
      <a:calendar-color/></d:prop></d:propfind>'''
    response, raw = await dav_request(session, "PROPFIND", data=body,
                                     headers={"Depth": "0", "Content-Type": "application/xml; charset=utf-8"})
    if response.status != 207:
        raise Refused("dav")
    props = successful_properties(raw, session.url)
    requested = {D + "displayname", D + "resourcetype", C + "supported-calendar-component-set", A + "calendar-color"}
    seen = set()
    for ps in ET.fromstring(raw).findall(D + "response/" + D + "propstat"):
        status = (ps.findtext(D + "status") or "").split()[1:2]
        prop = ps.find(D + "prop")
        if prop is None:
            raise Refused("dav")
        for value in prop:
            if value.tag in seen:
                raise Refused("dav")
            seen.add(value.tag)
            if status != ["200"] and not (status == ["404"] and value.tag in (D + "displayname", A + "calendar-color")):
                raise Refused("dav")
    if not requested <= seen:
        raise Refused("dav")
    kind, components = props.get(D + "resourcetype"), props.get(C + "supported-calendar-component-set")
    if kind is None or kind.find(C + "calendar") is None or components is None or not any(c.get("name") == "VTODO" for c in components):
        raise Refused("dav")
    name = props.get(D + "displayname")
    color = props.get(A + "calendar-color")
    return {"name": name.text if name is not None else None, "color": color.text if color is not None else None}


def canonical_color(value):
    if isinstance(value, str) and re.fullmatch(r"#[0-9a-fA-F]{6}(?:[0-9a-fA-F]{2})?", value):
        return value[:7].upper()
    return None


def multistatus(raw, require_success=True):
    try:
        root = ET.fromstring(raw)
        if root.tag != D + "multistatus":
            raise Refused("inventory")
        rows = root.findall(D + "response")
        if not rows or any(not row.findall(D + "propstat") or row.find(D + "status") is not None for row in rows) or (require_success and any((ps.findtext(D + "status") or "").split()[1:2] != ["200"]
                           for row in rows for ps in row.findall(D + "propstat"))):
            raise Refused("inventory")
        return rows
    except ET.ParseError:
        raise Refused("inventory") from None


async def raw_properties(session):
    response, raw = await dav_request(session, "PROPFIND", data=b'<d:propfind xmlns:d="DAV:"><d:allprop/></d:propfind>',
                                     headers={"Depth": "0", "Content-Type": "application/xml"})
    if response.status != 207:
        raise Refused("inventory")
    multistatus(raw)
    successful_properties(raw, session.url)
    return raw


async def inventory(session):
    response, raw = await dav_request(session, "PROPFIND",
        data=b'<d:propfind xmlns:d="DAV:"><d:prop><d:getetag/><d:resourcetype/></d:prop></d:propfind>',
        headers={"Depth": "1", "Content-Type": "application/xml"})
    if response.status != 207:
        raise Refused("inventory")
    result, seen, collection = [], set(), 0
    base = session.url.rstrip("/") + "/"
    for row in multistatus(raw, require_success=False):
        href = row.findtext(D + "href")
        if not href:
            raise Refused("inventory")
        target = urljoin(base, href)
        if target in seen:
            raise Refused("inventory")
        seen.add(target)
        props = {}
        for ps in row.findall(D + "propstat"):
            prop = ps.find(D + "prop")
            if prop is None:
                raise Refused("inventory")
            status = (ps.findtext(D + "status") or "").split()[1:2]
            if status != ["200"]:
                if target.rstrip("/") == base.rstrip("/") and status == ["404"] and [value.tag for value in prop] == [D + "getetag"]:
                    continue
                raise Refused("inventory")
            for value in prop:
                if value.tag in props:
                    raise Refused("inventory")
                props[value.tag] = value
        kind = props.get(D + "resourcetype")
        if target.rstrip("/") == base.rstrip("/"):
            collection += 1
            if kind is None or kind.find(D + "collection") is None:
                raise Refused("inventory")
            continue
        name = target[len(base):] if target.startswith(base) else ""
        if not plain_id(unquote(name)) or "/" in name or urlsplit(target).query or urlsplit(target).fragment:
            raise Refused("inventory")
        if kind is None or list(kind):
            raise Refused("inventory")
        etag = props.get(D + "getetag")
        etag = etag.text if etag is not None else None
        if not isinstance(etag, str) or not re.fullmatch(r'"[^"\x00-\x20\x7f]+"', etag):
            raise Refused("inventory")
        result.append({"href": target, "etag": etag})
    if collection != 1:
        raise Refused("inventory")
    return sorted(result, key=lambda value: value["href"])


async def capture_inventory(session):
    properties = await raw_properties(session)
    before = await inventory(session)
    resources, total = [], 0
    for entry in before:
        response, raw = await dav_request(session, "GET", entry["href"], headers={"If-Match": entry["etag"]})
        if response.status != 200 or response.headers.get("ETag") != entry["etag"]:
            raise Refused("inventory")
        total += len(raw)
        if total > 32 * 1024 * 1024:
            raise Refused("inventory")
        resources.append(dict(entry, bytes=base64.b64encode(raw).decode(), sha256=hashlib.sha256(raw).hexdigest()))
    if before != await inventory(session) or properties != await raw_properties(session):
        raise Refused("stale")
    return {"format": 1, "properties": base64.b64encode(properties).decode(), "resources": resources}


def same_metadata(field, left, right):
    if field == "color":
        return canonical_color(left) == canonical_color(right)
    return left == right


async def write_metadata(session, changes, expected):
    """Unverified prototype: preflight/readback do not protect concurrent edits.

    The shipping service keeps list-metadata gated in every write mode until a
    server-enforced write precondition is implemented and demonstrated.
    """
    if not changes or not set(changes) <= {"name", "color"}:
        raise Refused("protocol")
    before = await read_properties(session)
    if any(not same_metadata(field, before[field], expected.get(field)) for field in changes):
        raise Refused("stale")
    root = ET.Element(D + "propertyupdate")
    properties = ET.SubElement(ET.SubElement(root, D + "set"), D + "prop")
    tags = {"name": D + "displayname", "color": A + "calendar-color"}
    for field, value in changes.items():
        ET.SubElement(properties, tags[field]).text = value
    response, raw = await dav_request(session, "PROPPATCH", data=ET.tostring(root),
                                     headers={"Content-Type": "application/xml; charset=utf-8"})
    if response.status != 207:
        raise Refused("dav")
    confirmed = successful_properties(raw, session.url)
    if any(tags[field] not in confirmed for field in changes):
        raise Refused("dav")
    after = await read_properties(session)
    if any(not same_metadata(field, after[field], value) for field, value in changes.items()):
        raise Refused("dav")
    return after


async def create_task_collection(session, name, color):
    probe = b'<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>'
    try:
        response, _ = await dav_request(session, "PROPFIND", data=probe,
                                        headers={"Depth": "0", "Content-Type": "application/xml"})
        absent = response.status == 404
    except Exception as error:
        from vdirsyncer.exceptions import NotFoundError
        if not isinstance(error, NotFoundError):
            raise
        absent = True
    if not absent:
        raise Refused("exists")
    root = ET.Element(C + "mkcalendar")
    properties = ET.SubElement(ET.SubElement(root, D + "set"), D + "prop")
    ET.SubElement(properties, D + "displayname").text = name
    ET.SubElement(properties, A + "calendar-color").text = color
    ET.SubElement(ET.SubElement(properties, C + "supported-calendar-component-set"), C + "comp", name="VTODO")
    response, _ = await dav_request(session, "MKCALENDAR", data=ET.tostring(root),
                                   headers={"Content-Type": "application/xml; charset=utf-8", "If-None-Match": "*"})
    if response.status != 201:
        raise Refused("dav")
    after = await read_properties(session)
    if after["name"] != name or canonical_color(after["color"]) != canonical_color(color):
        raise Refused("dav")
    return after


async def select_discovered(discovered, base, identity):
    selected = None
    async for args in discovered:
        if args.get("collection") != identity:
            continue
        if selected is not None:
            raise Refused()
        validate_target(base, args["url"], identity)
        selected = args
    if selected is None:
        raise Refused()
    return selected


def pair_context(request, adapter, record=None):
    config_api, _utils, _discover, _aiohttp = adapter
    try:
        full = config_api.Config.from_filename_or_environment(request.get("config"))
        pair = full.get_pair(request["pair"])
    except Exception:
        raise Refused("config") from None
    if pair.name_a == pair.name_b or request["local_storage"] not in (pair.name_a, pair.name_b):
        raise Refused()
    remote_name = pair.name_b if pair.name_a == request["local_storage"] else pair.name_a
    local_raw = full.storages[request["local_storage"]]
    remote_raw = full.storages[remote_name]
    if local_raw.get("type") != "filesystem" or remote_raw.get("type") != "caldav" or local_raw.get("read_only") or remote_raw.get("read_only"):
        raise Refused()
    # A changing URL.fetch output cannot establish the same account across separate
    # execute/discovery/reconcile processes. Keep that layout unavailable explicitly.
    if "url.fetch" in remote_raw or not isinstance(remote_raw.get("url"), str):
        raise Refused()
    if "path.fetch" in local_raw or not isinstance(local_raw.get("path"), str):
        raise Refused()
    local_path = Path(config_api.expand_path(local_raw["path"])).resolve(strict=True)
    if local_path != Path(request["vdir"]).resolve(strict=True):
        raise Refused()
    # Refresh writes the discovery cache while holding a shared vdir lock. Never
    # let configured cache paths (including symlink aliases) enter task storage.
    try:
        status_path = Path(config_api.expand_path(full.general["status_path"])).resolve()
    except Exception:
        raise Refused("config") from None
    if status_path == local_path or local_path in status_path.parents:
        raise Refused()
    collection = local_path / request["collection"]
    creating = request["action"] == "create" and request.get("mode", "execute") == "execute"
    allow_absent = request["action"] == "create" and request.get("mode") == "reconcile"
    if collection.is_symlink() or (creating and collection.exists()) or (not creating and not allow_absent and not collection.is_dir()):
        raise Refused()
    identity = remote_identity(pair, request)
    if record is not None:
        digest = hashlib.sha256(json.dumps({"storages": [local_raw, remote_raw], "sides": [pair.name_a, pair.name_b],
                                           "collections": pair.collections, "metadata": pair.metadata,
                                           "status_path": full.general["status_path"]}, sort_keys=True).encode()).hexdigest()
        if request.get("mode", "execute") == "execute":
            record["connection_digest"] = digest
            atomic_json(request["operation"], record)
        elif record.get("connection_digest") != digest:
            raise Refused("mapping")
    # Only after structural validation may vdirsyncer expand remote credential fetches.
    try:
        remote = dict(full.get_storage_args(remote_name))
    except Exception:
        raise Refused("credentials") from None
    return full, pair, remote, identity


@contextlib.contextmanager
def dav_policy(session_class, base):
    """Keep all adapter discovery/auth requests on the configured origin; never follow redirects."""
    origin = urlsplit(base)
    original = session_class.request

    async def bounded(session, method, path="", **kwargs):
        target = urlsplit(urljoin(session.url, path) if path else session.url)
        if (target.scheme, target.netloc) != (origin.scheme, origin.netloc):
            raise Refused("mapping")
        kwargs["allow_redirects"] = False
        response = await original(session, method, path, **kwargs)
        if 300 <= response.status < 400:
            raise Refused("dav")
        actual = urlsplit(str(response.url))
        if (actual.scheme, actual.netloc) != (origin.scheme, origin.netloc):
            raise Refused("dav")
        return response

    session_class.request = bounded
    try:
        yield
    finally:
        session_class.request = original


async def refresh_discovery(full, pair, adapter, connector):
    config_api, utils, discover, _aiohttp = adapter

    async def refuse_missing(*_args, **_kwargs):
        raise Refused("discovery")

    # The supported API otherwise prompts or implicitly creates missing unrelated lists.
    # In this isolated helper process, replace that policy at both entry points, retaining
    # the library's own cache writer and mapping expansion.
    originals = discover.handle_collection_not_found, utils.handle_collection_not_found
    discover.handle_collection_not_found = utils.handle_collection_not_found = refuse_missing
    try:
        status = config_api.expand_path(full.general["status_path"])
        utils.prepare_status_path(status)
        await discover.collections_for_pair(status, pair, from_cache=False, connector=connector)
    finally:
        discover.handle_collection_not_found, utils.handle_collection_not_found = originals
    return {"refreshed": True}


async def resolve(request, adapter):
    return await dispatch(request, adapter, None)


async def dispatch(request, adapter, record):
    _config_api, utils, _discover, aiohttp = adapter
    full, pair, remote, identity = pair_context(request, adapter, record)
    mode, action = request.get("mode", "execute"), request["action"]
    async with aiohttp.TCPConnector() as connector:
        cls, _ = utils.storage_class_from_config(remote)
        with dav_policy(cls.session_class, remote["url"]):
            if mode == "refresh":
                return await refresh_discovery(full, pair, adapter, connector)
            if mode == "reconcile":
                if record is None or not record.get("remote_target"):
                    raise Refused("operation")
                target = record["remote_target"]
                validate_target(remote["url"], target, identity)
                options = dict(remote, url=target, collection=identity)
            elif action == "create":
                if record is None:
                    raise Refused("operation")
                root_session, _ = cls.session_class.init_and_remaining_args(**dict(remote, connector=connector))
                discovery = cls.discovery_class(root_session, dict(remote, connector=connector))
                home = await discovery.find_home()
                target = urljoin(home.rstrip("/") + "/", quote(identity, safe="") + "/")
                validate_target(remote["url"], target, identity)
                options = dict(remote, url=target, collection=identity)
            else:
                args = await select_discovered(cls.discover(**dict(remote, connector=connector)), remote["url"], identity)
                options = dict(remote)
                options.update(args)
                target = options["url"]
            validate_target(remote["url"], options["url"], identity)
            storage = await utils.storage_instance_from_config(options, create=False, connector=connector)
            if record is not None and mode == "execute":
                record["remote_target"] = target
                atomic_json(request["operation"], record)
            changes = {key: request[key] for key in ("name", "color") if request.get(key) is not None}
            if mode == "reconcile":
                properties = await read_properties(storage.session)
                if not request.get("keep_server") and any(not same_metadata(k, properties[k], v) for k, v in changes.items()):
                    record["receipt"] = {"observed": properties}
                    atomic_json(request["operation"], record)
                    raise Refused("choice")
            elif action == "create":
                properties = await create_task_collection(storage.session, request["name"], request["color"])
            elif action == "edit":
                properties = await write_metadata(storage.session, changes, request["expected"])
            elif action == "preview-delete":
                properties = await read_properties(storage.session)
                captured = await capture_inventory(storage.session)
                directory = Path(request["state"]) / "lists/backups" / record["id"]
                for parent in (directory.parent, directory):
                    if parent.is_symlink() or (parent.exists() and not parent.is_dir()):
                        raise Refused("operation")
                    parent.mkdir(mode=0o700, exist_ok=True)
                    descriptor = os.open(parent.parent, os.O_RDONLY)
                    try:
                        os.fsync(descriptor)
                    finally:
                        os.close(descriptor)
                atomic_json(directory / "remote.json", captured)
                return {"backup": record["id"], "resources": len(captured["resources"]), **properties}
            else:
                properties = await read_properties(storage.session)
            return {"collection": request["collection"], "remote_collection": identity, **properties}


def main():
    code, result = "protocol", None
    active_record = None
    try:
        raw = sys.stdin.buffer.read(MAX_INPUT + 1)
        if len(raw) > MAX_INPUT:
            raise Refused("protocol")
        request = validate_request(json.loads(raw))
        def timed_out(_signal, _frame):
            # Do not rely on asyncio/subprocess cleanup finishing: a credential process
            # or DNS executor can stall it after the TUI and its monitor have gone.
            if active_record is not None:
                try:
                    active_record["phase"] = "uncertain"
                    atomic_json(request["operation"], active_record)
                except Exception:
                    pass  # remote-started is also a durable uncertain outcome.
            if os.getpgrp() == os.getpid():
                os.killpg(os.getpgrp(), signal.SIGKILL)
            os._exit(124)
        signal.signal(signal.SIGALRM, timed_out)
        signal.alarm(min(request.get("timeout_secs", 300), 365 * 24 * 3600))
        # Dependency/credential libraries must not print into the protocol stream.
        with contextlib.redirect_stdout(io.StringIO()):
            adapter = load_adapter()
            with operation_guard(request) as record:
                active_record = record
                mode = request.get("mode", "execute")
                if record is not None and mode == "execute":
                    record["phase"] = "remote-started"
                    atomic_json(request["operation"], record)
                try:
                    if request["action"] == "probe":
                        result = {"adapter": "vdirsyncer-0.21", "protocol": 1}
                    else:
                        result = asyncio.run(dispatch(request, adapter, record))
                    if record is not None:
                        if mode == "refresh":
                            record["phase"] = "local-confirmed"
                        else:
                            record.update(phase="remote-confirmed", receipt=result)
                        atomic_json(request["operation"], record)
                except BaseException as error:
                    if record is not None:
                        record["phase"] = "uncertain"
                        if request["action"] == "preview-delete":
                            record["receipt"] = {"no_remote_write": True}
                        elif mode == "execute" and isinstance(error, Refused) and error.code in ("stale", "exists"):
                            record["receipt"] = {"no_remote_write": True, "reason": error.code}
                        atomic_json(request["operation"], record)
                    raise
                finally:
                    active_record = None
        reply = {"version": 1, "ok": True, "result": result}
    except Refused as error:
        code = error.code
        reply = {"version": 1, "ok": False, "code": code}
    except Exception:
        # Never return an exception string: DAV/config errors can contain account URLs
        # and credential commands. The operation service retains private diagnostics.
        reply = {"version": 1, "ok": False, "code": "dav"}
    print(json.dumps(reply))
    return 0 if reply["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
