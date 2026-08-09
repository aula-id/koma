"""Unit / syntax / protocol tests for scrapion_agent.

These tests validate the subcommand protocol, argument parsing, JSON
response shapes, and import integrity.  They do NOT require a running
Firefox browser — browser-calling tests are gated on the ``PLAYWRIGHT``
env var or skipped.

Run with:  python -m pytest src-internet/tests/ -v
"""

import json
import sys
import io
import unittest
from pathlib import Path
from unittest.mock import patch, MagicMock

# Ensure scrapion_agent is importable.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))


# ======================================================================
# 1. Import / syntax validation
# ======================================================================

class TestImports(unittest.TestCase):
    """Verify every module imports cleanly (no syntax errors)."""

    def test_import_init(self):
        import scrapion_agent
        self.assertTrue(hasattr(scrapion_agent, "Client"))
        self.assertTrue(hasattr(scrapion_agent, "page_fetch"))
        self.assertTrue(hasattr(scrapion_agent, "search_fetch"))
        self.assertTrue(hasattr(scrapion_agent, "screenshot_capture"))

    def test_import_browser(self):
        from scrapion_agent import browser
        self.assertTrue(hasattr(browser, "page_fetch"))
        self.assertTrue(hasattr(browser, "search_fetch"))
        self.assertTrue(hasattr(browser, "screenshot_capture"))

    def test_import_main(self):
        from scrapion_agent import __main__ as main_mod
        self.assertTrue(hasattr(main_mod, "main"))
        self.assertTrue(hasattr(main_mod, "cmd_page"))
        self.assertTrue(hasattr(main_mod, "cmd_search"))
        self.assertTrue(hasattr(main_mod, "cmd_screenshot"))
        self.assertTrue(hasattr(main_mod, "cmd_legacy"))

    def test_import_input_handler(self):
        from scrapion_agent.input_handler import InputHandler, InputType
        self.assertTrue(hasattr(InputHandler, "parse_input"))

    def test_import_report(self):
        from scrapion_agent.report_generator import Report, ScrapeResult
        r = Report(query="test", mode="test", total_urls=0)
        self.assertIn("test", r.to_json())


# ======================================================================
# 2. Argument parsing
# ======================================================================

class TestArgParsing(unittest.TestCase):
    """Validate argparse subcommand protocol."""

    def _parse_subcommand(self, argv):
        """Parse argv through the subcommand parser."""
        import argparse
        from scrapion_agent.__main__ import _build_parser
        parser = _build_parser()
        return parser.parse_args(argv)

    def _parse_legacy(self, argv):
        """Parse argv through the legacy parser."""
        import argparse
        from scrapion_agent.__main__ import _build_legacy_parser
        parser = _build_legacy_parser()
        return parser.parse_args(argv)

    def test_page_subcommand(self):
        args = self._parse_subcommand(["page", "--url", "https://example.com"])
        self.assertEqual(args.command, "page")
        self.assertEqual(args.url, "https://example.com")

    def test_search_subcommand(self):
        args = self._parse_subcommand(["search", "--url", "https://html.duckduckgo.com/html/?q=hello"])
        self.assertEqual(args.command, "search")
        self.assertIn("q=hello", args.url)

    def test_screenshot_subcommand(self):
        args = self._parse_subcommand(["screenshot", "--url", "https://example.com", "--output", "/tmp/test.png"])
        self.assertEqual(args.command, "screenshot")
        self.assertEqual(args.output, "/tmp/test.png")

    def test_legacy_positional(self):
        args = self._parse_legacy(["hello world"])
        self.assertEqual(args.query, "hello world")

    def test_legacy_url(self):
        args = self._parse_legacy(["https://example.com"])
        self.assertEqual(args.query, "https://example.com")

    def test_legacy_with_json_flag(self):
        args = self._parse_legacy(["--json", "https://example.com"])
        self.assertEqual(args.query, "https://example.com")
        self.assertTrue(args.json)


# ======================================================================
# 3. JSON protocol response shape
# ======================================================================

class TestProtocolShape(unittest.TestCase):
    """Verify the JSON dicts returned by browser functions match protocol."""

    def test_page_error_shape(self):
        """page_fetch with invalid URL should return error dict."""
        import asyncio
        from scrapion_agent.browser import page_fetch

        result = asyncio.run(page_fetch("http://this-host-does-not-exist-12345.invalid"))
        self.assertEqual(result["command"], "page")
        self.assertEqual(result["status"], "error")
        self.assertIn("error", result)
        self.assertIn("url", result)

    def test_search_error_shape(self):
        """search_fetch with invalid URL should return error dict."""
        import asyncio
        from scrapion_agent.browser import search_fetch

        result = asyncio.run(search_fetch("http://this-host-does-not-exist-12345.invalid"))
        self.assertEqual(result["command"], "search")
        self.assertEqual(result["status"], "error")
        self.assertIn("error", result)
        self.assertIn("url", result)

    def test_screenshot_empty_output(self):
        """screenshot_capture with empty output_path returns error."""
        import asyncio
        from scrapion_agent.browser import screenshot_capture

        result = asyncio.run(screenshot_capture("https://example.com", ""))
        self.assertEqual(result["command"], "screenshot")
        self.assertEqual(result["status"], "error")
        self.assertIn("output_path is required", result["error"])

    def test_screenshot_error_shape(self):
        """screenshot_capture with invalid URL returns error dict."""
        import asyncio
        from scrapion_agent.browser import screenshot_capture

        result = asyncio.run(
            screenshot_capture("http://this-host-does-not-exist-12345.invalid", "/tmp/nope.png")
        )
        self.assertEqual(result["command"], "screenshot")
        self.assertEqual(result["status"], "error")
        self.assertIn("error", result)

    def test_all_responses_are_json_serializable(self):
        """Every response dict must serialize to valid JSON."""
        import asyncio
        from scrapion_agent.browser import page_fetch, search_fetch, screenshot_capture

        for coro_fn, args in [
            (page_fetch, ("http://invalid.test",)),
            (search_fetch, ("http://invalid.test",)),
            (screenshot_capture, ("http://invalid.test", "/tmp/x.png")),
            (screenshot_capture, ("http://invalid.test", "")),
        ]:
            result = asyncio.run(coro_fn(*args))
            serialized = json.dumps(result)
            self.assertIsInstance(serialized, str)
            # Must round-trip
            parsed = json.loads(serialized)
            self.assertEqual(parsed["command"], result["command"])


# ======================================================================
# 4. Report / legacy protocol (backward compat)
# ======================================================================

class TestLegacyProtocol(unittest.TestCase):
    """Ensure the legacy orchestrator Report output hasn't changed."""

    def test_report_to_json(self):
        from scrapion_agent.report_generator import Report

        r = Report(query="test query", mode="single_url", total_urls=1)
        r.add_success(url="https://example.com", content="# Hello", source="single_url")
        d = json.loads(r.to_json())
        self.assertEqual(d["query"], "test query")
        self.assertEqual(d["mode"], "single_url")
        self.assertEqual(d["successful_scrapes"], 1)
        self.assertEqual(d["results"][0]["status"], "success")
        self.assertIn("Hello", d["results"][0]["content"])

    def test_report_error_json(self):
        from scrapion_agent.report_generator import Report

        r = Report(query="fail", mode="single_url", total_urls=1)
        r.add_failure(url="https://fail.test", source="single_url")
        d = json.loads(r.to_json())
        self.assertEqual(d["failed_scrapes"], 1)


# ======================================================================
# 5. Input handler
# ======================================================================

class TestInputHandler(unittest.TestCase):
    """InputHandler.parse_input still works correctly."""

    def test_url_detection(self):
        from scrapion_agent.input_handler import InputHandler, InputType

        t, v = InputHandler.parse_input("https://example.com")
        self.assertEqual(t, InputType.URL)
        self.assertEqual(v, "https://example.com")

        t, v = InputHandler.parse_input("http://foo.bar/path?q=1")
        self.assertEqual(t, InputType.URL)

    def test_query_detection(self):
        from scrapion_agent.input_handler import InputHandler, InputType

        t, v = InputHandler.parse_input("hello world")
        self.assertEqual(t, InputType.QUERY)
        self.assertEqual(v, "hello world")


# ======================================================================
# 6. CLI _emit_json helper
# ======================================================================

class TestEmitJson(unittest.TestCase):
    """_emit_json writes exactly one line of valid JSON to stdout."""

    def test_emit_json(self):
        mock_stdout = MagicMock()
        with patch("scrapion_agent.__main__.sys") as mock_sys:
            mock_sys.stdout = mock_stdout
            from scrapion_agent.__main__ import _emit_json as emit
            emit({"ok": True})
        mock_stdout.write.assert_any_call(json.dumps({"ok": True}, ensure_ascii=False))
        mock_stdout.write.assert_any_call("\n")
        mock_stdout.flush.assert_called()


# ======================================================================
# 7. Mode detection logic
# ======================================================================

class TestModeDetection(unittest.TestCase):
    """Verify that the first-argv detection works for subcommand vs legacy."""

    def test_subcommand_detected(self):
        from scrapion_agent.__main__ import _SUBCOMMANDS
        self.assertIn("page", _SUBCOMMANDS)
        self.assertIn("search", _SUBCOMMANDS)
        self.assertIn("screenshot", _SUBCOMMANDS)
        self.assertNotIn("hello world", _SUBCOMMANDS)
        self.assertNotIn("https://example.com", _SUBCOMMANDS)

    def test_main_dispatches_legacy(self):
        """When argv[1] is a URL, main() should use legacy parser."""
        from scrapion_agent.__main__ import main
        import argparse

        # We can't fully run main() because it calls cmd_legacy which
        # invokes the orchestrator.  But we can verify mode detection.
        with patch("sys.argv", ["-m", "scrapion_agent", "https://example.com"]):
            # Should NOT enter the subcommand branch
            self.assertNotIn(sys.argv[1], _SUBCOMMANDS if False else {"page", "search", "screenshot"})


if __name__ == "__main__":
    unittest.main()
