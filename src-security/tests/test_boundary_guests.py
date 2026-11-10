"""One boundary, one guest per active image; nothing outlives its daemon."""
from __future__ import annotations

from pathlib import Path

from koma_sec_daemon import boundary


def _boundary(tmp_path: Path, calls: list):
    b = boundary.new_boundary("k1", "img-a", tmp_path)
    b.network, b.proxy, b.guest, b.dns_addr = "koma-net-k1", "koma-proxy-k1", "koma-guest-k1", "10.0.0.2"

    class Done:
        returncode, stdout, stderr = 0, "", ""

    b._docker = lambda *args, check=True: calls.append(args) or Done()  # type: ignore[method-assign]
    b.guests = {"img-a": b.guest}
    return b


def test_a_second_image_gets_its_own_guest_once(tmp_path):
    calls: list = []
    b = _boundary(tmp_path, calls)
    assert b.guest_for(None) == "koma-guest-k1"
    assert b.guest_for("img-a") == "koma-guest-k1"
    second = b.guest_for("img-b")
    assert second == "koma-guest-k1-1"
    assert b.guest_for("img-b") == second
    runs = [c for c in calls if c[0] == "run"]
    assert len(runs) == 1
    run = runs[0]
    # Same isolation as the first guest: internal network, proxy DNS, labels.
    assert run[run.index("--network") + 1] == "koma-net-k1"
    assert run[run.index("--dns") + 1] == "10.0.0.2"
    assert "--read-only" in run and "--cap-drop" in run
    assert f"{boundary.OWNER_LABEL}=" in " ".join(run)
    assert run[-1] == "img-b"


def test_down_removes_every_guest(tmp_path):
    calls: list = []
    b = _boundary(tmp_path, calls)
    b.guest_for("img-b")
    calls.clear()
    b.down()
    removed = [c[-1] for c in calls if c[:2] == ("rm", "-f")]
    assert set(removed) == {"koma-guest-k1", "koma-guest-k1-1", "koma-proxy-k1"}
    assert b.guests == {}


def test_reap_leaves_live_owners_and_removes_dead_ones(monkeypatch):
    ran: list = []

    class Out:
        def __init__(self, stdout=""):
            self.returncode, self.stdout, self.stderr = 0, stdout, ""

    def fake_run(argv, **_kw):
        ran.append(argv)
        if argv[:3] == ["docker", "ps", "-a"]:
            return Out("c-live 111\nc-dead 222\n")
        if argv[:3] == ["docker", "network", "ls"]:
            return Out("koma-net-dead 222\n")
        return Out()

    monkeypatch.setattr(boundary, "_run", fake_run)
    monkeypatch.setattr(boundary.shutil, "which", lambda _n: "/usr/bin/docker")
    monkeypatch.setattr(boundary, "_alive", lambda pid: pid == 111)
    assert boundary.reap_orphans() == 1
    assert ["docker", "rm", "-f", "c-dead"] in ran
    assert ["docker", "network", "rm", "koma-net-dead"] in ran
    assert not any("c-live" in a for a in ran if a[:2] == ["docker", "rm"])
