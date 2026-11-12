"""Host tests for gen_adr_index.py (run by gate.sh and CI)."""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gen_adr_index


def adr(number: str, title: str, status: str, extra: str = "") -> str:
    return f"# ADR {number}: {title}\n\n- Status: {status}\n- Date: 2026-11-06\n{extra}\n## Context\n"


class GenAdrIndexTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.dir = self.root / "docs" / "adr"
        self.dir.mkdir(parents=True)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def put(self, name: str, text: str) -> None:
        (self.dir / name).write_text(text, encoding="utf-8")

    def run_main(self, *args: str) -> int:
        return gen_adr_index.main([*args, "--root", str(self.root)])

    def test_writes_then_checks_clean(self) -> None:
        self.put("0001-one.md", adr("0001", "One", "accepted; amended by ADR 0002"))
        self.put("0002-two.md", adr("0002", "Two", "proposed"))
        self.assertEqual(self.run_main(), 0)
        text = (self.dir / "README.md").read_text(encoding="utf-8")
        self.assertIn(
            "| [0001](0001-one.md) | One | accepted; amended by [ADR 0002](0002-two.md) |",
            text,
        )
        self.assertEqual(self.run_main("--check"), 0)

    def test_check_fails_when_out_of_date(self) -> None:
        self.put("0001-one.md", adr("0001", "One", "accepted"))
        self.assertEqual(self.run_main(), 0)
        self.put("0002-two.md", adr("0002", "Two", "proposed"))
        self.assertEqual(self.run_main("--check"), 1)

    def test_status_must_name_a_known_state(self) -> None:
        self.put("0001-one.md", adr("0001", "One", "done"))
        _, errors = gen_adr_index.build(self.dir)
        self.assertTrue(any("Status must start" in e for e in errors), errors)

    def test_title_number_must_match_the_file(self) -> None:
        self.put("0001-one.md", adr("0002", "One", "accepted"))
        _, errors = gen_adr_index.build(self.dir)
        self.assertTrue(any("first line" in e for e in errors), errors)

    def test_numbers_have_no_gaps(self) -> None:
        self.put("0001-one.md", adr("0001", "One", "accepted"))
        self.put("0003-three.md", adr("0003", "Three", "accepted"))
        _, errors = gen_adr_index.build(self.dir)
        self.assertTrue(any("without gaps" in e for e in errors), errors)

    def test_superseded_adr_must_say_so(self) -> None:
        self.put("0001-one.md", adr("0001", "One", "accepted"))
        self.put(
            "0002-two.md",
            adr("0002", "Two", "accepted", "- Supersedes: ADR 0001, rule 3\n"),
        )
        _, errors = gen_adr_index.build(self.dir)
        self.assertTrue(
            any("0001-one.md: superseded by ADR 0002" in e for e in errors), errors
        )
        self.put(
            "0001-one.md", adr("0001", "One", "accepted; rule 3 superseded by ADR 0002")
        )
        _, errors = gen_adr_index.build(self.dir)
        self.assertEqual(errors, [])

    def test_wrapped_status_lines_join(self) -> None:
        self.put(
            "0001-one.md",
            "# ADR 0001: One\n\n- Status: accepted; a long\n  reason\n- Date: 2026-11-06\n",
        )
        _, errors = gen_adr_index.build(self.dir)
        self.assertEqual(errors, [])
        text, _ = gen_adr_index.build(self.dir)
        self.assertIn("accepted; a long reason", text)


if __name__ == "__main__":
    unittest.main()
