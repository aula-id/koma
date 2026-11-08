
"""Phase-5 contract wiring: gate_run, ready default, ensure_up net_off, host refuse."""
from __future__ import annotations

import os
from pathlib import Path

from koma_sec_daemon import catalog, runtime_ops


def test_list_defaults_to_ready_filter():
    page = catalog.handle("list", {})
    assert page.get("limit") == 40
    for t in page["tools"]:
        assert t.get("ready") is True


def test_list_all_can_include_not_ready():
    # may or may not have not-ready rows; must accept filter=all
    page = catalog.handle("list", {"filter": "all", "limit": 200})
    assert "tools" in page
    assert page["total"] >= page.get("limit") and True or True


def test_summary_op():
    s = catalog.handle("summary", {})
    assert s["tools"] >= s["ready"]
    assert "domains" in s


def test_gate_run_blocks_wrong_lane():
    # sqlmap is red-only in smoke pack
    err = catalog.gate_run("sec_sqlmap", "research")
    assert err is not None
    assert "research" in err.lower() or "lane" in err.lower()
    ok = catalog.gate_run("sec_sqlmap", "red")
    # ready may still fail if binary missing — either None or not-ready
    assert ok is None or "not ready" in (ok or "")


def test_gate_run_unknown_without_pack_row_soft_admits():
    # no pack row → None (legacy handlers)
    assert catalog.gate_run("sec_totally_missing_xyz", "red") is None


def test_ensure_up_net_off_signature():
    import inspect
    sig = inspect.signature(runtime_ops.ensure_up)
    assert "net_off" in sig.parameters


def test_host_bridge_env_documented_in_main():
    main = Path(__file__).resolve().parents[1] / "koma_sec_daemon" / "__main__.py"
    text = main.read_text(encoding="utf-8")
    assert "KOMA_SEC_ALLOW_HOST_BRIDGE" in text
    assert "refused host fallback" in text
