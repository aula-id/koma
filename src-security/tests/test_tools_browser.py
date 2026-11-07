"""Gate 7, part 2: sec_browser against a real browser and a real server."""
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from koma_sec_daemon import verdict  # noqa: E402
from koma_sec_daemon.tools import sec_browser  # noqa: E402
from test_target import Target  # noqa: E402

try:
    import playwright  # noqa: F401

    HAVE = True
except ImportError:  # pragma: no cover
    HAVE = False


def run(args: dict) -> dict:
    return json.loads(sec_browser._handler(args, None))


@unittest.skipUnless(HAVE, "playwright not installed")
class BrowserTest(unittest.TestCase):
    def setUp(self) -> None:
        self.t = Target().__enter__()

    def tearDown(self) -> None:
        self.t.__exit__()

    def test_a_page_loads_and_is_confirmed(self) -> None:
        got = run({"url": self.t.base + "/"})
        self.assertEqual(got["verdict"], verdict.CONFIRMED)
        self.assertIn("root", got["excerpt"])

    def test_the_final_url_is_reported(self) -> None:
        got = run({"url": self.t.base + "/"})
        self.assertIn("final:", got["excerpt"])

    def test_a_redirect_chain_is_followed_and_the_final_url_differs(self) -> None:
        got = run({"url": self.t.base + "/redirect"})
        self.assertIn("/landed", got["excerpt"])

    def test_links_are_collected(self) -> None:
        got = run({"url": self.t.base + "/"})
        self.assertIn("links:", got["excerpt"])

    def test_a_screenshot_is_written(self) -> None:
        d = Path(tempfile.mkdtemp())
        try:
            path = d / "shot.png"
            got = run({"url": self.t.base + "/", "screenshot": str(path)})
            self.assertEqual(got["verdict"], verdict.CONFIRMED)
            self.assertTrue(path.exists())
            self.assertGreater(path.stat().st_size, 1000)
        finally:
            shutil.rmtree(d, ignore_errors=True)

    def test_a_dead_port_is_an_error_verdict(self) -> None:
        got = run({"url": "http://127.0.0.1:1/"})
        self.assertEqual(got["verdict"], verdict.ERROR)
        self.assertIn("navigate", got["excerpt"])

    def test_the_body_is_capped(self) -> None:
        got = run({"url": self.t.base + "/big"})
        self.assertLess(len(got["excerpt"]), 70_000)


class BrowserDescriptorTest(unittest.TestCase):
    def test_it_is_classified_browser_exec(self) -> None:
        # Runs on the HOST, not in the guest: the exec class has to say so or the
        # distinction is lost.
        self.assertEqual(sec_browser.DESCRIPTOR["exec"], "browser")

    def test_it_is_a_network_capability_for_lane_purposes(self) -> None:
        from koma_sec_daemon import effects

        self.assertEqual(effects.MANIFEST["sec_browser"][1], effects.BROWSER)


if __name__ == "__main__":
    unittest.main()
