"""Gate 3: the boundary, proved against a live daemon.

These need Docker and a built image (`src-security/image`). They are skipped, with
a stated reason, when either is missing — a skip here is a real "unverified", so
the reason is what matters, not the green.

Each test names the bypass it proves. If one starts passing for the wrong reason
(e.g. the guest still had a direct route) another test in the same class covers
the difference, so no single test carries the whole claim.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from koma_sec_daemon import boundary  # noqa: E402

IMAGE = os.environ.get("KOMA_SEC_IMAGE", "koma-safety:dev")
# A name that is definitely not in the rules, resolvable only through the proxy's
# forwarder. `example.com` is in scope below; wikipedia is not.
OUT_OF_SCOPE = "wikipedia.org"
OUT_OF_SCOPE_URL = f"http://{OUT_OF_SCOPE}/"


def _image_present() -> bool:
    res = boundary._run(["docker", "image", "inspect", IMAGE])
    return res.returncode == 0


def setUpModule() -> None:
    reason = boundary.preflight()
    if reason:
        raise unittest.SkipTest(reason)
    if not _image_present():
        raise unittest.SkipTest(f"image {IMAGE} not built (docker build src-security/image)")


class BoundaryCase(unittest.TestCase):
    """One boundary per test, torn down after. Shared key space is avoided."""

    key = ""

    def setUp(self) -> None:
        self.dir = Path.home() / f"koma-bnd-test/{self.id().rsplit('.', 1)[-1]}"
        self.b = boundary.new_boundary("t" + os.urandom(3).hex(), IMAGE, self.dir)
        boundary.write_rules(
            self.b.rules_path,
            boundary.rules_for(["example.com:80", "example.com:443"], []),
        )
        self.b.up()

    def tearDown(self) -> None:
        self.b.down()
        shutil.rmtree(self.dir, ignore_errors=True)

    def sh(self, cmd: str, timeout: float = 30) -> tuple[int, str]:
        res = self.b.exec(["sh", "-c", cmd], timeout)
        return res.returncode, (res.stdout + res.stderr).strip()


class IsolationTest(BoundaryCase):
    def test_the_guest_has_no_gateway(self) -> None:
        # The root fact: an --internal network has no default route, so even a
        # correct address is unreachable directly.
        _, out = self.sh("curl -sS -m 8 --noproxy '*' --resolve example.com:80:104.20.23.154 -o /dev/null -w '%{http_code}' http://example.com")
        self.assertIn("Couldn't connect", out, "the guest reached a host directly")

    def test_no_names_resolve_without_the_forwarder(self) -> None:
        # With the resolver pointed away there is no lookup at all, which is why
        # the forwarder exists rather than being a convenience.
        _, out = self.sh("env -u HTTP_PROXY curl -sS -m 8 --noproxy '*' http://example.com -o /dev/null 2>&1")
        self.assertTrue(
            "Couldn't connect" in out or "Could not resolve" in out,
            out,
        )

    def test_there_is_no_docker_socket(self) -> None:
        code, out = self.sh("test -e /var/run/docker.sock && echo present || echo absent")
        self.assertIn("absent", out)
        self.assertEqual(code, 0)


class ProxyUseTest(BoundaryCase):
    def test_in_scope_plain_http_works(self) -> None:
        _, out = self.sh("curl -sS -m 15 -o /dev/null -w '%{http_code}' http://example.com")
        self.assertIn("200", out)

    def test_in_scope_tls_works(self) -> None:
        # Proves CONNECT, which is the only tunnel the proxy supports.
        _, out = self.sh("curl -sS -m 20 -o /dev/null -w '%{http_code}' https://example.com")
        self.assertIn("200", out)

    def test_an_out_of_scope_host_is_refused(self) -> None:
        _, out = self.sh(f"curl -sS -m 15 {OUT_OF_SCOPE} -o /dev/null -w '%{{http_code}}'")
        self.assertIn("403", out)

    def test_a_prohibited_port_is_refused(self) -> None:
        # The host is in scope only on 80/443; 8443 must be denied by the port rule.
        _, out = self.sh("curl -sS -m 15 https://example.com:8443 -o /dev/null -w '%{http_code}'")
        self.assertIn("403", out)


class HygieneTest(BoundaryCase):
    def test_the_root_is_read_only(self) -> None:
        _, out = self.sh("touch /nope 2>&1 || true")
        self.assertIn("Read-only file system", out)

    def test_artifacts_are_writable(self) -> None:
        code, out = self.sh("echo hi > /artifacts/x && cat /artifacts/x")
        self.assertEqual(code, 0)
        self.assertEqual(out.strip(), "hi")

    def test_it_runs_unprivileged(self) -> None:
        _, out = self.sh("id -u")
        self.assertEqual(out.strip(), str(boundary.GUEST_UID))

    def test_tmp_is_writable_so_tools_can_stage(self) -> None:
        code, _ = self.sh("touch /tmp/x")
        self.assertEqual(code, 0)


class LifecycleTest(BoundaryCase):
    def test_a_timeout_kills_the_process_group(self) -> None:
        code, _ = self.sh("sleep 30", timeout=2)
        self.assertEqual(code, 124)

    def test_teardown_leaves_nothing_behind(self) -> None:
        names = [self.b.guest, self.b.proxy, self.b.network]
        self.b.down()
        for ref in names:
            res = boundary._run(["docker", "inspect", ref])
            self.assertNotEqual(res.returncode, 0, f"{ref} survived teardown")

    def test_a_crashed_run_restarts_cleanly(self) -> None:
        # `up` must clear its own leftovers: a second up on the same key has to
        # work, which is what makes recovery from a crash possible.
        key = self.b.scope_key
        again = boundary.new_boundary(key, IMAGE, self.dir / "again")
        boundary.write_rules(again.rules_path, boundary.rules_for(["example.com:80"], []))
        again.up()
        try:
            res = again.exec(["sh", "-c", "id -u"], 20)
            self.assertEqual(res.stdout.strip(), str(boundary.GUEST_UID))
        finally:
            again.down()


class RecordTest(BoundaryCase):
    def test_the_runtime_record_captures_driftable_facts(self) -> None:
        rec = self.b.record()
        self.assertTrue(rec["image"].startswith("sha256:"), "image digest not recorded")
        self.assertTrue(rec["read_only_root"])
        self.assertEqual(rec["proxy_legs"], ["isolated", "bridge"])


if __name__ == "__main__":
    unittest.main()


class EndToEndTest(BoundaryCase):
    """Drives the real daemon inside the boundary: the whole loop, no mocks."""

    def setUp(self) -> None:
        super().setUp()
        self.frames = "".join(
            json.dumps(f) + "\n"
            for f in [
                {"v": 2},
                {"id": 1, "op": "call", "tool": "sec_http", "args": {"url": "http://example.com/"}},
                {"id": 2, "op": "call", "tool": "sec_http", "args": {"url": OUT_OF_SCOPE_URL}},
                {"id": 3, "op": "call", "tool": "sec_http", "args": {"url": "http://example.com/"}},
            ]
        )
        (self.dir / "artifacts").mkdir(parents=True, exist_ok=True)
        (self.dir / "artifacts" / "in.jsonl").write_text(self.frames)

    def _run(self) -> dict:
        self.b.exec(["sh", "-c", "mkdir -p /tmp/run"], 20)
        res = self.b.exec(
            ["sh", "-c",
             "cd /opt/koma && KOMA_BOARD=/tmp/run/board timeout 90 "
             "python3 -m koma_sec_daemon < /artifacts/in.jsonl 2>/tmp/run/err"],
            150,
        )
        out = {}
        handshake = None
        for line in res.stdout.splitlines():
            try:
                frame = json.loads(line)
            except ValueError:
                continue
            if "tools" in frame:
                handshake = len(frame["tools"])
                continue
            if "id" in frame:
                out[frame["id"]] = json.loads(frame["result"])["verdict"]
        return {"handshake": handshake, "verdicts": out}

    def test_the_loop_holds_end_to_end(self) -> None:
        got = self._run()
        self.assertEqual(got["handshake"], 28, "the daemon did not advertise its tools")
        self.assertEqual(got["verdicts"].get(1), "confirmed", "an in-scope call failed")
        # The proxy's refusal must be distinguishable from the target answering.
        self.assertEqual(got["verdicts"].get(2), "blocked", "out-of-scope was not blocked")
        # The duplicate check is at dispatch, so the same route twice is refused.
        self.assertEqual(got["verdicts"].get(3), "blocked", "a duplicate route was re-run")

    def test_evidence_is_written_where_the_board_points(self) -> None:
        self._run()
        listing = self.b.exec(["sh", "-c", "ls /tmp/run/board/evidence 2>&1 | head -4"], 20).stdout
        self.assertIn(".raw", listing)
