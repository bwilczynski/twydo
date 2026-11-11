import importlib.util
import io
import os
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "capture_fixture.py"
spec = importlib.util.spec_from_file_location("capture_fixture", SCRIPT)
cf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cf)

ICS = (
    "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:a1\r\nSUMMARY:Synthetic one\r\n"
    "DUE:{due}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
)


def run(argv):
    out = io.StringIO()
    with redirect_stdout(out), redirect_stderr(io.StringIO()):
        code = cf.main(argv)
    return code, out.getvalue()


class CaptureTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)
        self.src = self.tmp / "src"
        (self.src / "list1").mkdir(parents=True)
        (self.src / "list2").mkdir()
        self.out = self.tmp / "out"
        (self.src / "list1/a.ics").write_bytes(ICS.format(due="20261104T100000Z").encode())
        (self.src / "list1/ignored.ics").write_text("SUMMARY:Unselected synthetic bystander\n")

    def capture(self, state="before", extra=()):
        return run(["capture", str(self.src), str(self.out), "--operation", "move",
                    "--case", "family", "--state", state, "--collection", "list1",
                    "--collection", "list2", "--resource", "list1/a.ics",
                    "--device", "iphone", "--origin", "device", "--claim", "Move preserves fields",
                    "--test", "tests/move.rs::preserves_fields", "--action", 'Did "a" thing', *extra])

    def test_capture_requires_explicit_state_and_selection(self):
        self.assertEqual(self.capture()[0], 0)
        state = self.out / "move/family/before"
        self.assertEqual((state / "list1/a.ics").read_bytes(), (self.src / "list1/a.ics").read_bytes())
        self.assertFalse((state / "list1/ignored.ics").exists())
        manifest = (state.parent / "case.toml").read_text()
        import tomllib
        data = tomllib.loads(manifest)
        self.assertEqual(data["states"]["before"]["members"], ["list1/a.ics"])
        self.assertEqual(data["states"]["before"]["action"], 'Did "a" thing')
        self.assertEqual(data["origin"], "device")

    def test_capture_multiple_collections_and_empty_destination(self):
        self.assertEqual(self.capture()[0], 0)
        self.assertTrue((self.out / "move/family/before/list2").is_dir())
        self.assertEqual((self.out / "move/family/before/list2/.gitkeep").read_bytes(), b"")
        (self.src / "list2/b.ics").write_text("SUMMARY:Synthetic destination\n")
        self.assertEqual(self.capture("after", ["--resource", "list2/b.ics"])[0], 0)
        self.assertTrue((self.out / "move/family/after/list2/b.ics").is_file())

    def test_capture_does_not_read_previous_steps(self):
        self.assertEqual(self.capture("after")[0], 0)
        (self.src / "list1/a.ics").write_bytes(b"SUMMARY:Earlier state\r\n")
        self.assertEqual(self.capture("before")[0], 0)
        self.assertEqual((self.out / "move/family/before/list1/a.ics").read_bytes(), b"SUMMARY:Earlier state\r\n")
        self.assertFalse(list(self.out.rglob("changes.diff")))
        self.assertFalse(list(self.out.rglob("step.toml")))

    def test_capture_preserves_selected_bytes_and_source(self):
        path = self.src / "list1/a.ics"
        os.utime(path, (1000000000, 1000000000))
        before = path.read_bytes(), path.stat().st_mtime_ns
        self.assertEqual(self.capture()[0], 0)
        self.assertEqual((path.read_bytes(), path.stat().st_mtime_ns), before)

    def test_refuses_existing_state_bad_names_and_escaping_selection(self):
        self.assertEqual(self.capture()[0], 0)
        self.assertEqual(self.capture()[0], 2)
        self.assertEqual(self.capture("../escape")[0], 2)
        self.assertEqual(self.capture("other", ["--resource", "../private.ics"])[0], 2)
        self.assertFalse((self.out / "move/family/other").exists())

    def test_refuses_symlinks_and_missing_selection_without_partial_capture(self):
        (self.src / "list2/alias.ics").symlink_to(self.tmp / "outside.ics")
        self.assertEqual(self.capture("other", ["--resource", "list2/alias.ics"])[0], 2)
        self.assertFalse((self.out / "move/family/other").exists())


class ScanTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.dir = Path(self._tmp.name)

    def test_scan_flags_private_data(self):
        (self.dir / "a.ics").write_text(
            "BEGIN:VTODO\r\nSUMMARY:S\r\nATTENDEE:mailto:x@corp.test\r\n"
            "URL:https://nas.local/dav\r\nLOCATION;X=1:Somewhere\r\n"
            "DESCRIPTION:@actionvalue:555\r\nEND:VTODO\r\n")
        (self.dir / "notes.txt").write_text("hi")
        code, out = run(["scan", str(self.dir)])
        self.assertEqual(code, 1)
        self.assertIn("a.ics:3:", out)
        self.assertIn("a.ics:4:", out)
        self.assertIn("a.ics:5:", out)
        self.assertIn("a.ics:6:", out)
        self.assertIn("notes.txt", out)

    def test_scan_unfolds_lines(self):
        (self.dir / "a.ics").write_text(
            "BEGIN:VTODO\r\nSUMMARY:S\r\nDESCRIPTION:see bob@exam\r\n ple.net now\r\nEND:VTODO\r\n")
        code, out = run(["scan", str(self.dir)])
        self.assertEqual(code, 1)
        self.assertIn("a.ics:3:", out)

    def test_scan_clean_passes(self):
        (self.dir / "a.ics").write_text(
            "BEGIN:VTODO\r\nSUMMARY:Synthetic one\r\n"
            "DESCRIPTION:see https://example.com/x @tasktype:0 @startdate:2026-11-04\r\n"
            "END:VTODO\r\n")
        (self.dir / "displayname").write_text("Test")
        (self.dir / "case.toml").write_text('claim = "Synthetic claim"\n')
        code, out = run(["scan", str(self.dir)])
        self.assertEqual(code, 0, out)
        self.assertIn("Synthetic one", out)
        self.assertIn("Confirm every line above is synthetic before committing.", out)

    def test_scan_diff_prefixes_do_not_hide_properties(self):
        (self.dir / "changes.diff").write_text("+ORGANIZER:mailto:q\n-LOCATION:x\n")
        code, out = run(["scan", str(self.dir)])
        self.assertEqual(code, 1)
        self.assertIn("changes.diff:1:", out)
        self.assertIn("file type not allowed", out)

    def test_scan_missing_or_empty_dir_is_error(self):
        code, _ = run(["scan", str(self.dir / "nope")])
        self.assertEqual(code, 2)
        code, _ = run(["scan", str(self.dir)])
        self.assertEqual(code, 2)

    def test_scan_lists_free_text_for_confirmation(self):
        (self.dir / "displayname").write_text("Synthetic List\n")
        (self.dir / "a.ics").write_text(
            "BEGIN:VTODO\r\nSUMMARY:S\r\nCOMMENT:comment-val\r\nCATEGORIES:cat-val\r\n"
            "X-WR-CALNAME:calname-val\r\nX-2DOAPP-METADATA:@tasktype:0\r\nEND:VTODO\r\n")
        code, out = run(["scan", str(self.dir)])
        self.assertEqual(code, 0, out)
        for v in ("Synthetic List", "comment-val", "cat-val", "calname-val", "@tasktype:0"):
            self.assertIn(v, out)

    def test_scan_dotless_email_but_not_tokens(self):
        (self.dir / "a.ics").write_text("SUMMARY:mail x@localhost\r\nDESCRIPTION:y@nas\r\n")
        code, out = run(["scan", str(self.dir)])
        self.assertEqual(code, 1)
        self.assertIn("a.ics:1:", out)
        self.assertIn("a.ics:2:", out)
        (self.dir / "a.ics").write_text("DESCRIPTION:@tasktype:0 @startdate:2026-11-04\r\n"
                                        "X-2DOAPP-METADATA:@startdate:2026-11-04\r\n")
        code, out = run(["scan", str(self.dir)])
        self.assertEqual(code, 0, out)


if __name__ == "__main__":
    unittest.main()
