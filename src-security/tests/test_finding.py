"""Gate 6, part 3: a confirmation must be backed by evidence that resolves."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from koma_sec_daemon import evidence, finding  # noqa: E402


class Base(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = Path(tempfile.mkdtemp())
        self.s = evidence.Store(self.dir)

    def tearDown(self) -> None:
        shutil.rmtree(self.dir, ignore_errors=True)


class ConfirmTest(Base):
    def test_a_confirmed_finding_with_resolving_evidence_passes(self) -> None:
        h = self.s.put("scan", "row")
        finding.Finding("f1", "confirmed", handles=[h]).check(self.s)

    def test_a_confirmed_finding_with_no_handle_is_refused(self) -> None:
        with self.assertRaises(finding.NotConfirmed):
            finding.Finding("f1", "confirmed").check(self.s)

    def test_a_confirmed_finding_whose_evidence_vanished_is_refused(self) -> None:
        # This is the gate's core case: a verdict alone must not confirm.
        h = self.s.put("scan", "row")
        f = finding.Finding("f1", "confirmed", handles=[h])
        (self.dir / f"{h}.raw").unlink()
        with self.assertRaises(finding.NotConfirmed) as ctx:
            f.check(self.s)
        self.assertIn("no longer resolves", str(ctx.exception))

    def test_a_confirmed_finding_with_one_good_and_one_dead_handle_is_refused(self) -> None:
        good = self.s.put("a", "x")
        dead = self.s.put("b", "y")
        (self.dir / f"{dead}.raw").unlink()
        with self.assertRaises(finding.NotConfirmed):
            finding.Finding("f1", "confirmed", handles=[good, dead]).check(self.s)

    def test_other_buckets_need_no_evidence(self) -> None:
        for bucket in ("unproven", "disproved", "blocked", "untested"):
            finding.Finding("f", bucket).check(self.s)

    def test_an_unknown_bucket_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            finding.Finding("f", "maybe")


class ReportTest(Base):
    def test_the_report_keeps_untested_separate_from_unproven(self) -> None:
        # Collapsing these is how a gap starts reading as a negative result.
        out = finding.report(
            [finding.Finding("a", "untested"), finding.Finding("b", "unproven")],
            self.s,
        )
        self.assertEqual([f["id"] for f in out["untested"]], ["a"])
        self.assertEqual([f["id"] for f in out["unproven"]], ["b"])

    def test_a_confirmation_losing_its_evidence_is_demoted_not_dropped(self) -> None:
        h = self.s.put("scan", "row")
        f = finding.Finding("f1", "confirmed", handles=[h])
        (self.dir / f"{h}.raw").unlink()
        out = finding.report([f], self.s)
        self.assertEqual(out["confirmed"], [])
        self.assertEqual([r["id"] for r in out["unproven"]], ["f1"])
        self.assertTrue(out["unproven"][0]["demoted"])

    def test_every_bucket_is_present_even_when_empty(self) -> None:
        out = finding.report([], self.s)
        self.assertEqual(sorted(out), sorted(finding.BUCKETS))


if __name__ == "__main__":
    unittest.main()
