"""Typed-result contract: the wrap boundary and the excerpt ceiling."""
import json
import unittest

from koma_sec_daemon import verdict


class AsTypedTest(unittest.TestCase):
    def test_bare_string_is_labelled_untyped(self):
        # A handler not yet migrated must not reach the parent as a raw string.
        frame = json.loads(verdict.as_typed("nmap output"))
        self.assertEqual(frame["verdict"], verdict.UNTYPED)
        self.assertEqual(frame["excerpt"], "nmap output")

    def test_typed_result_passes_through_unchanged(self):
        original = verdict.typed(verdict.NOT_FOUND, "nothing here")
        self.assertEqual(verdict.as_typed(original), original)

    def test_none_becomes_an_empty_untyped_result(self):
        # Never an empty string: the parent would have no verdict to read.
        self.assertEqual(json.loads(verdict.as_typed(None))["excerpt"], "")

    def test_non_result_text_is_not_mistaken_for_a_verdict(self):
        # A handler whose output happens to be JSON but not a verdict.
        frame = json.loads(verdict.as_typed('{"count": 3}'))
        self.assertEqual(frame["verdict"], verdict.UNTYPED)


class TypedTest(unittest.TestCase):
    def test_excerpt_is_clipped_with_a_marker(self):
        got = json.loads(verdict.typed(verdict.CONFIRMED, "x" * (verdict.EXCERPT_LIMIT + 50)))
        self.assertLessEqual(len(got["excerpt"]), verdict.EXCERPT_LIMIT + 80)
        self.assertIn("more chars", got["excerpt"])

    def test_the_marker_names_the_handle_when_there_is_one(self):
        got = json.loads(
            verdict.typed(verdict.CONFIRMED, "x" * (verdict.EXCERPT_LIMIT + 10), handle="rec-1")
        )
        self.assertIn("rec-1", got["excerpt"])
        self.assertEqual(got["handle"], "rec-1")

    def test_unknown_verdict_raises(self):
        with self.assertRaises(ValueError):
            verdict.typed("probably")

    def test_unavailable_carries_the_hint(self):
        got = json.loads(verdict.unavailable("nuclei", "pip install x"))
        self.assertEqual(got["verdict"], verdict.UNAVAILABLE)
        self.assertIn("pip install x", got["excerpt"])

    def test_every_declared_verdict_serializes(self):
        for v in verdict.VERDICTS:
            self.assertEqual(json.loads(verdict.typed(v))["verdict"], v)


if __name__ == "__main__":
    unittest.main()
