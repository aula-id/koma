"""Handshake negotiation: a mismatch is refused, never guessed.

Run with `python3 -m unittest discover -s src-security -t src-security`, or the
single file. Stdlib only, so it runs on a bare interpreter.
"""
import unittest

from koma_sec_daemon.protocol import PROTOCOL_VERSION, negotiate


class NegotiateTest(unittest.TestCase):
    def test_matching_revision_is_accepted(self):
        self.assertIsNone(negotiate({"v": PROTOCOL_VERSION}))

    def test_missing_revision_is_refused(self):
        # A half-migrated pair must not run with unmigrated capability metadata.
        self.assertIsNotNone(negotiate({}))

    def test_major_mismatch_is_refused(self):
        self.assertIsNotNone(negotiate({"v": PROTOCOL_VERSION + 1}))

    def test_non_integer_revision_is_refused(self):
        self.assertIsNotNone(negotiate({"v": "two"}))
        self.assertIsNotNone(negotiate({"v": None}))

    def test_refusal_names_the_reason(self):
        # The parent surfaces this to the user, so it must say which side is which.
        refusal = negotiate({"v": PROTOCOL_VERSION + 1})
        self.assertIn("mismatch", refusal)
        self.assertIn(str(PROTOCOL_VERSION), refusal)


if __name__ == "__main__":
    unittest.main()
