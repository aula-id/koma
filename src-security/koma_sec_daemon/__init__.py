"""
koma_sec_daemon — empty liveness process for the koma agent.

No tools. Speaks newline-delimited JSON on stdin/stdout so the parent can
mark the security daemon running and arm yolo. stderr is for logs only.
"""
from __future__ import annotations

__version__ = "0.2.0"
