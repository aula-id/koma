"""Dynamic target identity: stable, session+cwd scoped, collision-free."""
import unittest

from koma_sec_daemon import target

BASE = dict(session="s-1", cwd="/work/repo", verb="red", host="localhost")


class FingerprintTest(unittest.TestCase):
    def test_same_context_and_target_is_byte_identical(self):
        self.assertEqual(
            target.fingerprint(**BASE, port=8080), target.fingerprint(**BASE, port=8080)
        )

    def test_two_sessions_on_one_checkout_never_collide(self):
        a = target.fingerprint(**BASE, port=8080)
        b = target.fingerprint(**{**BASE, "session": "s-2"}, port=8080)
        self.assertNotEqual(a, b)

    def test_two_checkouts_never_collide(self):
        a = target.fingerprint(**BASE)
        b = target.fingerprint(**{**BASE, "cwd": "/work/other"}, port=None)
        self.assertNotEqual(a, b)

    def test_port_and_path_are_part_of_identity(self):
        a = target.fingerprint(**BASE, port=8080)
        b = target.fingerprint(**BASE, port=8443)
        c = target.fingerprint(**BASE, port=8080, path="/api")
        self.assertEqual(len({a, b, c}), 3)

    def test_host_case_and_padding_are_normalised(self):
        self.assertEqual(
            target.fingerprint(**{**BASE, "host": "LOCALHOST "}),
            target.fingerprint(**BASE),
        )

    def test_the_result_stays_readable_and_bounded(self):
        got = target.fingerprint(**{**BASE, "cwd": "/very/" + "deep/" * 40 + "repo"}, port=80)
        self.assertTrue(got.startswith("red-localhost-80@"))
        # The cwd is hashed, so its depth does not grow the identifier.
        self.assertLess(len(got), 90)

    def test_session_and_cwd_are_required(self):
        for missing in ("session", "cwd"):
            with self.subTest(missing=missing):
                with self.assertRaises(ValueError):
                    target.fingerprint(**{**BASE, missing: ""})

    def test_unknown_verb_raises(self):
        with self.assertRaises(ValueError):
            target.fingerprint(**{**BASE, "verb": "ops"})

    def test_verb_is_read_from_a_tool_name(self):
        self.assertEqual(target.verb_of("red_portscan"), "red")
        self.assertEqual(target.verb_of("blue_verify"), "blue")
        self.assertIsNone(target.verb_of("build"))

    def test_digest_matches_the_rust_placement_key(self):
        # tool/scope.rs computes the same value; a divergence forks every record.
        # Pinned literally so a change on either side is caught here.
        self.assertEqual(target._digest("/work/repo"), "1cd4fc81c8")
        self.assertEqual(target._digest(""), "cbf29ce484")

    def test_record_filename_matches_the_rust_placement_key(self):
        # tool/scope.rs::record_path slugs the same way; the vectors are shared.
        self.assertEqual(target.record_filename(host="LocalHost", port=8080),
                         "localhost-8080.json")
        self.assertEqual(target.record_filename(host="h:1/a", port=None), "h-1-a.json")

    def test_record_filename_separates_paths_on_one_port(self):
        self.assertNotEqual(target.record_filename(host="h", port=80, path="/api"),
                            target.record_filename(host="h", port=80, path="/admin"))

    def test_store_name_keys_on_the_same_identity(self):
        a = target.store_name(**BASE, port=8080)
        b = target.store_name(**BASE, port=8080)
        self.assertEqual(a, b)
        self.assertTrue(a.endswith(".json"))


if __name__ == "__main__":
    unittest.main()
