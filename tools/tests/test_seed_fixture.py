import importlib.util
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "seed_fixture.py"
spec = importlib.util.spec_from_file_location("seed_fixture", SCRIPT)
sf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sf)

# A parent and a child; the child's <2Do Meta> folds the parent UID across two lines.
PARENT = (
    "BEGIN:VCALENDAR\nBEGIN:VTODO\nUID:aaaa1111\nSUMMARY:P\n"
    "CREATED:20260101T000000Z\nDTSTAMP:20260101T000000Z\nEND:VTODO\nEND:VCALENDAR\n"
)
CHILD = (
    "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:bbbb2222\r\nRELATED-TO:aaaa1111\r\n"
    "SUMMARY:p1\r\nLAST-MODIFIED:20260101T000000Z\r\n"
    "X-2DOAPP-METADATA:<2Do Meta>%7B%22parentuid%22%3A%22aaaa\r\n"
    " 1111%22%2C%22RUID%22%3A%22bbbb2222%22%7D</2Do Meta>\r\n"
    "END:VTODO\r\nEND:VCALENDAR\r\n"
)
NOW = datetime(2026, 11, 2, 12, 0, 0, tzinfo=timezone.utc)


class SeedTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.src, self.dest = root / "src", root / "dest"
        self.src.mkdir()
        self.dest.mkdir()
        (self.src / "aaaa1111.ics").write_text(PARENT)
        (self.src / "bbbb2222.ics").write_bytes(CHILD.encode())
        uids = iter(["new1", "new2", "new3", "new4"])
        self.new_uid = lambda: next(uids)

    def seed(self, **kw):
        return sf.seed(self.src, self.dest, now=NOW, new_uid=self.new_uid, **kw)

    def read(self, name):
        return (self.dest / name).read_bytes().decode()

    def test_rewrites_uids_links_and_stamps(self):
        self.assertEqual(self.seed(), [("P", "new1"), ("p1", "new2")])
        parent = self.read("new1.ics")
        self.assertIn("UID:new1\n", parent)
        self.assertIn("CREATED:20261102T120000Z\n", parent)
        self.assertIn("DTSTAMP:20261102T120000Z\n", parent)
        child = self.read("new2.ics")
        self.assertIn("\r\n", child)
        unfolded = "\n".join(sf.unfold(child.replace("\r\n", "\n")))
        self.assertIn("RELATED-TO:new1", unfolded)
        self.assertIn("%22parentuid%22%3A%22new1%22", unfolded)
        self.assertIn("%22RUID%22%3A%22new2%22", unfolded)
        self.assertIn("LAST-MODIFIED:20261102T120000Z", unfolded)
        self.assertNotIn("aaaa1111", unfolded)
        self.assertNotIn("bbbb2222", unfolded)

    def test_rename_and_only(self):
        out = self.seed(only=["P"], rename={"P": "Q"})
        self.assertEqual(out, [("Q", "new1")])
        self.assertIn("SUMMARY:Q\n", self.read("new1.ics"))
        self.assertEqual(len(list(self.dest.iterdir())), 1)

    def test_child_without_parent_is_refused(self):
        with self.assertRaises(SystemExit):
            self.seed(only=["p1"])

    def test_dry_run_writes_nothing(self):
        self.seed(dry_run=True)
        self.assertEqual(list(self.dest.iterdir()), [])

    def test_long_lines_fold_at_75_octets(self):
        line = "DESCRIPTION:" + "ą" * 60
        folded = sf.fold(line)
        self.assertTrue(all(len(p.encode()) <= 75 for p in folded.split("\n")))
        self.assertEqual(sf.unfold(folded), [line])


if __name__ == "__main__":
    unittest.main()
