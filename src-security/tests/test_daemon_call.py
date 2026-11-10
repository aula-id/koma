"""A `call` frame reaches the guest dispatch instead of dying in the daemon."""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path


def test_a_call_is_dispatched_not_a_name_error(tmp_path):
    env = dict(
        os.environ,
        KOMA_SEC_PROBE_CACHE=str(tmp_path / "probe"),
        KOMA_SEC_RUN_DIR=str(tmp_path / "run"),
        # No Docker reachable: the call must come back as a typed boundary
        # error, the same path a working runtime takes up to `docker run`.
        PATH=str(tmp_path),
    )
    p = subprocess.Popen(
        [sys.executable, "-m", "koma_sec_daemon"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        text=True, cwd=str(Path(__file__).resolve().parent.parent), env=env,
    )
    try:
        p.stdin.write(json.dumps({"v": 2}) + "\n")
        p.stdin.flush()
        assert json.loads(p.stdout.readline())["ok"] is True
        p.stdin.write(json.dumps({
            "id": 1, "op": "call", "tool": "sec_decode",
            "args": {"data": "aGVsbG8=", "mode": "auto"}, "lane": "research",
        }) + "\n")
        p.stdin.flush()
        while True:
            reply = json.loads(p.stdout.readline())
            if reply.get("id") == 1 and "progress" not in reply:
                break
        text = json.dumps(reply)
        assert "is not defined" not in text, text
        assert "referenced before assignment" not in text, text
        assert reply["ok"] is True, text
        assert "isolation boundary unavailable" in reply["result"], text
    finally:
        p.stdin.close()
        p.wait(timeout=10)
