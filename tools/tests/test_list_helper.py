"""Offline collection-helper validation and DAV response tests; no account access."""
import asyncio
import importlib.util
from pathlib import Path
import types
import unittest
import tempfile

PATH = Path(__file__).resolve().parents[2] / "crates/twydo-core/src/lists/helper.py"


def helper():
    spec = importlib.util.spec_from_file_location("list_helper", PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ListHelperTests(unittest.TestCase):
    def setUp(self):
        self.h = helper()

    def request(self, **extra):
        return dict(version=1, action="resolve", pair="test", local_storage="local",
                    vdir="/tmp/synthetic-vdir", collection="synthetic", **extra)

    def test_invalid_protocol_and_targets_fail_before_adapter_import(self):
        for change in ({"version": 9}, {"version": True}, {"action": "DELETE"},
                       {"collection": "../other"}, {"collection": "a%2fb"},
                       {"collection": "."}, {"pair": ""}, {"mode": "refresh"}):
            request = self.request()
            request.update(change)
            with self.subTest(change=change), self.assertRaises(self.h.Refused):
                self.h.validate_request(request)

    def test_identity_mapping_handles_both_pair_orientations(self):
        request = self.request()
        for local_first in (True, False):
            options = dict(name_a="local" if local_first else "remote",
                           name_b="remote" if local_first else "local",
                           collections=[["logical", "synthetic", "server-id"]]
                           if local_first else [["logical", "server-id", "synthetic"]])
            pair = types.SimpleNamespace(**options)
            self.assertEqual(self.h.remote_identity(pair, request), "server-id")
            pair.collections.append(pair.collections[0])
            with self.assertRaises(self.h.Refused):
                self.h.remote_identity(pair, request)

    def test_mapped_dynamic_layout_and_multiple_bindings_are_refused(self):
        request=self.request()
        for collections in (
            [["alias","remote-one","synthetic"],"from b"],
            [["alias","remote-one","synthetic"],["alias","synthetic","other"]],
            [["one","remote-one","synthetic"],["two","remote-two","synthetic"]],
            [["one","remote-one","synthetic"],["two","remote-one","other"]],
        ):
            pair=types.SimpleNamespace(name_a="remote",name_b="local",collections=collections)
            with self.subTest(collections=collections),self.assertRaises(self.h.Refused):
                self.h.remote_identity(pair,request)

    def test_unmapped_or_unsafe_remote_identity_is_refused(self):
        pair = types.SimpleNamespace(name_a="remote", name_b="local",
                                     collections=[["logical", "../root", "synthetic"]])
        with self.assertRaises(self.h.Refused):
            self.h.remote_identity(pair, self.request())
        pair.collections = ["different"]
        with self.assertRaises(self.h.Refused):
            self.h.remote_identity(pair, self.request())
        pair.collections = ["from a", "from b"]
        self.assertEqual(self.h.remote_identity(pair, self.request()), "synthetic")

    def test_discovery_cannot_choose_between_duplicate_remote_identities(self):
        async def discovered():
            yield dict(collection="synthetic", url="https://example.com/dav/synthetic/")
            yield dict(collection="synthetic", url="https://example.com/other/synthetic/")

        with self.assertRaises(self.h.Refused):
            asyncio.run(self.h.select_discovered(discovered(), "https://example.com/dav/", "synthetic"))

    def test_unsafe_pair_never_fetches_credentials(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "synthetic").mkdir()
            marker = root / "credentials-fetched"
            request = self.request()
            request["vdir"] = str(root)
            pair = types.SimpleNamespace(name_a="remote", name_b="local", collections=["from a", "from b"])

            def fetch(_name):
                marker.write_text("fetched")
                raise RuntimeError("credential boundary reached")

            full = types.SimpleNamespace(
                get_pair=lambda _name: pair, get_storage_args=fetch,
                storages={"local": {"type": "filesystem", "path": str(root)},
                          "remote": {"type": "caldav", "url": "https://example.com/dav/account/"}})
            api = types.SimpleNamespace(
                Config=types.SimpleNamespace(from_filename_or_environment=lambda _path: full),
                expand_path=lambda path: path)
            for field, value in (("read_only", True), ("type", "http")):
                full.storages["remote"][field] = value
                with self.assertRaises(self.h.Refused):
                    asyncio.run(self.h.resolve(request, (api, None, None, None)))
                self.assertFalse(marker.exists())
                full.storages["remote"] = {"type": "caldav", "url": "https://example.com/dav/account/"}
            full.storages["remote"] = {"type": "caldav", "url.fetch": ["command", "synthetic-url"]}
            with self.assertRaises(self.h.Refused):
                asyncio.run(self.h.resolve(request, (api, None, None, None)))
            self.assertFalse(marker.exists())
            full.storages["remote"] = {"type": "caldav", "url": "https://example.com/dav/account/"}
            full.storages["local"]["path"] = str(root.parent)
            with self.assertRaises(self.h.Refused):
                asyncio.run(self.h.resolve(request, (api, None, None, None)))
            self.assertFalse(marker.exists())

    def test_discovery_cache_inside_vdir_or_symlink_alias_is_refused_before_credentials(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);vdir=root/"vdir";(vdir/"synthetic").mkdir(parents=True)
            (root/"alias").symlink_to(vdir,target_is_directory=True)
            calls=[]
            pair=types.SimpleNamespace(name_a="remote",name_b="local",collections=["from a","from b"])
            full=types.SimpleNamespace(get_pair=lambda _:pair,
                storages={"local":{"type":"filesystem","path":str(vdir)},"remote":{"type":"caldav","url":"https://example.com/dav/"}},general={})
            def fetch(_):calls.append(True);return full.storages["remote"]
            full.get_storage_args=fetch
            api=types.SimpleNamespace(Config=types.SimpleNamespace(from_filename_or_environment=lambda _:full),expand_path=lambda p:p)
            request=self.request();request["vdir"]=str(vdir)
            for cache in (vdir,vdir/"synthetic/cache",root/"alias/synthetic/cache"):
                full.general["status_path"]=str(cache)
                with self.subTest(cache=cache),self.assertRaises(self.h.Refused):
                    self.h.pair_context(request,(api,None,None,None))
                self.assertEqual(calls,[])
            full.general["status_path"]=str(root/"safe-cache")
            self.h.pair_context(request,(api,None,None,None))
            self.assertEqual(len(calls),1)

    def test_discover_request_needs_no_collection_or_operation(self):
        request = dict(version=1, action="discover", pair="test", local_storage="local", vdir="/tmp/x")
        self.assertIs(self.h.validate_request(request), request)
        with self.assertRaises(self.h.Refused) as caught:
            self.h.validate_request(dict(request, mode="refresh"))
        self.assertEqual(caught.exception.code, "protocol")

    def test_discover_policy_creates_only_local_filesystem(self):
        calls, adopted = [], []

        async def original(*args, **kwargs):
            calls.append((args, kwargs))
            return {"type": "filesystem", "collection": args[1]}

        policy = self.h.local_only_policy("local", original, adopted)
        local = {"type": "filesystem", "instance_name": "local"}
        remote = {"type": "caldav", "instance_name": "remote"}
        result = asyncio.run(policy(local, "phone-list"))
        self.assertEqual(result, {"type": "filesystem", "collection": "phone-list"})
        self.assertEqual(len(calls), 1)
        self.assertIs(calls[0][1]["implicit_create"], True)
        self.assertEqual(adopted, ["phone-list"])
        with self.assertRaises(self.h.Refused) as caught:
            asyncio.run(policy(remote, "stale"))
        self.assertEqual((caught.exception.code, caught.exception.missing), ("discovery", ["stale"]))
        with self.assertRaises(self.h.Refused) as caught:
            asyncio.run(policy(local, "../x"))
        self.assertEqual(caught.exception.code, "mapping")
        self.assertEqual(len(calls), 1)
        self.assertEqual(adopted, ["phone-list"])

    def test_discover_rejects_operation_but_allows_state(self):
        base = {"version": 1, "action": "discover", "pair": "p", "local_storage": "l", "vdir": "/x"}
        self.h.validate_request(dict(base))
        self.h.validate_request(dict(base, state="/s"))
        with self.assertRaises(self.h.Refused) as caught:
            self.h.validate_request(dict(base, operation="/o"))
        self.assertEqual(caught.exception.code, "protocol")

    def test_discover_guard_holds_the_shared_vdir_lock_only(self):
        import fcntl
        import os
        with tempfile.TemporaryDirectory() as raw:
            state = Path(raw)
            request = {"action": "discover", "state": str(state)}
            with self.h.discover_guard(request):
                fd = os.open(state / "vdir.lock", os.O_RDWR)
                try:
                    fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
                    fcntl.flock(fd, fcntl.LOCK_UN)
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                finally:
                    os.close(fd)
                self.assertFalse((state / "sync/run.lock").exists())
            fd = os.open(state / "vdir.lock", os.O_RDWR)
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            finally:
                os.close(fd)

    def test_discover_guard_takes_no_lock_of_its_own_when_the_parent_handed_one_down(self):
        with tempfile.TemporaryDirectory() as raw:
            request = {"action": "discover", "state": raw}
            with self.h.discover_guard(request, inherited_lock=True):
                self.assertFalse((Path(raw) / "vdir.lock").exists())

    def test_discover_guard_without_state_or_for_other_actions_is_a_noop(self):
        with self.h.discover_guard({"action": "discover"}):
            pass
        with self.h.discover_guard({"action": "probe", "state": "/nonexistent-state"}):
            pass

    def test_discover_policy_classifies_local_side_mapping_failures(self):
        async def original(*_args, **_kwargs):
            raise AssertionError("must not create")

        policy = self.h.local_only_policy("local", original, [])
        # The configured local storage, but not a filesystem one: a configuration problem.
        with self.assertRaises(self.h.Refused) as caught:
            asyncio.run(policy({"type": "caldav", "instance_name": "local"}, "x"))
        self.assertEqual((caught.exception.code, caught.exception.missing), ("mapping", None))
        # Some other storage that is not the CalDAV remote: also not "missing on the server".
        with self.assertRaises(self.h.Refused) as caught:
            asyncio.run(policy({"type": "filesystem", "instance_name": "other"}, "x"))
        self.assertEqual(caught.exception.code, "mapping")
        with self.assertRaises(self.h.Refused) as caught:
            asyncio.run(policy({"type": "caldav", "instance_name": "remote"}, "x"))
        self.assertEqual((caught.exception.code, caught.exception.missing), ("discovery", ["x"]))

    def test_operation_guard_refuses_stale_collection_before_remote_work(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            vdir, state = root / "vdir", root / "state"
            (vdir / "synthetic").mkdir(parents=True)
            (state / "lists/operations").mkdir(parents=True)
            (state / "sync").mkdir()
            (state / "vdir").write_text(str(vdir.resolve()) + "\n")
            (state / "lists/active").write_text("operation1")
            request = self.request()
            request.update(vdir=str(vdir), state=str(state), operation=str(state / "lists/operations/operation1.json"))
            import json
            record = dict(format=1, id="operation1", phase="prepared", vdir=str(vdir), request=request, holds={})
            Path(request["operation"]).write_text(json.dumps(record))
            (vdir / "synthetic/new.ics").write_text("synthetic concurrent content")
            with self.assertRaises(self.h.Refused):
                with self.h.operation_guard(request):
                    self.fail("stale operation entered its remote phase")

    def test_metadata_edit_writes_only_requested_property_and_verifies_it(self):
        transport = MemoryDAV()
        result = asyncio.run(self.h.write_metadata(transport, {"name": "After"}, {"name": "Before", "color": "#3289D2"}))
        self.assertEqual(result, {"name": "After", "color": "#3289D2"})
        self.assertEqual(transport.state["unknown"], "preserved")
        self.assertEqual(transport.written_properties, ["{DAV:}displayname"])
        self.assertTrue(all(call[2]["allow_redirects"] is False for call in transport.calls))

    def test_stale_metadata_is_refused_before_any_write(self):
        transport = MemoryDAV()
        with self.assertRaises(self.h.Refused):
            asyncio.run(self.h.write_metadata(transport, {"name": "After"}, {"name": "Stale"}))
        self.assertEqual(transport.written_properties, [])
        self.assertEqual(transport.state["name"], "Before")

    def test_property_failure_and_false_success_do_not_report_a_saved_edit(self):
        for fail_status, ignore_write in ((True, False), (False, True)):
            transport = MemoryDAV(fail_status=fail_status, ignore_write=ignore_write)
            with self.assertRaises(self.h.Refused):
                asyncio.run(self.h.write_metadata(transport, {"name": "After"}, {"name": "Before"}))

    def test_unreadable_or_omitted_color_cannot_become_an_absence_receipt(self):
        for status in ("403 Forbidden","missing"):
            transport=MetadataStatusDAV(status)
            with self.subTest(status=status),self.assertRaises(self.h.Refused):
                asyncio.run(self.h.write_metadata(transport,{"name":"After"},{"name":"Before"}))
            self.assertFalse(transport.written_properties)
        result=asyncio.run(self.h.read_properties(MetadataStatusDAV("404 Not Found")))
        self.assertIsNone(result["color"])

    def test_create_requires_absence_and_requests_a_vtodo_collection(self):
        transport = MemoryDAV(exists=False)
        result = asyncio.run(self.h.create_task_collection(transport, "New <list>", "#379417"))
        self.assertEqual(result, {"name": "New <list>", "color": "#379417"})
        creates = [call for call in transport.calls if call[0] == "MKCALENDAR"]
        self.assertEqual(len(creates), 1)
        self.assertEqual(creates[0][2]["headers"]["If-None-Match"], "*")
        import xml.etree.ElementTree as ET
        body = ET.fromstring(creates[0][2]["data"])
        component = body.find(".//{urn:ietf:params:xml:ns:caldav}comp")
        self.assertEqual(component.get("name"), "VTODO")

    def test_create_never_reuses_an_existing_identity(self):
        transport = MemoryDAV()
        with self.assertRaises(self.h.Refused):
            asyncio.run(self.h.create_task_collection(transport, "New", "#379417"))
        self.assertFalse(any(call[0] == "MKCALENDAR" for call in transport.calls))

    def test_discovered_target_cannot_escape_origin_or_collection(self):
        base = "https://example.com/dav/account/"
        self.h.validate_target(base, "https://example.com/calendars/account/synthetic/", "synthetic")
        for target in ("https://evil.example.org/synthetic/", base,
                       "https://example.com/dav/account/other/",
                       "https://example.com/dav/account/a%2fsynthetic/",
                       "https://example.com/dav/account/synthetic/?query=x"):
            with self.subTest(target=target), self.assertRaises(self.h.Refused):
                self.h.validate_target(base, target, "synthetic")

    def test_read_properties_requires_successful_vtodo_collection(self):
        def response(status="200 OK", component="VTODO", duplicate=False):
            row = f'''<d:response><d:href>/dav/synthetic/</d:href><d:propstat>
              <d:prop><d:displayname>Synthetic</d:displayname><d:resourcetype><c:calendar/></d:resourcetype>
              <c:supported-calendar-component-set><c:comp name="{component}"/></c:supported-calendar-component-set>
              <a:calendar-color>#3289D2</a:calendar-color></d:prop><d:status>HTTP/1.1 {status}</d:status>
              </d:propstat></d:response>'''
            return ('<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" '
                    'xmlns:a="http://apple.com/ns/ical/">' + row * (2 if duplicate else 1) + '</d:multistatus>').encode()

        class Session:
            url = "https://example.com/dav/synthetic/"

            def __init__(self, data):
                self.data = data
                self.calls = []

            async def request(self, method, path, **kwargs):
                self.calls.append((method, path, kwargs))

                async def read(n=-1):
                    result, self.data = self.data[:n], self.data[n:]
                    return result
                return types.SimpleNamespace(status=207, url=self.url,
                                             content=types.SimpleNamespace(read=read))

        transport = Session(response())
        result = asyncio.run(self.h.read_properties(transport))
        self.assertEqual(result["name"], "Synthetic")
        self.assertEqual(result["color"], "#3289D2")
        self.assertFalse(transport.calls[0][2]["allow_redirects"])
        for data in (response(status="403 Forbidden"), response(component="VEVENT"), response(duplicate=True)):
            with self.assertRaises(self.h.Refused):
                asyncio.run(self.h.read_properties(Session(data)))

    def test_delete_inventory_keeps_remote_only_and_malformed_resources_exactly(self):
        transport = InventoryDAV()
        result = asyncio.run(self.h.capture_inventory(transport))
        import base64
        self.assertEqual([base64.b64decode(r["bytes"]) for r in result["resources"]],
                         [b"malformed synthetic VTODO\x00", b"opaque synthetic resource"])
        self.assertEqual(len(result["resources"]), 2)
        self.assertTrue(all(c[0] in ("PROPFIND", "GET") for c in transport.calls))
        self.assertIn("properties", result)

    def test_delete_inventory_refuses_incomplete_stale_or_unsafe_results(self):
        for fault in ("weak", "missing", "duplicate", "foreign", "nested", "failed", "changed", "get-stale", "properties"):
            with self.subTest(fault=fault), self.assertRaises(self.h.Refused):
                asyncio.run(self.h.capture_inventory(InventoryDAV(fault)))

    def test_collection_etag_absence_does_not_weaken_resource_backup_validators(self):
        captured=asyncio.run(self.h.capture_inventory(InventoryDAV("collection-etag-absent")))
        self.assertEqual(len(captured["resources"]),2)
        self.assertEqual(captured["resources"][0]["etag"],'"v0"')

    def delete_request(self, **extra):
        request = self.request()
        request.update(action="delete", operation="/state/lists/operations/op.json",
                       state="/state", backup="backup1")
        request.update(extra)
        return request

    def test_delete_request_requires_backup_operation_and_state(self):
        for mode in ("execute", "reconcile"):
            self.h.validate_request(self.delete_request(mode=mode))
        for change in ({"backup": None}, {"backup": ""}, {"backup": "../x"}, {"backup": "/abs"}, {"backup": 7},
                       {"operation": None}, {"state": None}, {"mode": "refresh"}, {"collection": "../x"}):
            with self.subTest(change=change), self.assertRaises(self.h.Refused) as caught:
                request = self.delete_request()
                request.update(change)
                self.h.validate_request(request)
            self.assertEqual(caught.exception.code, "protocol")
        request = self.delete_request()
        del request["backup"]
        with self.assertRaises(self.h.Refused):
            self.h.validate_request(request)

    def captured(self):
        return asyncio.run(self.h.capture_inventory(InventoryDAV()))

    def test_delete_revalidates_then_sends_one_unconditional_delete(self):
        backup, transport = self.captured(), InventoryDAV()
        asyncio.run(self.h.delete_collection(transport, backup))
        deletes = [c for c in transport.calls if c[0] == "DELETE"]
        self.assertEqual(len(deletes), 1)
        self.assertEqual(deletes[0][1], "")
        self.assertEqual(transport.url, "https://example.com/dav/synthetic/")
        self.assertFalse({k.lower() for k in deletes[0][2].get("headers", {})} & {"if-match", "if-none-match"})
        self.assertEqual(transport.calls[-1][0], "DELETE")
        self.assertTrue(any(c[0] == "GET" for c in transport.calls[:-1]))

    def test_delete_treats_404_as_deleted_and_other_statuses_as_dav_failures(self):
        backup = self.captured()
        asyncio.run(self.h.delete_collection(InventoryDAV(delete_status=404), backup))
        for status in (403, 405, 409, 423, 500):
            with self.subTest(status=status), self.assertRaises(self.h.Refused) as caught:
                asyncio.run(self.h.delete_collection(InventoryDAV(delete_status=status), backup))
            self.assertEqual(caught.exception.code, "dav")

    def test_delete_refuses_changed_inventory_or_properties(self):
        backup = self.captured()
        for fault in ("extra", "etag-changed", "bytes-changed", "props-changed"):
            transport = InventoryDAV(fault)
            with self.subTest(fault=fault), self.assertRaises(self.h.Refused) as caught:
                asyncio.run(self.h.delete_collection(transport, backup))
            self.assertEqual(caught.exception.code, "changed")
            self.assertFalse(any(c[0] == "DELETE" for c in transport.calls))
        missing = dict(backup, resources=backup["resources"][:1] + [dict(backup["resources"][1], href=backup["resources"][1]["href"] + "x")])
        with self.assertRaises(self.h.Refused) as caught:
            asyncio.run(self.h.delete_collection(InventoryDAV(), missing))
        self.assertEqual(caught.exception.code, "changed")

    def test_delete_refuses_incomplete_inventory_before_delete(self):
        backup = self.captured()
        for fault in ("weak", "missing", "duplicate", "foreign", "nested", "failed", "changed", "get-stale", "properties"):
            transport = InventoryDAV(fault)
            with self.subTest(fault=fault), self.assertRaises(self.h.Refused):
                asyncio.run(self.h.delete_collection(transport, backup))
            self.assertFalse(any(c[0] == "DELETE" for c in transport.calls))

    def test_delete_transport_failure_surfaces_for_an_uncertain_record(self):
        with self.assertRaises(OSError):
            asyncio.run(self.h.delete_collection(InventoryDAV("delete-transport"), self.captured()))

    def test_delete_reconcile_distinguishes_absent_present_changed_and_unreachable(self):
        backup = self.captured()
        transport = InventoryDAV(absent=True)
        asyncio.run(self.h.reconcile_delete(transport, backup))
        for fault, code in ((None, "present"), ("extra", "changed"), ("props-changed", "changed"), ("bytes-changed", "changed")):
            with self.subTest(fault=fault), self.assertRaises(self.h.Refused) as caught:
                asyncio.run(self.h.reconcile_delete(InventoryDAV(fault), backup))
            self.assertEqual(caught.exception.code, code)
        class Down(InventoryDAV):
            async def request(self, *args, **kwargs):
                raise OSError("synthetic network failure")
        with self.assertRaises(OSError):
            asyncio.run(self.h.reconcile_delete(Down(), backup))
        for transport in (InventoryDAV(), InventoryDAV("extra"), InventoryDAV(absent=True)):
            try:
                asyncio.run(self.h.reconcile_delete(transport, backup))
            except self.h.Refused:
                pass
            self.assertFalse(any(c[0] == "DELETE" for c in transport.calls))

    def test_delete_backup_must_be_a_plain_file_under_unlinked_directories(self):
        import json
        import os
        with tempfile.TemporaryDirectory() as temp:
            state = Path(temp) / "state"
            directory = state / "lists/backups/backup1"
            directory.mkdir(parents=True)
            request = self.delete_request(state=str(state))
            with self.assertRaises(self.h.Refused) as caught:
                self.h.read_backup(request)
            self.assertEqual(caught.exception.code, "operation")
            (directory / "remote.json").write_text(json.dumps(self.captured()))
            self.assertEqual(self.h.read_backup(request)["format"], 1)
            real = Path(temp) / "real.json"
            (directory / "remote.json").rename(real)
            (directory / "remote.json").symlink_to(real)
            with self.assertRaises(self.h.Refused):
                self.h.read_backup(request)
            (directory / "remote.json").unlink()
            os.mkfifo(directory / "remote.json")
            with self.assertRaises(self.h.Refused):
                self.h.read_backup(request)
            (directory / "remote.json").unlink()
            (directory / "remote.json").write_text("{not json")
            with self.assertRaises(self.h.Refused):
                self.h.read_backup(request)
            (directory / "remote.json").unlink()
            directory.rmdir()
            directory.symlink_to(Path(temp), target_is_directory=True)
            with self.assertRaises(self.h.Refused):
                self.h.read_backup(request)


class InventoryDAV:
    url = "https://example.com/dav/synthetic/"

    def __init__(self, fault=None, absent=False, delete_status=204):
        self.fault, self.calls, self.inventories = fault, [], 0
        self.absent, self.delete_status = absent, delete_status

    def tag(self, index):
        return '"v%d-new"' % index if self.fault == "etag-changed" else f'"v{index}"'

    async def request(self, method, path="", **kwargs):
        import xml.etree.ElementTree as ET
        self.calls.append((method, path, kwargs))
        status, headers = 207, {}
        if method == "DELETE":
            status, raw = self.delete_status, b""
            if self.fault == "delete-transport":
                raise OSError("synthetic transport failure")
            if 200 <= status < 300 or status == 404:
                self.absent = True
        elif method == "GET":
            index = 0 if path.endswith("remote.ics") else 2 if path.endswith("added.ics") else 1
            raw = [b"malformed synthetic VTODO\x00", b"opaque synthetic resource", b"added synthetic resource"][index]
            if self.fault == "bytes-changed" and index == 0:
                raw += b"!"
            headers["ETag"] = '"changed"' if self.fault == "get-stale" else self.tag(index)
            status = 200
        elif kwargs["headers"]["Depth"] == "0":
            name = b"Renamed" if self.fault == "props-changed" else b"Synthetic"
            raw = (b'''<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/"><d:response><d:href>/dav/synthetic/</d:href><d:propstat><d:prop><d:displayname>''' + name
                   + b'''</d:displayname><d:resourcetype><c:calendar/></d:resourcetype><c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set><a:calendar-color>#3289D2</a:calendar-color></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>''')
            if self.fault == "properties":
                raw = raw.replace(b"200 OK", b"403 Forbidden")
            if self.absent:
                status, raw = 404, b""
        else:
            self.inventories += 1
            root = ET.Element("{DAV:}multistatus")
            names = ("", "remote.ics", "unknown.bin") + (("added.ics",) if self.fault == "extra" else ())
            for index, name in enumerate(names):
                row = ET.SubElement(root, "{DAV:}response")
                href = self.url + name
                if name and self.fault == "foreign": href = "https://elsewhere.example.org/" + name
                if name and self.fault == "nested": href = self.url + "nested/" + name
                ET.SubElement(row, "{DAV:}href").text = href
                ps = ET.SubElement(row, "{DAV:}propstat")
                ET.SubElement(ps, "{DAV:}status").text = "HTTP/1.1 " + ("403 Forbidden" if self.fault == "failed" else "200 OK")
                prop = ET.SubElement(ps, "{DAV:}prop")
                kind = ET.SubElement(prop, "{DAV:}resourcetype")
                if not name:
                    ET.SubElement(kind, "{DAV:}collection")
                    if self.fault=="collection-etag-absent":
                        absent=ET.SubElement(row,"{DAV:}propstat")
                        ET.SubElement(ET.SubElement(absent,"{DAV:}prop"),"{DAV:}getetag")
                        ET.SubElement(absent,"{DAV:}status").text="HTTP/1.1 404 Not Found"
                elif self.fault != "missing":
                    tag = self.tag(index - 1)
                    if self.fault == "weak": tag = "W/" + tag
                    if self.fault == "changed" and self.inventories > 1: tag = '"changed"'
                    ET.SubElement(prop, "{DAV:}getetag").text = tag
                if self.fault == "duplicate" and name: root.append(row)
            raw = ET.tostring(root)
        class Content:
            async def read(self, n):
                nonlocal raw
                result,raw=raw[:n],raw[n:]
                return result
        return types.SimpleNamespace(status=status,url=path or self.url,headers=headers,content=Content())


class MemoryDAV:
    """The network boundary only: all validation/writing is production helper code."""
    url = "https://example.com/dav/synthetic/"

    def __init__(self, fail_status=False, ignore_write=False, exists=True, fail_delete=False):
        self.fail_delete = fail_delete
        self.state = dict(name="Before", color="#3289D2", unknown="preserved")
        self.fail_status, self.ignore_write = fail_status, ignore_write
        self.calls, self.written_properties = [], []
        self.exists = exists

    async def request(self, method, path, **kwargs):
        import xml.etree.ElementTree as ET
        from xml.sax.saxutils import escape
        from urllib.parse import urlsplit
        self.calls.append((method, path, kwargs))
        href = urlsplit(self.url).path
        status_code = 207
        if method == "PROPFIND":
            raw = f'''<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/">
              <d:response><d:href>{href}</d:href><d:propstat><d:prop>
              <d:displayname>{escape(self.state['name'])}</d:displayname><a:calendar-color>{escape(self.state['color'])}</a:calendar-color>
              <d:resourcetype><c:calendar/></d:resourcetype><c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set>
              </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>'''.encode()
            if not self.exists:
                status_code, raw = 404, b""
        elif method == "PROPPATCH":
            root = ET.fromstring(kwargs["data"])
            values = root.findall("{DAV:}set/{DAV:}prop/*")
            for prop in values:
                self.written_properties.append(prop.tag)
                field = {"{DAV:}displayname": "name", "{http://apple.com/ns/ical/}calendar-color": "color"}[prop.tag]
                if not self.ignore_write:
                    self.state[field] = prop.text
            status = "403 Forbidden" if self.fail_status else "200 OK"
            properties = b"".join(ET.tostring(p) for p in values).decode()
            raw = f'<d:multistatus xmlns:d="DAV:"><d:response><d:href>{href}</d:href><d:propstat><d:prop>{properties}</d:prop><d:status>HTTP/1.1 {status}</d:status></d:propstat></d:response></d:multistatus>'.encode()
        elif method == "MKCALENDAR":
            assert not self.exists
            self.exists = True
            root = ET.fromstring(kwargs["data"])
            self.state["name"] = root.findtext(".//{DAV:}displayname")
            self.state["color"] = root.findtext(".//{http://apple.com/ns/ical/}calendar-color")
            status_code, raw = 201, b""
        elif method == "DELETE":
            if self.fail_delete:
                raise OSError("synthetic transport failure")
            self.exists = False
            status_code, raw = 204, b""
        else:
            raise AssertionError("unexpected request")

        class Content:
            async def read(self, n):
                nonlocal raw
                result, raw = raw[:n], raw[n:]
                return result
        return types.SimpleNamespace(status=status_code, url=self.url, content=Content())


class MetadataStatusDAV(MemoryDAV):
    def __init__(self,status):
        super().__init__()
        self.property_status=status

    async def request(self,method,path,**kwargs):
        response=await super().request(method,path,**kwargs)
        if method!="PROPFIND":return response
        import xml.etree.ElementTree as ET
        root=ET.fromstring(await response.content.read(4*1024*1024))
        row=root.find("{DAV:}response")
        prop=row.find("{DAV:}propstat/{DAV:}prop")
        color=prop.find("{http://apple.com/ns/ical/}calendar-color")
        prop.remove(color)
        if self.property_status!="missing":
            ps=ET.SubElement(row,"{DAV:}propstat")
            ET.SubElement(ps,"{DAV:}prop").append(color)
            ET.SubElement(ps,"{DAV:}status").text="HTTP/1.1 "+self.property_status
        raw=ET.tostring(root)
        async def read(n):
            nonlocal raw
            result,raw=raw[:n],raw[n:]
            return result
        response.content=types.SimpleNamespace(read=read)
        return response


if __name__ == "__main__":
    unittest.main()
