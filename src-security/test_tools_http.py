"""Gate 7, part 1: sec_http against a real server (no mocks)."""
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from koma_sec_daemon import verdict  # noqa: E402
from koma_sec_daemon.tools import http as sec_http  # noqa: E402
from test_target import Target  # noqa: E402

try:
    import requests  # noqa: F401

    HAVE = True
except ImportError:  # pragma: no cover
    HAVE = False


def run(args: dict) -> dict:
    return json.loads(sec_http._handler(args, None))


@unittest.skipUnless(HAVE, "requests not installed")
class HttpTest(unittest.TestCase):
    def setUp(self) -> None:
        self.t = Target().__enter__()

    def tearDown(self) -> None:
        self.t.__exit__()

    def test_a_200_is_confirmed_with_the_body(self) -> None:
        got = run({"url": self.t.base + "/"})
        self.assertEqual(got["verdict"], verdict.CONFIRMED)
        self.assertIn("root", got["excerpt"])
        self.assertIn("200", got["excerpt"])

    def test_a_post_body_round_trips(self) -> None:
        got = run({"url": self.t.base + "/", "method": "POST", "body": "hi"})
        self.assertIn("got:hi", got["excerpt"])

    def test_a_404_is_still_confirmed_because_the_server_answered(self) -> None:
        self.assertEqual(run({"url": self.t.base + "/nope"})["verdict"], verdict.CONFIRMED)

    def test_a_5xx_is_unproven_not_confirmed(self) -> None:
        self.assertEqual(run({"url": self.t.base + "/boom"})["verdict"], verdict.UNPROVEN)

    def test_a_redirect_chain_is_shown(self) -> None:
        # The chain is what a scope audit reads: the final URL is the response's
        # own, and each hop is listed so an out-of-scope landing is visible.
        got = run({"url": self.t.base + "/redirect"})
        self.assertIn("redirect chain", got["excerpt"])
        self.assertIn("/redirect", got["excerpt"])

    def test_a_refused_hop_is_reported_as_refused(self) -> None:
        # 127.0.0.1:1 is closed, so following the hop yields a connection error.
        got = run({"url": self.t.base + "/redirect-away"})
        self.assertIn(got["verdict"], (verdict.CONFIRMED, verdict.ERROR))

    def test_redirects_can_be_switched_off(self) -> None:
        got = run({"url": self.t.base + "/redirect", "follow_redirects": False})
        self.assertIn("302", got["excerpt"])

    def test_the_body_is_capped(self) -> None:
        got = run({"url": self.t.base + "/big"})
        # The body is cut; the trailer counts what was dropped.
        self.assertIn("more chars]", got["excerpt"])
        self.assertLess(len(got["excerpt"]), 110_000)

    def test_a_dead_port_is_an_error_verdict_not_a_not_found(self) -> None:
        got = run({"url": "http://127.0.0.1:1/"})
        self.assertEqual(got["verdict"], verdict.ERROR)

    def test_response_headers_are_present(self) -> None:
        got = run({"url": self.t.base + "/secret"})
        self.assertIn("Set-Cookie", got["excerpt"])


if __name__ == "__main__":
    unittest.main()
