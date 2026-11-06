"""Gate 6, part 1: the board store under real cross-process contention."""
import json
import multiprocessing as mp
import os
import shutil
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from koma_sec_daemon import board  # noqa: E402


class Base(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = Path(tempfile.mkdtemp())
        self.b = board.Board(self.dir)

    def tearDown(self) -> None:
        shutil.rmtree(self.dir, ignore_errors=True)


class IdTest(Base):
    def test_identifiers_are_traversal_safe(self) -> None:
        for raw in ("../../etc/passwd", "a/b", "GET /admin", "x" * 200, "..", "Ünïcode"):
            got = board.safe_id(raw)
            self.assertNotIn("/", got)
            self.assertNotIn("..", got)
            self.assertTrue(got)
            self.assertLessEqual(len(got), 80)

    def test_safe_id_is_idempotent(self) -> None:
        # Every layer that re-hashes must get the same key, or a record written
        # through one path is unreachable through another.
        once = board.safe_id("../../etc/passwd")
        self.assertEqual(board.safe_id(once), once)

    def test_distinct_inputs_stay_distinct(self) -> None:
        self.assertNotEqual(board.safe_id("a/b"), board.safe_id("a-b"))
        self.assertNotEqual(board.safe_id("GET /a"), board.safe_id("GET  /a"))

    def test_an_empty_input_still_yields_an_id(self) -> None:
        self.assertTrue(board.safe_id(""))


class RecordTest(Base):
    def test_put_and_read_round_trip(self) -> None:
        self.b.put("asset", "a.example", {"note": "x"})
        rec = self.b.read("asset", "a.example")
        self.assertEqual(rec["body"]["note"], "x")
        self.assertEqual(rec["version"], 1)

    def test_put_bumps_the_version(self) -> None:
        self.b.put("route", "r1", {"a": 1})
        rec = self.b.put("route", "r1", {"a": 2})
        self.assertEqual(rec["version"], 2)

    def test_compare_and_set_rejects_a_stale_writer(self) -> None:
        self.b.put("route", "r1", {"a": 1})
        self.b.put("route", "r1", {"a": 2}, expect=1)
        with self.assertRaises(board.StaleWrite):
            self.b.put("route", "r1", {"a": 3}, expect=1)

    def test_an_unknown_kind_is_rejected(self) -> None:
        with self.assertRaises(board.BoardError):
            self.b.put("nonsense", "x", {})

    def test_list_returns_only_valid_records(self) -> None:
        self.b.put("asset", "a", {})
        self.b.put("asset", "b", {})
        (self.dir / "asset" / "junk.json").write_text("not json")
        self.assertEqual(len(self.b.list("asset")), 2)

    def test_a_record_with_another_schema_is_not_returned(self) -> None:
        p = self.dir / "asset" / "x.json"
        p.parent.mkdir(parents=True)
        p.write_text(json.dumps({"schema": 99, "body": {}}))
        self.assertIsNone(self.b.read("asset", "x"))


class ClaimTest(Base):
    def test_a_claim_blocks_another_owner(self) -> None:
        self.b.claim("r1", "alice")
        with self.assertRaises(board.StaleClaim):
            self.b.claim("r1", "bob")

    def test_re_claiming_your_own_route_is_a_renewal(self) -> None:
        first = self.b.claim("r1", "alice")
        second = self.b.claim("r1", "alice")
        self.assertGreater(second["body"]["expires"], first["body"]["expires"] - 1)

    def test_a_lapsed_claim_is_won_outright(self) -> None:
        self.b.claim("r1", "alice", lease=0.01)
        time.sleep(0.05)
        self.b.claim("r1", "bob")

    def test_release_only_frees_your_own(self) -> None:
        self.b.claim("r1", "alice")
        self.assertFalse(self.b.release("r1", "bob"))
        self.assertTrue(self.b.release("r1", "alice"))
        self.b.claim("r1", "bob")

    def test_a_live_claim_reports_as_claimed(self) -> None:
        self.b.claim("r1", "alice")
        held, owner = self.b.is_claimed("r1")
        self.assertTrue(held)
        self.assertEqual(owner, "alice")

    def test_an_expired_claim_is_not_reported_as_held(self) -> None:
        self.b.claim("r1", "alice", lease=0.01)
        time.sleep(0.05)
        held, _ = self.b.is_claimed("r1")
        self.assertFalse(held)


def _contend(root: str, owner: str, out: str) -> None:
    """Child: race to claim one route and record whether it won."""
    b = board.Board(Path(root))
    try:
        b.claim("shared-route", owner)
        Path(out).write_text("won")
    except board.StaleClaim:
        Path(out).write_text("lost")


def _append_many(root: str, tag: str, n: int) -> None:
    """Child: append n distinct records, so a lost update is visible."""
    b = board.Board(Path(root))
    for i in range(n):
        b.put("evidence", f"{tag}-{i}", {"tag": tag, "i": i})


class MultiProcessTest(Base):
    def test_two_processes_claiming_one_route_produce_one_owner(self) -> None:
        # The gate's core claim. Both start together; exactly one may win.
        ctx = mp.get_context("spawn")
        outs = [str(self.dir / "r0"), str(self.dir / "r1")]
        procs = [
            ctx.Process(target=_contend, args=(str(self.dir), f"p{i}", outs[i]))
            for i in range(2)
        ]
        for p in procs:
            p.start()
        for p in procs:
            p.join(timeout=30)
        results = [Path(o).read_text() for o in outs]
        self.assertEqual(sorted(results), ["lost", "won"], results)

    def test_concurrent_writers_do_not_lose_updates(self) -> None:
        # Atomic replacement alone passes single-writer tests and fails this one.
        ctx = mp.get_context("spawn")
        procs = [
            ctx.Process(target=_append_many, args=(str(self.dir), f"t{i}", 12))
            for i in range(4)
        ]
        for p in procs:
            p.start()
        for p in procs:
            p.join(timeout=60)
        self.assertEqual(len(self.b.list("evidence")), 48)

    def test_a_crashed_holder_does_not_wedge_the_board(self) -> None:
        # The lock is released by the kernel on death, so the next writer proceeds.
        ctx = mp.get_context("spawn")
        p = ctx.Process(target=_append_many, args=(str(self.dir), "dead", 500))
        p.start()
        time.sleep(0.05)
        p.kill()
        p.join(timeout=10)
        self.b.put("asset", "after", {"ok": True})
        self.assertEqual(self.b.read("asset", "after")["body"]["ok"], True)


if __name__ == "__main__":
    unittest.main()


def _release_contend(root: str, owner: str, out: str) -> None:
    """Child: claim then release a shared route repeatedly."""
    b = board.Board(Path(root))
    wins = 0
    for _ in range(20):
        try:
            b.claim("hot", owner, lease=30)
            wins += 1
            b.release("hot", owner)
        except board.StaleClaim:
            time.sleep(0.005)
    Path(out).write_text(str(wins))


class ContendLoopTest(Base):
    def test_claim_release_under_contention_stays_consistent(self) -> None:
        # Repeated claim/release from two processes stresses the read-modify-write
        # window; afterwards the board must not be left holding a ghost claim.
        ctx = mp.get_context("spawn")
        outs = [str(self.dir / "w0"), str(self.dir / "w1")]
        procs = [
            ctx.Process(target=_release_contend, args=(str(self.dir), f"o{i}", outs[i]))
            for i in range(2)
        ]
        for p in procs:
            p.start()
        for p in procs:
            p.join(timeout=60)
        wins = [int(Path(o).read_text()) for o in outs]
        self.assertGreater(sum(wins), 0, "nobody ever won a claim")
        held, _ = self.b.is_claimed("hot")
        self.assertFalse(held, "a claim was left behind")


class DispatchGateTest(Base):
    def test_a_free_route_is_admitted_and_claimed(self) -> None:
        self.assertIsNone(board.dispatch_gate(self.b, "r1", "alice"))
        held, owner = self.b.is_claimed("r1")
        self.assertTrue(held)
        self.assertEqual(owner, "alice")

    def test_a_route_claimed_by_another_is_refused(self) -> None:
        self.b.claim("r1", "bob")
        got = board.dispatch_gate(self.b, "r1", "alice")
        self.assertIn("already claimed by bob", got)

    def test_re_entering_your_own_claim_is_allowed(self) -> None:
        self.b.claim("r1", "alice")
        self.assertIsNone(board.dispatch_gate(self.b, "r1", "alice"))

    def test_an_expired_claim_is_admitted(self) -> None:
        self.b.claim("r1", "bob", lease=0.01)
        time.sleep(0.05)
        self.assertIsNone(board.dispatch_gate(self.b, "r1", "alice"))


class DaemonWiringTest(unittest.TestCase):
    """Drives the real daemon: the gate must run before the tool, not in a prompt."""

    def setUp(self) -> None:
        self.dir = Path(tempfile.mkdtemp())
        self.env = dict(os.environ, KOMA_BOARD=str(self.dir))

    def tearDown(self) -> None:
        shutil.rmtree(self.dir, ignore_errors=True)

    def _daemon(self):
        import subprocess

        p = subprocess.Popen(
            [sys.executable, "-m", "koma_sec_daemon"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, cwd=str(Path(__file__).parent), env=self.env,
        )
        p.stdin.write(json.dumps({"v": 2}) + "\n")
        p.stdin.flush()
        p.stdout.readline()
        return p

    def test_output_is_retained_as_evidence(self) -> None:
        p = self._daemon()
        try:
            p.stdin.write(json.dumps({
                "id": 1, "op": "call", "tool": "sec_decode",
                "args": {"data": "aGVsbG8=", "mode": "auto"},
            }) + "\n")
            p.stdin.flush()
            p.stdout.readline()
            time.sleep(0.6)
            self.assertTrue((self.dir / "evidence" / "index.json").exists())
        finally:
            p.kill()
            p.wait(timeout=10)

    def test_a_claimed_route_blocks_a_second_frame(self) -> None:
        # Pre-claim the route the next call will use, from another owner.
        self.b = board.Board(self.dir)
        self.b.claim("sec_http a.example:", "someone-else")
        p = self._daemon()
        try:
            p.stdin.write(json.dumps({
                "id": 7, "op": "call", "tool": "sec_http",
                "args": {"url": "http://a.example/"},
            }) + "\n")
            p.stdin.flush()
            frame = json.loads(p.stdout.readline())
            self.assertIn("blocked", frame["result"])
        finally:
            p.kill()
            p.wait(timeout=10)


class RouteOfTest(unittest.TestCase):
    def test_a_url_becomes_a_route(self) -> None:
        from koma_sec_daemon.__main__ import _route_of

        self.assertEqual(_route_of("sec_ffuf", {"url": "https://A.example:8443/x"}), "sec_ffuf a.example:8443")

    def test_a_local_tool_has_no_route(self) -> None:
        from koma_sec_daemon.__main__ import _route_of

        self.assertIsNone(_route_of("sec_decode", {"data": "x"}))

    def test_a_network_tool_without_a_destination_has_no_route(self) -> None:
        from koma_sec_daemon.__main__ import _route_of

        self.assertIsNone(_route_of("sec_http", {}))
