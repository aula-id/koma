"""Readiness comes from the active guest image, never the host PATH."""
from __future__ import annotations

import pytest

from koma_sec_daemon import catalog, guest_probe


@pytest.fixture
def guest(monkeypatch, tmp_path):
    """Pretend the active guest was probed and has exactly `present`."""
    monkeypatch.setenv("KOMA_SEC_PROBE_CACHE", str(tmp_path))
    monkeypatch.setenv("KOMA_SEC_COMPOSITION", "recon")
    monkeypatch.setenv("KOMA_SEC_IMAGE", "koma-test:recon")
    guest_probe.reset()

    def use(present: set[str] | None, error: str | None = None):
        def fake(image, names):
            return guest_probe.Probe(
                image,
                present=None if present is None else frozenset(present) & names,
                error=error,
                names=names,
            )

        monkeypatch.setattr(guest_probe, "_probe_now", fake)
        guest_probe.reset()

    yield use
    guest_probe.reset()


def _recon_tool() -> tuple[str, str]:
    for tid, row in sorted(catalog._load_packs().items()):
        if row.get("image_or_pack") == "recon" and row.get("requires"):
            return tid, row["requires"][0]
    raise AssertionError("no recon tool in packs")


def test_host_path_does_not_make_a_guest_tool_ready(guest, monkeypatch):
    tid, binary = _recon_tool()
    # The host has everything; the guest has nothing.
    monkeypatch.setattr(guest_probe.shutil, "which", lambda _n: "/usr/bin/x")
    guest(set())
    tool = catalog.get_tool(tid)
    assert tool["ready"] is False
    assert tool["ready_source"] == "guest"
    assert tool["missing"] == binary
    assert catalog.gate_run(tid, "red")


def test_a_binary_in_the_guest_is_ready_and_admitted(guest, monkeypatch):
    tid, binary = _recon_tool()
    monkeypatch.setattr(guest_probe.shutil, "which", lambda _n: None)
    guest({binary})
    assert catalog.get_tool(tid)["ready"] is True
    assert catalog.gate_run(tid, "red") is None
    category = (catalog._load_packs()[tid].get("categories") or ["other"])[0]
    page = catalog.handle("list", {"category": category, "lane": "red", "limit": 200})
    assert tid in {t["name"] for t in page["tools"]}


def test_the_guest_can_run_a_tool_another_composition_lists(guest, monkeypatch):
    tid = next(
        t for t, r in sorted(catalog._load_packs().items())
        if r.get("image_or_pack") == "web" and r.get("requires") and "red" in (r.get("lanes") or [])
    )
    need = catalog._load_packs()[tid]["requires"]
    guest(set(need))
    assert catalog.gate_run(tid, "red") is None


def test_summary_reports_the_probe_and_index_carries_ready(guest):
    tid, binary = _recon_tool()
    guest({binary})
    catalog.guest(5.0)
    summary = catalog.handle("summary")
    assert summary["source"] == "guest"
    rows = {r["id"]: r for r in catalog.handle("index")["tools"]}
    assert rows[tid]["ready"] is True


def test_without_a_guest_readiness_is_an_estimate_from_the_pin(guest):
    tid, _ = _recon_tool()
    guest(None, error="docker daemon is not reachable")
    tool = catalog.get_tool(tid)
    assert tool["ready_source"] == "index"
    assert tool["ready"] is True  # recon is the active composition
    summary = catalog.handle("summary")
    assert summary["source"] == "index"
    assert summary["probe_error"] == "docker daemon is not reachable"
    assert "ready" not in catalog.handle("index")["tools"][0]


def test_a_failed_probe_is_retried_after_the_delay(guest, monkeypatch):
    calls = []

    def fake(image, names):
        calls.append(image)
        return guest_probe.Probe(image, error="down", names=names)

    monkeypatch.setattr(guest_probe, "_probe_now", fake)
    names = frozenset({"x"})
    guest_probe.current(names, wait=5.0)
    guest_probe.current(names, wait=5.0)
    assert len(calls) == 1
    monkeypatch.setattr(guest_probe, "RETRY_AFTER_S", 0.0)
    guest_probe.current(names, wait=5.0)
    assert len(calls) == 2


def test_digest_pinned_probe_is_cached_on_disk(monkeypatch, tmp_path):
    monkeypatch.setenv("KOMA_SEC_PROBE_CACHE", str(tmp_path))
    monkeypatch.setattr(guest_probe, "in_guest", lambda: False)
    runs = []

    def run(image, names):
        runs.append(image)
        return guest_probe.Probe(image, present=frozenset({"nmap"}), names=names)

    monkeypatch.setattr(guest_probe, "_run_probe", run)
    image = "ghcr.io/x/y@sha256:" + "a" * 64
    names = frozenset({"nmap", "amass"})
    assert guest_probe._probe_now(image, names).present == {"nmap"}
    assert guest_probe._probe_now(image, names).present == {"nmap"}
    assert len(runs) == 1
    # A floating tag is never cached: its content can change under the name.
    guest_probe._probe_now("ghcr.io/x/y:latest", names)
    guest_probe._probe_now("ghcr.io/x/y:latest", names)
    assert len(runs) == 3


@pytest.fixture
def two_guests(monkeypatch, tmp_path):
    """recon + web active; each guest has exactly the binaries given."""
    monkeypatch.setenv("KOMA_SEC_PROBE_CACHE", str(tmp_path))
    monkeypatch.setenv("KOMA_SEC_COMPOSITIONS", "recon,web")
    monkeypatch.setenv("KOMA_SEC_IMAGES", "img-recon,img-web")
    guest_probe.reset()

    def use(have: dict[str, set[str]]):
        def fake(image, names):
            return guest_probe.Probe(image, present=frozenset(have.get(image, set())) & names, names=names)

        monkeypatch.setattr(guest_probe, "_probe_now", fake)
        guest_probe.reset()

    yield use
    guest_probe.reset()


def _tool_of(image: str) -> tuple[str, str]:
    for tid, row in sorted(catalog._load_packs().items()):
        if row.get("image_or_pack") == image and row.get("requires") and "red" in (row.get("lanes") or []):
            return tid, row["requires"][0]
    raise AssertionError(f"no {image} tool")


def test_two_active_images_both_count_and_route_to_their_guest(two_guests):
    recon_tool, recon_bin = _tool_of("recon")
    web_tool, web_bin = _tool_of("web")
    two_guests({"img-recon": {recon_bin}, "img-web": {web_bin}})
    assert catalog.get_tool(recon_tool)["guest_image"] == "img-recon"
    assert catalog.get_tool(web_tool)["guest_image"] == "img-web"
    assert catalog.gate_run(web_tool, "red") is None
    cats = catalog.handle("categories", {"lane": "red"})
    assert cats["images"] == ["recon", "web"]
    assert cats["ready"] >= 2


def test_categories_only_count_tools_the_guests_can_run(two_guests):
    recon_tool, recon_bin = _tool_of("recon")
    two_guests({"img-recon": {recon_bin}})
    cats = catalog.handle("categories", {"lane": "red"})
    names = {c["name"] for c in cats["categories"]}
    tool_cats = set(catalog._load_packs()[recon_tool].get("categories") or [])
    assert tool_cats & names
    listed = set()
    for c in cats["categories"]:
        page = catalog.handle("list", {"category": c["name"], "lane": "red", "limit": 200})
        assert page["total"] == c["tools"]
        listed |= {t["name"] for t in page["tools"]}
    assert recon_tool in listed
    assert all(catalog.get_tool(n)["ready"] for n in listed)
    # 600+ catalog rows, but the model only meets the ready handful.
    assert len(listed) < 60


def test_a_tool_in_no_active_image_says_which_images_were_checked(two_guests):
    tid, _ = _tool_of("recon")
    two_guests({})
    tool = catalog.get_tool(tid)
    assert tool["ready"] is False
    assert "recon, web" in tool["hint"]
