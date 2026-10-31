import importlib.util
import io
import json
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "reconcile.py"
spec = importlib.util.spec_from_file_location("reconcile", SCRIPT)
rc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rc)


def src_task(uid, title="Synthetic", caluid="L1", **kw):
    t = {"uid": uid, "caluid": caluid, "title": title, "parent": "", "iscomp": "0",
         "deleted": "0", "due": "", "duetime": "999999", "repeattype": "0",
         "repeatval": "0", "recurrence": "1", "recurrenceendtype": "0", "alarms": []}
    t.update(kw)
    return t


def dst_task(uid, title="Synthetic", collection="col-a", **kw):
    t = {"uid": uid, "collection": collection, "summary": title, "kind": "task",
         "status": "open", "due": None, "due_is_date": False, "start": None,
         "start_is_date": False, "completed": None, "rrule": None,
         "from_completion": False, "parent": None, "alarms": []}
    t.update(kw)
    return t


def inventory(tasks, lists=None):
    lists = lists if lists is not None else [{"uid": "L1", "title": "Work", "isarchived": "0"}]
    return {"tasks": tasks, "lists": lists, "list_groups": []}


def export(tasks, cols=None):
    cols = cols if cols is not None else [{"id": "col-a", "display_name": "Work", "color": None}]
    return {"schema": 1, "collections": cols, "tasks": tasks}


def run(inv, exp, extra=()):
    with tempfile.TemporaryDirectory() as d:
        a, b = Path(d) / "inv.json", Path(d) / "exp.json"
        a.write_text(json.dumps(inv))
        b.write_text(json.dumps(exp))
        out = io.StringIO()
        with redirect_stdout(out), redirect_stderr(io.StringIO()):
            code = rc.main([str(a), str(b), *extra])
    return code, out.getvalue()


class RuleTests(unittest.TestCase):
    def test_repeattype_to_rrule(self):
        f = rc.repeat_to_rrule
        self.assertIsNone(f(0, 0, 0, None, None))
        self.assertEqual(f(258, 1, 0, None, None), "FREQ=MONTHLY")
        self.assertEqual(f(258, 3, 0, None, None), "FREQ=MONTHLY;INTERVAL=3")
        self.assertEqual(f(124, 0, 0, None, None), "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR")
        self.assertEqual(f(261, 2, 0, None, None), "FREQ=MONTHLY;BYDAY=2TU")
        self.assertEqual(f(267, 1, 0, None, None), "FREQ=MONTHLY;BYMONTHDAY=-1")
        self.assertEqual(f(268, 5, 0, None, None),
                         "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1")
        self.assertEqual(f(269, 1, 0, None, None), "FREQ=MONTHLY;BYDAY=SA,SU;BYSETPOS=1")
        self.assertEqual(f(258, 1, 2, 3, None), "FREQ=MONTHLY;COUNT=3")
        self.assertEqual(f(259, 1, 1, None, "2030-05-06"), "FREQ=YEARLY;UNTIL=20300506")

    def test_canonical_rrule(self):
        c = rc.canonical_rrule
        self.assertEqual(c("freq=monthly;interval=1"), c("FREQ=MONTHLY"))
        self.assertEqual(c("BYDAY=MO;FREQ=WEEKLY"), c("FREQ=WEEKLY;BYDAY=MO"))
        self.assertEqual(c("FREQ=DAILY"), c(rc.repeat_to_rrule(127, 0, 0, None, None)))
        self.assertEqual(c("FREQ=DAILY;UNTIL=20300506T000000Z"), c("FREQ=DAILY;UNTIL=20300506"))

    def test_duetime_sentinels(self):
        for v in ("999999", "-1", "0", "", None):
            self.assertIsNone(rc.source_due_time(v), v)
        self.assertEqual(rc.source_due_time("930"), "09:30")
        self.assertEqual(rc.source_due_time("1405"), "14:05")


class ReportTests(unittest.TestCase):
    def test_clean_match_exit_0(self):
        inv = inventory([
            src_task("u1", due="2026-11-04", duetime="930", repeattype="258", repeatval="1",
                     alarms=[{"deleted": "0"}]),
            src_task("u2", iscomp="1"),
            src_task("u3", parent="u1"),
            src_task("gone", deleted="1"),
        ])
        exp = export([
            dst_task("u1", due="2026-11-04T09:30:00-05:00", rrule="FREQ=MONTHLY",
                     alarms=[{"relative_seconds": -60}]),
            dst_task("u2", status="completed"),
            dst_task("u3", parent="u1"),
        ])
        code, out = run(inv, exp)
        self.assertEqual(code, 0, out)
        self.assertIn("matched-by-uid: 3", out)
        self.assertIn("deleted-skipped: 1", out)
        self.assertIn("mismatches: 0", out)

    def test_fallback_match(self):
        inv = inventory([src_task("u1", title="  Same  Title ", due="2026-11-04")])
        exp = export([dst_task("z9", title="same title", due="2026-11-04")])
        code, out = run(inv, exp)
        self.assertEqual(code, 0, out)
        self.assertIn("matched-by-fallback: 1", out)

    def test_mismatches_reported_by_uid(self):
        inv = inventory([
            src_task("u1", title="Secret title", due="2026-11-04"),
            src_task("u2", title="Child secret", parent="u1"),
        ])
        exp = export([dst_task("u1", title="Secret title", due="2026-11-05")])
        code, out = run(inv, exp)
        self.assertEqual(code, 1)
        self.assertIn("u1: due: source=2026-11-04 dest=2026-11-05", out)
        self.assertIn("u2:", out)
        self.assertNotIn("secret", out.lower())
        code, out = run(inv, exp, ["--show-titles"])
        self.assertIn("Child secret", out)

    def test_special_lists_explained(self):
        lists = [{"uid": "L1", "title": "Work", "isarchived": "0"},
                 {"uid": "S1", "title": "Starred", "starred": "1"},
                 {"uid": "S2", "title": "Upcoming", "issmartcalendar": "1"}]
        code, out = run(inventory([src_task("u1")], lists), export([dst_task("u1")]))
        self.assertEqual(code, 0, out)
        section = out.split("not a collection")[1]
        self.assertIn("Starred", section)
        self.assertIn("Upcoming", section)
        self.assertNotIn("missing", out)

    def test_archived_via_config(self):
        inv = inventory([src_task("u1")], [{"uid": "L1", "title": "Work", "isarchived": "1"}])
        exp = export([dst_task("u1")])
        with tempfile.TemporaryDirectory() as d:
            cfg = Path(d) / "config.toml"
            cfg.write_text('vdir = "x"\n[lists.col-a]\narchived = true\n')
            code, out = run(inv, exp, ["--config", str(cfg)])
            self.assertEqual(code, 0, out)
            cfg.write_text('vdir = "x"\n')
            code, out = run(inv, exp, ["--config", str(cfg)])
            self.assertEqual(code, 1)
            self.assertIn("L1: archived: source=1 dest=0", out)

    def test_repeat_and_flags_mismatch(self):
        inv = inventory([src_task("u1", repeattype="258", repeatval="1", recurrence="2",
                                  recurrenceendtype="2", recurrenceendrepeats="3")])
        exp = export([dst_task("u1", rrule="FREQ=MONTHLY;COUNT=4")])
        code, out = run(inv, exp)
        self.assertEqual(code, 1)
        self.assertIn("u1: from_completion: source=True dest=False", out)
        self.assertIn("u1: repeat_end:", out)


if __name__ == "__main__":
    unittest.main()
