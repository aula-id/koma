"""Low-level async browser operations for page, search, and screenshot.

All functions are async and return dicts suitable for JSON serialisation.
They are consumed by ``__main__.py`` subcommands and kept separate from
the orchestrator so the legacy one-shot flow is untouched.
"""

import asyncio
from pathlib import Path


async def _launch_browser(playwright):
    """Launch a headless Firefox with standard stealth args."""
    return await playwright.firefox.launch(
        headless=True,
        args=[
            "--disable-blink-features=AutomationControlled",
            "--disable-dev-shm-usage",
            "--no-sandbox",
            "--start-maximized",
        ],
    )


async def _new_page(browser):
    """Create a new page with anti-detection init script."""
    page = await browser.new_page(no_viewport=True)
    await page.add_init_script(
        "Object.defineProperty(navigator, 'webdriver', {get: () => undefined})"
    )
    return page


# ---------------------------------------------------------------------------
# page — fetch a URL and return its markdown content
# ---------------------------------------------------------------------------

async def page_fetch(url: str) -> dict:
    """Navigate to *url*, extract main content as markdown, return JSON-safe dict.

    Returns ``{"command": "page", "status": "success", "url": ...,
    "content": ..., "title": ...}`` on success.
    """
    from playwright.async_api import async_playwright
    from markdownify import markdownify as md

    try:
        async with async_playwright() as p:
            browser = await _launch_browser(p)
            try:
                page = await _new_page(browser)
                await page.goto(url, timeout=90_000, wait_until="networkidle")
                title = await page.title()
                html = await page.content()
                content = md(html, heading_style="ATX")
                return {
                    "command": "page",
                    "status": "success",
                    "url": url,
                    "title": title or "",
                    "content": content,
                }
            finally:
                await browser.close()
    except Exception as exc:
        return {
            "command": "page",
            "status": "error",
            "url": url,
            "error": str(exc),
        }


# ---------------------------------------------------------------------------
# search — navigate to a pre-built search URL and extract results
# ---------------------------------------------------------------------------

async def search_fetch(search_url: str) -> dict:
    """Navigate to a pre-built search URL, extract result titles/links/snippets.

    Returns ``{"command": "search", "status": "success", "url": ...,
    "results": [...]}`` on success.

    This is intentionally simple: it navigates, waits for network idle,
    then tries to extract common result patterns.  It does NOT type into
    search boxes — the caller supplies a fully-formed URL.
    """
    from playwright.async_api import async_playwright

    try:
        async with async_playwright() as p:
            browser = await _launch_browser(p)
            try:
                page = await _new_page(browser)
                await page.goto(search_url, timeout=90_000, wait_until="networkidle")
                await asyncio.sleep(0.3)
                title = await page.title()
                html = await page.content()

                # Best-effort extraction of result blocks.  DDG HTML endpoint
                # uses `.result__body` containers; fall back to generic patterns.
                results = []

                # Try DDG-specific selectors first
                containers = await page.query_selector_all(".result__body")
                if not containers:
                    containers = await page.query_selector_all(".result")
                if not containers:
                    # Broader fallback: grab all links with meaningful text
                    containers = await page.query_selector_all("a[href]")

                seen_urls: set[str] = set()
                for el in containers:
                    try:
                        if await el.get_attribute("href") is not None:
                            # It's a bare <a> tag
                            link = await el.get_attribute("href") or ""
                            text = (await el.text_content() or "").strip()
                            if text and link and link not in seen_urls:
                                seen_urls.add(link)
                                results.append({
                                    "title": text,
                                    "link": link,
                                    "snippet": "",
                                })
                            continue

                        # It's a container — try common sub-selectors
                        title_el = await el.query_selector(
                            "h2 a, h3 a, a.result__a, a"
                        )
                        snippet_el = await el.query_selector(
                            ".result__snippet, .snippet, p"
                        )

                        r_title = ""
                        r_link = ""
                        r_snippet = ""

                        if title_el:
                            r_title = (await title_el.text_content() or "").strip()
                            r_link = await title_el.get_attribute("href") or ""
                        if snippet_el:
                            r_snippet = (await snippet_el.text_content() or "").strip()

                        if r_title and r_link and r_link not in seen_urls:
                            seen_urls.add(r_link)
                            results.append({
                                "title": r_title,
                                "link": r_link,
                                "snippet": r_snippet,
                            })
                    except Exception:
                        continue

                return {
                    "command": "search",
                    "status": "success",
                    "url": search_url,
                    "title": title or "",
                    "results": results,
                }
            finally:
                await browser.close()
    except Exception as exc:
        return {
            "command": "search",
            "status": "error",
            "url": search_url,
            "error": str(exc),
        }


# ---------------------------------------------------------------------------
# screenshot — capture a PNG screenshot to an explicit output path
# ---------------------------------------------------------------------------

async def screenshot_capture(
    url: str,
    output_path: str,
    width: int = 0,
    height: int = 0,
    full_page: bool = False,
) -> dict:
    """Navigate to *url* and save a PNG screenshot to *output_path*.

    When *width* and *height* are > 0 the page is created with that viewport;
    otherwise the default no-viewport mode is used (full-page capture).
    *full_page* controls whether Playwright captures the full scrollable page
    or just the visible viewport. When true, the page is auto-scrolled top to
    bottom first to trigger IntersectionObserver animations and lazy loads.
    """
    from playwright.async_api import async_playwright

    if not output_path:
        return {
            "command": "screenshot",
            "status": "error",
            "url": url,
            "error": "output_path is required for screenshot command",
        }

    # Ensure parent directory exists.
    out = Path(output_path)
    out.parent.mkdir(parents=True, exist_ok=True)

    try:
        async with async_playwright() as p:
            browser = await _launch_browser(p)
            try:
                # Create page with optional explicit viewport.
                if width > 0 and height > 0:
                    page = await browser.new_page(
                        viewport={"width": width, "height": height}
                    )
                    await page.add_init_script(
                        "Object.defineProperty(navigator, 'webdriver', {get: () => undefined})"
                    )
                else:
                    page = await _new_page(browser)

                await page.goto(url, timeout=90_000, wait_until="networkidle")
                await asyncio.sleep(0.3)

                # For full_page: auto-scroll to trigger animations before capture.
                if full_page:
                    vp_height = (page.viewport_size or {}).get("height", 1080)
                    await _prepare_full_page(page, vp_height)

                # Capture viewport size for the response metadata.
                viewport = page.viewport_size
                vp_width = viewport["width"] if viewport else 0
                vp_height = viewport["height"] if viewport else 0

                await page.screenshot(path=str(out), full_page=full_page)
                return {
                    "command": "screenshot",
                    "status": "success",
                    "url": url,
                    "output_path": str(out.resolve()),
                    "width": vp_width,
                    "height": vp_height,
                }
            finally:
                await browser.close()
    except Exception as exc:
        return {
            "command": "screenshot",
            "status": "error",
            "url": url,
            "error": str(exc),
        }


async def _prepare_full_page(page, viewport_height: int) -> None:
    """Scroll through the page to trigger animations, then return to top.

    Handles IntersectionObserver fade-in, lazy-loaded content, and infinite
    scroll (stops after height stabilises 3 times or 50 iterations).
    """
    scroll_step = max(viewport_height - 100, 400)
    stable_count = 0
    prev_height = 0
    max_iterations = 50

    await page.keyboard.press("Home")
    await asyncio.sleep(0.3)

    for _ in range(max_iterations):
        await page.mouse.wheel(0, scroll_step)
        await asyncio.sleep(0.4)

        height_after = await page.evaluate("document.documentElement.scrollHeight")
        if height_after == prev_height:
            stable_count += 1
            if stable_count >= 3:
                break
        else:
            stable_count = 0
        prev_height = height_after

    await page.keyboard.press("Home")
    await asyncio.sleep(0.5)

    # Force all elements visible — full_page resizes the viewport which
    # can re-trigger IntersectionObserver and reset opacity/transform.
    await page.evaluate("""() => {
        const style = document.createElement('style');
        style.id = '__koma_fullpage_fix';
        style.textContent = `
            *, *::before, *::after {
                animation-duration: 0s !important;
                animation-delay: 0s !important;
                transition-duration: 0s !important;
                transition-delay: 0s !important;
            }
        `;
        document.head.appendChild(style);
        document.querySelectorAll('[style*="opacity"], [style*="transform"]').forEach(el => {
            el.style.opacity = '1';
            el.style.transform = 'none';
        });
    }""")
    await asyncio.sleep(0.2)
