"""Gate 7: the four debugger tools — xss, recording, replay, decrypt.

`sec_xss` and `sec_replay` run against the real fixture server (no mocks).
`sec_decrypt` is pure and runs against fixed vectors. `sec_recording` is file work.

The sharp case is `test_an_escaping_fix_is_not_fooled`: a target that ENCODES the
payload is still reflected, and a version-only detector would call that a fix.
"""
import base64
import binascii
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from koma_sec_daemon import verdict  # noqa: E402
from koma_sec_daemon.tools import sec_decrypt, sec_recording, sec_replay, sec_xss  # noqa: E402
from test_target import Target  # noqa: E402

try:
    import requests  # noqa: F401

    HAVE = True
except ImportError:  # pragma: no cover
    HAVE = False


def run(mod, args: dict) -> dict:
    return json.loads(mod._handler(args, None))


class Sandbox(unittest.TestCase):
    """Point the recordings dir at a temp dir for every test."""

    def setUp(self) -> None:
        self._old = os.environ.get("KOMA_BOARD")
        self._dir = tempfile.mkdtemp()
        os.environ["KOMA_BOARD"] = self._dir

    def tearDown(self) -> None:
        if self._old is None:
            os.environ.pop("KOMA_BOARD", None)
        else:
            os.environ["KOMA_BOARD"] = self._old


class DecryptTest(unittest.TestCase):
    def test_a_cbc_blob_with_a_hex_key_and_iv_is_confirmed(self) -> None:
        from cryptography.hazmat.primitives import padding  # noqa: PLC0415
        from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes  # noqa: PLC0415

        key, iv = bytes(range(16)), bytes(reversed(range(16)))
        pad = padding.PKCS7(128).padder()
        body = pad.update(b"secret value") + pad.finalize()
        dec = Cipher(algorithms.AES(key), modes.CBC(iv)).encryptor()
        blob = dec.update(body) + dec.finalize()

        got = run(
            sec_decrypt,
            {
                "data": binascii.hexlify(blob).decode(),
                "data_encoding": "hex",
                "scheme": "aes-cbc",
                "key": binascii.hexlify(key).decode(),
                "key_encoding": "hex",
                "iv": binascii.hexlify(iv).decode(),
            },
        )
        self.assertEqual(got["verdict"], verdict.CONFIRMED)
        self.assertIn("secret value", got["excerpt"])

    def test_a_wrong_key_reports_a_borderline_padding_not_a_success(self) -> None:
        # The dangerous case: CBC "succeeds" with a wrong key and yields garbage.
        from cryptography.hazmat.primitives import padding  # noqa: PLC0415
        from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes  # noqa: PLC0415

        iv = b"0" * 16
        pad = padding.PKCS7(128).padder()
        body = pad.update(b"hello world") + pad.finalize()
        dec = Cipher(algorithms.AES(b"a" * 16), modes.CBC(iv)).encryptor()
        blob = dec.update(body) + dec.finalize()

        got = run(
            sec_decrypt,
            {
                "data": base64.b64encode(blob).decode(),
                "scheme": "aes-cbc",
                "key": base64.b64encode(b"b" * 16).decode(),
                "key_encoding": "base64",
                "iv": binascii.hexlify(iv).decode(),
            },
        )
        self.assertIn(got["verdict"], (verdict.UNPROVEN, verdict.ERROR))

    def test_xor_round_trips(self) -> None:
        plain = b"flag{local}"
        blob = bytes(b ^ ord("k") for b in plain)
        got = run(
            sec_decrypt,
            {
                "data": base64.b64encode(blob).decode(),
                "scheme": "xor",
                "key": "k",
                "key_encoding": "raw",
            },
        )
        self.assertEqual(got["verdict"], verdict.CONFIRMED)
        self.assertIn("flag{local}", got["excerpt"])

    def test_bad_base64_is_a_clean_error(self) -> None:
        got = run(sec_decrypt, {"data": "!!!!", "scheme": "xor", "key": "k"})
        self.assertEqual(got["verdict"], verdict.ERROR)
        self.assertIn("base64", got["excerpt"])

    def test_an_unknown_scheme_is_refused(self) -> None:
        got = run(sec_decrypt, {"data": "AAAA", "scheme": "rot13", "key": "k"})
        self.assertEqual(got["verdict"], verdict.ERROR)

    def test_the_key_interpretation_is_reported(self) -> None:
        # A silent guess turns a wrong key into a confusing padding error.
        got = run(
            sec_decrypt,
            {"data": "", "scheme": "xor", "key": "k", "key_encoding": "raw"},
        )
        self.assertIn("empty", got["excerpt"])


class RecordingTest(Sandbox):
    def test_a_recording_round_trips(self) -> None:
        saved = run(
            sec_recording,
            {"op": "save", "name": "r1", "requests": [{"url": "http://a.example/x"}]},
        )
        self.assertEqual(saved["verdict"], verdict.CONFIRMED)
        loaded = run(sec_recording, {"op": "load", "name": "r1"})
        self.assertIn("http://a.example/x", loaded["excerpt"])

    def test_a_raw_request_is_normalised(self) -> None:
        got = run(
            sec_recording,
            {
                "op": "save",
                "name": "raw1",
                "requests": ["POST http://a.example/y HTTP/1.1\nX-A: 1\n\nbody"],
            },
        )
        self.assertEqual(got["verdict"], verdict.CONFIRMED)
        loaded = run(sec_recording, {"op": "load", "name": "raw1"})
        self.assertIn("POST http://a.example/y", loaded["excerpt"])

    def test_a_traversal_name_is_refused(self) -> None:
        got = run(sec_recording, {"op": "save", "name": "../escape", "requests": [{"url": "http://a/"}]})
        self.assertEqual(got["verdict"], verdict.ERROR)

    def test_a_missing_recording_is_not_found(self) -> None:
        self.assertEqual(run(sec_recording, {"op": "load", "name": "nope"})["verdict"], verdict.NOT_FOUND)

    def test_one_bad_entry_does_not_lose_the_good_ones(self) -> None:
        got = run(
            sec_recording,
            {"op": "save", "name": "mix", "requests": [{"url": "http://a/ok"}, {"nope": 1}]},
        )
        self.assertEqual(got["verdict"], verdict.UNPROVEN)
        self.assertIn("1 rejected", got["excerpt"])


@unittest.skipUnless(HAVE, "requests not installed")
class XssTest(unittest.TestCase):
    def setUp(self) -> None:
        self.t = Target().__enter__()

    def tearDown(self) -> None:
        self.t.__exit__()

    def test_a_reflecting_parameter_is_confirmed(self) -> None:
        got = run(sec_xss, {"url": self.t.base + "/reflect", "param": "q"})
        self.assertEqual(got["verdict"], verdict.CONFIRMED)
        self.assertIn("VERBATIM", got["excerpt"])

    def test_an_escaping_target_is_not_confirmed(self) -> None:
        # THE case a version-only detector gets wrong: the marker still comes back,
        # so "is the marker present" passes, but the payload does not execute.
        got = run(sec_xss, {"url": self.t.base + "/reflect-enc", "param": "q"})
        self.assertEqual(got["verdict"], verdict.NOT_FOUND)
        self.assertIn("encoded", got["excerpt"])

    def test_a_target_that_ignores_the_parameter_is_not_confirmed(self) -> None:
        got = run(sec_xss, {"url": self.t.base + "/landed", "param": "q"})
        self.assertEqual(got["verdict"], verdict.NOT_FOUND)

    def test_the_marker_is_unique_per_call(self) -> None:
        # A fixed marker would match a stale cached body.
        a = run(sec_xss, {"url": self.t.base + "/reflect", "param": "q"})
        b = run(sec_xss, {"url": self.t.base + "/reflect", "param": "q"})
        self.assertEqual(a["verdict"], b["verdict"])


@unittest.skipUnless(HAVE, "requests not installed")
class ReplayTest(Sandbox):
    def setUp(self) -> None:
        super().setUp()
        self.t = Target().__enter__()

    def tearDown(self) -> None:
        self.t.__exit__()
        super().tearDown()

    def test_a_recording_replays_against_the_target(self) -> None:
        run(sec_recording, {"op": "save", "name": "rp", "requests": [{"url": self.t.base + "/"}]})
        got = run(sec_replay, {"name": "rp"})
        self.assertEqual(got["verdict"], verdict.CONFIRMED)
        self.assertIn("root", got["excerpt"])

    def test_repointing_replaces_the_origin_and_keeps_the_path(self) -> None:
        run(sec_recording, {"op": "save", "name": "rp2", "requests": [{"url": "http://old.example/landed"}]})
        got = run(sec_replay, {"name": "rp2", "target": self.t.base})
        self.assertIn("landed", got["excerpt"])

    def test_a_5xx_makes_the_run_unproven(self) -> None:
        run(sec_recording, {"op": "save", "name": "rp3", "requests": [{"url": self.t.base + "/boom"}]})
        self.assertEqual(run(sec_replay, {"name": "rp3"})["verdict"], verdict.UNPROVEN)

    def test_a_missing_recording_is_not_found(self) -> None:
        self.assertEqual(run(sec_replay, {"name": "none"})["verdict"], verdict.NOT_FOUND)


if __name__ == "__main__":
    unittest.main()
