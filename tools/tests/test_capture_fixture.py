import importlib.util
import io
import os
import tempfile
import unittest
from contextlib import redirect_stdout
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
    with redirect_stdout(out):
        code = cf.main(argv)
    return code, out.getvalue()


class CaptureTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)
        self.src = self.tmp / "src"
        self.src.mkdir()
        self.out = self.tmp / "out"
        (self.src / "displayname").write_text("Test List")
        (self.src / "color").write_text("#112233")
        (self.src / "ignored.txt").write_text("x")

    def capture(self, step):
        return run(["capture", str(self.src), str(self.out), "--scenario", "s1",
                    "--step", step, "--device", "mac", "--action", 'Did "a" thing'])

    def test_capture_writes_step_and_diff(self):
        (self.src / "a.ics").write_text(ICS.format(due="20261104T100000Z"))
        code, _ = self.capture("01-first")
        self.assertEqual(code, 0)
        d1 = self.out / "s1" / "01-first"
        self.assertFalse((d1 / "ignored.txt").exists())
        diff1 = (d1 / "changes.diff").read_text()
        for name in ("a.ics", "color", "displayname"):
            self.assertIn(f"added: {name}", diff1)
        toml = (d1 / "step.toml").read_text()
        self.assertIn('action = "Did \\"a\\" thing"', toml)
        self.assertIn('device = "mac"', toml)
        self.assertIn("captured_at = ", toml)

        (self.src / "a.ics").write_text(ICS.format(due="20261105T100000Z"))
        self.capture("02-second")
        diff2 = (self.out / "s1" / "02-second" / "changes.diff").read_text()
        self.assertIn("-DUE:20261104T100000Z", diff2)
        self.assertIn("+DUE:20261105T100000Z", diff2)
        self.assertNotIn("added:", diff2)

    def test_removed_and_previous_is_greatest_lower(self):
        (self.src / "a.ics").write_text(ICS.format(due="1"))
        self.capture("01-a")
        self.capture("03-c")
        (self.src / "a.ics").unlink()
        self.capture("02-b")  # previous is 01-a
        self.assertIn("removed: a.ics", (self.out / "s1" / "02-b" / "changes.diff").read_text())

    def test_capture_never_modifies_source(self):
        (self.src / "a.ics").write_text(ICS.format(due="1"))
        os.utime(self.src / "a.ics", (1000000000, 1000000000))
        files = sorted(self.src.iterdir())
        before = {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in files}
        self.capture("01-first")
        after = {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in sorted(self.src.iterdir())}
        self.assertEqual(before, after)


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
        (self.dir / "step.toml").write_text('scenario = "s1"\n')
        (self.dir / "changes.diff").write_text("added: a.ics\n")
        code, out = run(["scan", str(self.dir)])
        self.assertEqual(code, 0, out)
        self.assertIn("Synthetic one", out)
        self.assertIn("Confirm every line above is synthetic before committing.", out)

    def test_scan_diff_prefixes_do_not_hide_properties(self):
        (self.dir / "changes.diff").write_text("+ORGANIZER:mailto:q\n-LOCATION:x\n")
        code, out = run(["scan", str(self.dir)])
        self.assertEqual(code, 1)
        self.assertIn("changes.diff:1:", out)
        self.assertIn("changes.diff:2:", out)


if __name__ == "__main__":
    unittest.main()
