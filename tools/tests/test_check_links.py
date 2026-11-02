import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "check_links.py"
spec = importlib.util.spec_from_file_location("check_links", SCRIPT)
cl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cl)


class BrokenLinksTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def write(self, rel, text=""):
        p = self.root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)

    def test_relative_link_ok(self):
        self.write("a.md", "see [x](sub/b.md)\n")
        self.write("sub/b.md")
        self.assertEqual(cl.broken_links(self.root), [])

    def test_missing_target_reported(self):
        self.write("a.md", "one\ntwo\nsee [x](nope.md)\n")
        self.assertEqual(cl.broken_links(self.root), [(Path("a.md"), 3, "nope.md")])

    def test_anchor_and_external_ignored(self):
        self.write(
            "a.md",
            "[x](https://e.com) [x](#sec) [x](mailto:a@b.c) [x](b.md#sec)\n",
        )
        self.write("b.md")
        self.assertEqual(cl.broken_links(self.root), [])

    def test_link_to_directory_ok(self):
        self.write("a.md", "[x](sub/)\n")
        (self.root / "sub").mkdir()
        self.assertEqual(cl.broken_links(self.root), [])

    def test_skips_target_and_git(self):
        self.write("target/x.md", "[x](nope.md)\n")
        self.write(".git/x.md", "[x](nope.md)\n")
        self.assertEqual(cl.broken_links(self.root), [])

    def test_link_inside_code_fence_ignored(self):
        self.write("a.md", "```\n[x](nope.md)\n```\n")
        self.assertEqual(cl.broken_links(self.root), [])

    def test_link_inside_inline_code_ignored(self):
        self.write("a.md", "write `[x](nope.md)` or ``a](b|c)``\n")
        self.assertEqual(cl.broken_links(self.root), [])

    def test_link_from_subdirectory_resolves_relative_to_file(self):
        self.write("docs/a.md", "[x](../b.md)\n")
        self.write("b.md")
        self.assertEqual(cl.broken_links(self.root), [])


if __name__ == "__main__":
    unittest.main()
