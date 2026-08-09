"""
Headless one-shot entrypoint for scrapion_agent.

Usage:
    python -m scrapion_agent "<query or URL>"          # legacy one-shot
    python -m scrapion_agent page --url <URL>           # browser page fetch
    python -m scrapion_agent search --url <SEARCH_URL>  # browser search
    python -m scrapion_agent screenshot --url <URL> --output <PATH>

stdout is always valid JSON (report on success, error object on failure).
All library debug output is redirected to stderr.
"""

import sys
import os
import json
import asyncio
import argparse

# Skip the Firefox auto-install check; koma --install-internet handles that separately.
os.environ["SCRAPION_SKIP_BROWSER_CHECK"] = "1"

# Known subcommand names — used for first-argv detection.
_SUBCOMMANDS = frozenset({"page", "search", "screenshot", "daemon"})


def _emit_json(real_stdout, payload):
    """Write a JSON payload to the real stdout."""
    real_stdout.write(json.dumps(payload, ensure_ascii=False))
    real_stdout.flush()


def _build_parser():
    """Build the subcommand-based argparse parser."""
    parser = argparse.ArgumentParser(
        description="Headless one-shot web search and scrape (subcommand mode)",
        prog="python -m scrapion_agent",
    )
    sub = parser.add_subparsers(dest="command")

    p_page = sub.add_parser("page", help="Fetch a URL and return markdown content")
    p_page.add_argument("--url", required=True, help="URL to fetch")

    p_search = sub.add_parser("search", help="Navigate to a search URL and extract results")
    p_search.add_argument("--url", required=True, help="Fully-formed search URL")

    p_ss = sub.add_parser("screenshot", help="Capture a PNG screenshot")
    p_ss.add_argument("--url", required=True, help="URL to screenshot")
    p_ss.add_argument("--output", required=True, help="Output PNG path")
    p_ss.add_argument("--width", type=int, default=0, help="Viewport width in pixels (0 = default)")
    p_ss.add_argument("--height", type=int, default=0, help="Viewport height in pixels (0 = default)")
    p_ss.add_argument("--full-page", action=argparse.BooleanOptionalAction, default=True,
                      help="Capture full scrollable page (default: True; use --no-full-page for viewport only)")

    return parser


def _build_legacy_parser():
    """Build the legacy one-shot parser (positional query/URL)."""
    parser = argparse.ArgumentParser(
        description="Headless one-shot web search and scrape (legacy mode)",
        prog="python -m scrapion_agent",
    )
    parser.add_argument("query", help="Search query or URL to scrape")
    parser.add_argument(
        "--json",
        action="store_true",
        default=True,
        help=argparse.SUPPRESS,
    )
    return parser


def cmd_page(args):
    """Handle `page --url <URL>` subcommand via browser.py."""
    from scrapion_agent.browser import page_fetch

    real_stdout = sys.stdout
    sys.stdout = sys.stderr
    try:
        result = asyncio.run(page_fetch(args.url))
    except Exception as exc:
        result = {"command": "page", "status": "error", "url": args.url, "error": str(exc)}
    finally:
        sys.stdout = real_stdout

    _emit_json(real_stdout, result)


def cmd_search(args):
    """Handle `search --url <SEARCH_URL>` subcommand via browser.py."""
    from scrapion_agent.browser import search_fetch

    real_stdout = sys.stdout
    sys.stdout = sys.stderr
    try:
        result = asyncio.run(search_fetch(args.url))
    except Exception as exc:
        result = {"command": "search", "status": "error", "url": args.url, "error": str(exc)}
    finally:
        sys.stdout = real_stdout

    _emit_json(real_stdout, result)


def cmd_screenshot(args):
    """Handle `screenshot --url <URL> --output <PATH>` subcommand via browser.py."""
    from scrapion_agent.browser import screenshot_capture

    real_stdout = sys.stdout
    sys.stdout = sys.stderr
    try:
        result = asyncio.run(screenshot_capture(
            args.url,
            args.output,
            width=args.width,
            height=args.height,
            full_page=args.full_page,
        ))
    except Exception as exc:
        result = {
            "command": "screenshot",
            "status": "error",
            "url": args.url,
            "error": str(exc),
        }
    finally:
        sys.stdout = real_stdout

    _emit_json(real_stdout, result)


def cmd_legacy(args):
    """Handle legacy one-shot mode (positional query/URL)."""
    query = args.query

    real_stdout = sys.stdout
    sys.stdout = sys.stderr

    report = None
    try:
        from scrapion_agent import Client
        report = Client(skip_browser_check=True).run(query)
    except Exception as exc:
        sys.stdout = real_stdout
        _emit_json(real_stdout, {"error": str(exc), "query": query})
        sys.exit(1)
    finally:
        sys.stdout = real_stdout

    _emit_json(real_stdout, report.to_json() if hasattr(report, "to_json") else report)


def main():
    if len(sys.argv) >= 2 and sys.argv[1] in _SUBCOMMANDS:
        # daemon is a special subcommand — delegate directly
        if sys.argv[1] == "daemon":
            from scrapion_agent.daemon import main as daemon_main
            sys.argv = [sys.argv[0]] + sys.argv[2:]  # strip "daemon"
            daemon_main()
            return

        parser = _build_parser()
        args = parser.parse_args()
        if args.command == "page":
            cmd_page(args)
        elif args.command == "search":
            cmd_search(args)
        elif args.command == "screenshot":
            cmd_screenshot(args)
    else:
        parser = _build_legacy_parser()
        args = parser.parse_args()
        cmd_legacy(args)


if __name__ == "__main__":
    main()
