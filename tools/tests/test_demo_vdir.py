import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "demo_vdir.py"
spec = importlib.util.spec_from_file_location("demo_vdir", SCRIPT)
dv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dv)

SAMPLE = Path(__file__).resolve().parents[2] / "crates/twydo-core/tests/data/sample-vdir"


def todo(*lines):
    return "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\n" + "".join(l + "\r\n" for l in lines) + "END:VTODO\r\nEND:VCALENDAR\r\n"


class ShiftTest(unittest.TestCase):
    def test_date_value_shifted(self):
        out = dv.shift_ics(todo("DUE;VALUE=DATE:20261105"), 10)
        self.assertIn("DUE;VALUE=DATE:20261115\r\n", out)

    def test_datetime_shifted_keeps_tzid_and_time(self):
        out = dv.shift_ics(todo("DUE;TZID=America/New_York:20261104T150000"), 30)
        self.assertIn("DUE;TZID=America/New_York:20261204T150000\r\n", out)

    def test_utc_stamps_shifted(self):
        out = dv.shift_ics(
            todo(
                "COMPLETED:20261102T170000Z",
                "DTSTART;VALUE=DATE:20261110",
                "CREATED:20261101T080000Z",
                "DTSTAMP:20261101T080000Z",
                "LAST-MODIFIED:20261101T080000Z",
            ),
            1,
        )
        for line in (
            "COMPLETED:20261103T170000Z",
            "DTSTART;VALUE=DATE:20261111",
            "CREATED:20261102T080000Z",
            "DTSTAMP:20261102T080000Z",
            "LAST-MODIFIED:20261102T080000Z",
        ):
            self.assertIn(line + "\r\n", out)

    def test_weekly_byday_follows_due(self):
        out = dv.shift_ics(todo("DUE;VALUE=DATE:20261104", "RRULE:FREQ=WEEKLY;BYDAY=WE"), 1)
        self.assertIn("RRULE:FREQ=WEEKLY;BYDAY=TH\r\n", out)

    def test_monthly_bymonthday_follows_due(self):
        out = dv.shift_ics(todo("DUE;VALUE=DATE:20261107", "RRULE:FREQ=MONTHLY;BYMONTHDAY=7"), 3)
        self.assertIn("DUE;VALUE=DATE:20261110\r\n", out)
        self.assertIn("RRULE:FREQ=MONTHLY;BYMONTHDAY=10\r\n", out)

    def test_monthly_day_clamped_to_28(self):
        out = dv.shift_ics(todo("DUE;VALUE=DATE:20261107", "RRULE:FREQ=MONTHLY;BYMONTHDAY=7"), 23)
        self.assertIn("DUE;VALUE=DATE:20261128\r\n", out)
        self.assertIn("RRULE:FREQ=MONTHLY;BYMONTHDAY=28\r\n", out)

    def test_relative_trigger_untouched(self):
        src = todo("DUE;VALUE=DATE:20261105", "BEGIN:VALARM", "TRIGGER:-PT15M", "END:VALARM")
        self.assertIn("TRIGGER:-PT15M\r\n", dv.shift_ics(src, 5))

    def test_folded_date_line_rejected(self):
        with self.assertRaises(ValueError):
            dv.shift_ics(todo("DUE;TZID=America/New_York:2026", " 1104T150000"), 1)


class GenerateTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.out = Path(self._tmp.name) / "vdir"

    def tearDown(self):
        self._tmp.cleanup()

    def files(self, root):
        return sorted(p.relative_to(root) for p in root.rglob("*") if p.is_file() and p.name != "README.md")

    def test_offset_zero_copies_sample_exactly(self):
        self.assertEqual(dv.main([str(self.out), "--today", "2026-11-04"]), 0)
        self.assertEqual(self.files(self.out), self.files(SAMPLE))
        for rel in self.files(SAMPLE):
            self.assertEqual((self.out / rel).read_bytes(), (SAMPLE / rel).read_bytes(), rel)

    def test_later_today_keeps_every_collection_file(self):
        self.assertEqual(dv.main([str(self.out), "--today", "2026-12-01"]), 0)
        self.assertEqual(self.files(self.out), self.files(SAMPLE))
        self.assertIn(
            "DUE;VALUE=DATE:20261202",
            (self.out / "inbox/pay-electricity.ics").read_text(),
        )


if __name__ == "__main__":
    unittest.main()
