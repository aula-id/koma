"""
koma_sec_daemon — long-lived security daemon for the koma TUI agent.

Communicates with its parent process (koma) over newline-delimited JSON on
stdin/stdout. stderr is used for logging and debug output.

Tools shipped in this milestone:
  sec_http    — stateless single HTTP request (proof of stateless tool)
  sec_remote  — stateful pwntools remote session (proof of stateful session)
"""

__version__ = "0.1.0"
