"""Evidence validation tests exercise files, membership and claimed observations."""
import tempfile
import unittest
from pathlib import Path

from tools.evidence import load_case, validate


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "tests").mkdir()
        (self.root / "tests/claim.rs").write_text("fn preserves_identity() {}\n")
        (self.root / "compat").mkdir()
        vocabulary = self.root / "crates/twydo-core/src/compat/contracts.rs"
        vocabulary.parent.mkdir(parents=True)
        vocabulary.write_text('''contracts! {
    EditTitleResource => EditTitle, "edit-title.resource", "editing a title";
    EditNotesResource => EditNotes, "edit-notes.resource", "editing notes";
}''')

    def case(self, extra="", origin="device", members='["list1/task.ics"]'):
        case = self.root / "fixtures/edit-title/resource"
        (case / "before/list1").mkdir(parents=True, exist_ok=True)
        (case / "before/list1/task.ics").write_bytes(b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n")
        (case / "case.toml").write_text(f'''claim = "A title edit preserves identity"
origin = "{origin}"
observed = "Phone retained task identity after renaming"
tests = ["tests/claim.rs::preserves_identity"]
[provenance]
source_revision = "unknown"
source_path = "unknown"
[environment]
iphone_2do = "unknown"
[states.before]
path = "before"
collections = ["list1"]
members = {members}
{extra}''')
        return case

    def test_complete_case_loads_without_altering_bytes(self):
        case = self.case()
        before = (case / "before/list1/task.ics").read_bytes()
        self.assertEqual(validate(self.root), [])
        self.assertEqual(load_case(self.root, "edit-title/resource")["origin"], "device")
        self.assertEqual((case / "before/list1/task.ics").read_bytes(), before)

    def test_rejects_duplicate_or_escaping_case_paths(self):
        self.case()
        with self.assertRaises(ValueError):
            load_case(self.root, "../tests")
        path = self.root / "fixtures/edit-title/resource/case.toml"
        path.write_text(path.read_text().replace('path = "before"', 'path = "../../../tests"'))
        self.assertTrue(any("path" in e for e in validate(self.root)))

    def test_rejects_missing_or_unowned_state_resources(self):
        self.case(members='["list1/missing.ics"]')
        errors = validate(self.root)
        self.assertTrue(any("missing.ics" in e for e in errors))
        self.assertTrue(any("task.ics" in e for e in errors))

    def test_empty_collection_state_is_valid(self):
        case = self.case(members="[]")
        (case / "before/list1/task.ics").unlink()
        self.assertEqual(validate(self.root), [])

    def test_empty_collection_marker_is_not_a_task_resource(self):
        case = self.case(members="[]")
        (case / "before/list1/task.ics").unlink()
        marker = case / "before/list1/.gitkeep"
        marker.write_bytes(b"")
        self.assertEqual(validate(self.root), [])
        marker.write_text("hidden content")
        self.assertTrue(validate(self.root))

    def test_rejects_inherited_states_and_verdicts(self):
        self.case(extra='base_step = "previous"\nverdict = "pass"\n')
        self.assertTrue(any("unknown" in e for e in validate(self.root)))

    def test_unknown_provenance_is_explicit(self):
        case = self.case()
        p = case / "case.toml"
        p.write_text(p.read_text().replace('source_revision = "unknown"\n', ""))
        self.assertTrue(any("provenance" in e for e in validate(self.root)))

    def test_test_reference_must_exist(self):
        case = self.case()
        p = case / "case.toml"
        p.write_text(p.read_text().replace("preserves_identity", "missing_test"))
        self.assertTrue(any("test" in e for e in validate(self.root)))

    def test_symlink_cannot_escape_case(self):
        case = self.case()
        (case / "before/list1/task.ics").unlink()
        (case / "before/list1/task.ics").symlink_to(self.root / "tests/claim.rs")
        self.assertTrue(any("path" in e or "symlink" in e for e in validate(self.root)))

    def test_synthetic_only_cannot_prove_device_requirement(self):
        self.case(origin="synthetic")
        (self.root / "compat/matrix.toml").write_text('''matrix_version = 2
[[contract]]
id = "edit-title.resource"
iphone_baseline = "pass"
twydo_to_iphone = "pass"
iphone_to_twydo = "pass"
evidence = ["edit-title/resource"]
''')
        self.assertTrue(any("observation" in e for e in validate(self.root)))

    def test_protocol_only_cannot_prove_a_phone_direction(self):
        self.case(origin="protocol")
        (self.root / "compat/matrix.toml").write_text('''matrix_version = 2
[[contract]]
id = "edit-title.resource"
iphone_baseline = "pass"
twydo_to_iphone = "pass"
iphone_to_twydo = "pass"
evidence = ["edit-title/resource"]
''')
        self.assertTrue(any("device observation" in e for e in validate(self.root)))

    def test_contract_evidence_must_resolve(self):
        self.case()
        (self.root / "compat/matrix.toml").write_text('''matrix_version = 2
[[contract]]
id = "edit-title.resource"
evidence = ["edit-title/missing"]
''')
        self.assertTrue(any("missing" in e for e in validate(self.root)))

    def test_contract_vocabulary_comes_from_the_typed_gate(self):
        self.case()
        (self.root / "compat/matrix.toml").write_text('''matrix_version = 2
[[contract]]
id = "edit-title.anything"
evidence = ["edit-title/resource"]
''')
        self.assertTrue(any("unknown" in e for e in validate(self.root)))

    def test_resources_outside_any_case_are_rejected(self):
        self.case()
        (self.root / "fixtures/stray.ics").write_text("SUMMARY:Synthetic stray\n")
        self.assertTrue(any("unowned" in e for e in validate(self.root)))

    def test_phone_roundtrip_cannot_prove_atomicity(self):
        self.case()
        (self.root / "compat/matrix.toml").write_text('''matrix_version = 2
[[contract]]
id = "list-metadata.collection"
iphone_baseline = "pass"
twydo_to_iphone = "pass"
iphone_to_twydo = "pass"
atomicity = "pass"
evidence = ["edit-title/resource"]
''')
        self.assertTrue(any("protocol observation" in e for e in validate(self.root)))

    def test_cross_operation_reuse_requires_an_explicit_claim(self):
        self.case()
        (self.root / "compat/matrix.toml").write_text('''matrix_version = 2
[[contract]]
id = "edit-notes.resource"
evidence = ["edit-title/resource"]
''')
        self.assertTrue(any("cross" in e for e in validate(self.root)))
