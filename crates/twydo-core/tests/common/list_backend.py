"""Offline adapter/network boundary for Rust tests of the bundled helper process.

Executes the actual embedded source passed to the configured Python command. Only the
external vdirsyncer objects and DAV transport are replaced; request validation, locks,
durable phases, XML writers/parsers and all Rust service behavior remain production code.
"""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import sys
import types
import time
from urllib.parse import urlsplit

REPO = Path(__file__).resolve().parents[4]
spec = importlib.util.spec_from_file_location("synthetic_dav", REPO / "tools/tests/test_list_helper.py")
memory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(memory)
STORE = None


def load():
    return json.loads(STORE.read_text()) if STORE.exists() else {"calls": [], "collections": {}, "flags": {}}


def save(data):
    STORE.write_text(json.dumps(data))


class Session(memory.MemoryDAV):
    def __init__(self, **options):
        self.url = options["url"].rstrip("/") + "/"
        self.identity = urlsplit(self.url).path.rstrip("/").rsplit("/", 1)[-1]
        store = load()
        super().__init__(exists=self.identity in store["collections"],
                         fail_status=store["flags"].get("fail_status", False),
                         ignore_write=store["flags"].get("ignore_write", False))
        if self.exists:
            self.state = store["collections"][self.identity]
        # `mutate_before_delete` makes the listing carry one resource the preview never saw.
        fault = store["flags"].get("inventory_fault") or ("extra" if store["flags"].get("mutate_before_delete") else None)
        self.inventory = memory.InventoryDAV(fault)
        self.inventory.url = self.url

    @classmethod
    def init_and_remaining_args(cls, **options):
        return cls(**options), {}

    async def request(self, method, path="", **kwargs):
        if method == "GET" or (method == "PROPFIND" and kwargs.get("headers", {}).get("Depth") == "1"):
            result = await self.inventory.request(method, path, **kwargs)
        elif method == "DELETE" and load()["flags"].get("fail_delete_once"):
            # A transport failure before the server applies the DELETE: the collection stays.
            store = load()
            store["flags"].pop("fail_delete_once")
            store["calls"].append(method)
            save(store)
            raise OSError("synthetic transport failure")
        elif method == "DELETE" and load()["flags"].get("partial_delete_once"):
            # The server removes one resource, then the transport fails: the collection stays.
            store = load()
            store["flags"].pop("partial_delete_once")
            store["flags"]["inventory_fault"] = "partial"
            store["calls"].append(method)
            save(store)
            raise OSError("synthetic transport failure")
        else:
            result = await super().request(method, path, **kwargs)
        store = load()
        store["calls"].append(method)
        if self.exists:
            store["collections"][self.identity] = self.state
        elif method == "DELETE":
            store["collections"].pop(self.identity, None)
        hang = ((method == "MKCALENDAR" and store["flags"].pop("hang_after_create_once", False))
                or (method == "DELETE" and store["flags"].pop("hang_after_delete_once", False)))
        save(store)
        if hang:
            time.sleep(store["flags"].get("hang_seconds", 10))
        return result


class Discovery:
    def __init__(self, session, options):
        self.options = options

    async def find_home(self):
        return "https://example.com/dav/"


class Storage:
    session_class = Session
    discovery_class = Discovery

    @classmethod
    async def discover(cls, **options):
        for identity in load()["collections"]:
            yield dict(options, collection=identity, url=f"https://example.com/dav/{identity}/")


class Connector:
    async def __aenter__(self):
        return self

    async def __aexit__(self, *_args):
        pass


class Config:
    @classmethod
    def from_filename_or_environment(cls, _path):
        # Tests read back which config path each helper run was given.
        _seen = Path(os.environ["TWYDO_TEST_LIST_CONNECTION"]).parent / "server.json"
        if _seen.exists():
            _data = json.loads(_seen.read_text())
            _data.setdefault("config_paths", []).append(_path)
            _seen.write_text(json.dumps(_data))
        # The generated vdirsyncer file is not parsed here: the harness names its own
        # connection file.
        global STORE
        path = Path(os.environ["TWYDO_TEST_LIST_CONNECTION"])
        settings = json.loads(path.read_text())
        STORE = path.parent / "server.json"
        self = cls()
        self.general = {"status_path": str(Path(path).parent / "cache")}
        self.storages = {"twydo_local": {"type": "filesystem", "path": settings["vdir"], "instance_name": "twydo_local"},
                         "twydo_remote": {"type": "caldav", "url": settings.get("remote_url", "https://example.com/dav/"), "instance_name": "twydo_remote"}}
        self.pair = types.SimpleNamespace(name="twydo", name_a="twydo_remote", name_b="twydo_local",
                                          metadata=["displayname", "color"], collections=["from a", "from b"])
        return self

    def get_pair(self, name):
        assert name == "twydo"
        return self.pair

    def get_storage_args(self, name):
        return dict(self.storages[name])


async def storage_instance_from_config(options, create=True, *, connector):
    assert create is False, "implicit collection creation is forbidden"
    return types.SimpleNamespace(session=Session(**options))


async def missing(*_args, **_kwargs):
    raise AssertionError("missing collection policy was not overridden")


async def create_missing(config, collection, e=None, implicit_create=False):
    """Stand-in for vdirsyncer's own handler: only an explicit local filesystem create."""
    assert implicit_create and config["type"] == "filesystem", "unexpected creation"
    Path(config["path"], collection).mkdir()
    # Like vdirsyncer 0.21, the handler's return value becomes the collection's storage args.
    return dict(config, path=str(Path(config["path"], collection)), collection=collection)


async def collections_for_pair(status, pair, from_cache=True, *, connector):
    assert from_cache is False
    store = load()
    store["calls"].append("DISCOVER")
    if store["flags"].pop("fail_refresh_once", False):
        save(store)
        raise RuntimeError("synthetic interrupted discovery")
    save(store)
    if store["flags"].get("discover_sleep"):
        time.sleep(store["flags"]["discover_sleep"])
    script = store.get("discover", {})
    config = Config.from_filename_or_environment(None)
    local, remote = config.get_storage_args("twydo_local"), config.get_storage_args("twydo_remote")
    for name in script.get("remote_only", []):
        if not Path(local["path"], name).exists():
            args = await discovery.handle_collection_not_found(local, name)
            assert isinstance(args, dict) and "type" in args, "handler must return storage args"
    if store["flags"].get("hang_after_adopt"):
        # vdirsyncer has created the folders and has not saved its cache yet.
        time.sleep(store["flags"]["hang_after_adopt"])
    for name in script.get("local_only", []):
        args = await discovery.handle_collection_not_found(remote, name)
        assert isinstance(args, dict) and "type" in args, "handler must return storage args"
    Path(status, "twydo.collections").write_text("verified synthetic discovery")
    return []


api = types.SimpleNamespace(Config=Config, expand_path=os.path.expanduser)
utils = types.SimpleNamespace(storage_class_from_config=lambda _options: (Storage, {}),
                              storage_instance_from_config=storage_instance_from_config,
                              handle_collection_not_found=create_missing,
                              prepare_status_path=lambda path: Path(path).mkdir(parents=True, exist_ok=True))
discovery = types.SimpleNamespace(collections_for_pair=collections_for_pair, handle_collection_not_found=missing)
aiohttp = types.SimpleNamespace(TCPConnector=Connector)
namespace = {"__name__": "bundled_helper"}
exec(sys.argv[-1], namespace)
namespace["load_adapter"] = lambda: (api, utils, discovery, aiohttp)
raise SystemExit(namespace["main"]())
