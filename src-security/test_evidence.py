"""Gate 6, part 2: evidence handles resolve, redact, survive, and cap."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from koma_sec_daemon import evidence  # noqa: E402


class Base(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = Path(tempfile.mkdtemp())
        self.s = evidence.Store(self.dir)

    def tearDown(self) -> None:
        shutil.rmtree(self.dir, ignore_errors=True)


class ResolveTest(Base):
    def test_a_stored_handle_resolves_to_its_bytes(self) -> None:
        h = self.s.put("scan", "row 1\nrow 2\n")
        self.assertEqual(self.s.resolve(h), "row 1\nrow 2\n")
        self.assertTrue(self.s.resolves(h))

    def test_an_unknown_handle_does_not_resolve(self) -> None:
        self.assertFalse(self.s.resolves("never-stored"))
        with self.assertRaises(evidence.EvidenceError):
            self.s.resolve("never-stored")

    def test_a_deleted_artifact_stops_resolving(self) -> None:
        # The gate's point: a handle is not proof on its own.
        h = self.s.put("gone", "x")
        (self.dir / f"{h}.raw").unlink()
        self.assertFalse(self.s.resolves(h))

    def test_identifiers_are_traversal_safe(self) -> None:
        h = self.s.put("../../../etc/passwd", "x")
        self.assertNotIn("/", h)
        self.assertTrue(self.s.resolves(h))

    def test_handles_are_stable_for_one_label(self) -> None:
        self.assertEqual(self.s.put("same", "a"), self.s.put("same", "b"))
        self.assertEqual(self.s.resolve(self.s.put("same", "c")), "c")

    def test_provenance_is_kept(self) -> None:
        h = self.s.put("scan", "x", provenance="sec_ffuf target=a.example")
        self.assertEqual(self.s.meta(h)["provenance"], "sec_ffuf target=a.example")


class RedactionTest(unittest.TestCase):
    def test_an_auth_header_is_scrubbed(self) -> None:
        self.assertNotIn("s3cret", evidence.redact("Authorization: Bearer s3cret\n"))

    def test_a_cookie_is_scrubbed(self) -> None:
        self.assertNotIn("abc123", evidence.redact("Set-Cookie: sid=abc123; Path=/"))

    def test_common_token_shapes_are_scrubbed(self) -> None:
        # The claim is that the secret is GONE; which marker replaced it is an
        # implementation detail (a specific token rule fires before the generic one).
        for raw, secret in (
            ("token=ghp_" + "a" * 30, "a" * 30),
            ("key=sk-" + "b" * 30, "b" * 30),
            ("id AKIA" + "C" * 16, "C" * 16),
        ):
            self.assertNotIn(secret, evidence.redact(raw))

    def test_a_jwt_is_scrubbed(self) -> None:
        jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        self.assertNotIn(jwt, evidence.redact(jwt))

    def test_ordinary_text_is_untouched(self) -> None:
        self.assertEqual(evidence.redact("status 200 OK"), "status 200 OK")

    def test_the_raw_copy_survives_redaction(self) -> None:
        # Redaction is best-effort; the original must still be recoverable.
        dir_ = Path(tempfile.mkdtemp())
        try:
            s = evidence.Store(dir_)
            h = s.put("hdrs", "Authorization: Bearer s3cret\n")
            self.assertIn("s3cret", s.resolve(h))
            self.assertNotIn("s3cret", s.redacted(h))
        finally:
            shutil.rmtree(dir_, ignore_errors=True)


class RetentionTest(Base):
    def test_the_count_cap_drops_the_oldest_first(self) -> None:
        original = evidence.MAX_ARTIFACTS
        evidence.MAX_ARTIFACTS = 5
        try:
            handles = [self.s.put(f"f{i}", "x") for i in range(8)]
            self.assertEqual(len(self.s._index()), 5)
            self.assertFalse(self.s.resolves(handles[0]))
            self.assertTrue(self.s.resolves(handles[-1]))
        finally:
            evidence.MAX_ARTIFACTS = original

    def test_the_size_cap_drops_until_under(self) -> None:
        original = evidence.MAX_BYTES
        evidence.MAX_BYTES = 100
        try:
            for i in range(6):
                self.s.put(f"b{i}", "y" * 40)
            self.assertLessEqual(evidence.Store._total(self.s._index()), 100)
        finally:
            evidence.MAX_BYTES = original


if __name__ == "__main__":
    unittest.main()
