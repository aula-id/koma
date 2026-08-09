"""
Scrapion - Web Scraping Automation System

A Python library for automated web scraping with intelligent fallback mechanisms.
"""

from .input_handler import InputHandler, InputType
from .list_manager import UrlListManager, UrlSource
from .report_generator import Report, ScrapeResult
from .orchestrator import Client
from .browser import page_fetch, search_fetch, screenshot_capture

# Backward compatibility alias
Orchestrator = Client

__version__ = "0.1.0"
__all__ = [
    "Client",
    "Orchestrator",  # Backward compatibility
    "InputHandler",
    "InputType",
    "UrlListManager",
    "UrlSource",
    "Report",
    "ScrapeResult",
    "page_fetch",
    "search_fetch",
    "screenshot_capture",
]
