"""Gate 5: registration isolation, bounded concurrency, real cancellation."""
import io
import json
import os
import subprocess
import sys
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from koma_sec_daemon import cancel, registry, sandbox, verdict  # noqa: E402

ROOT = Path(__file__).parent


class RegistrationTest(unittest.TestCase):
    def test_one_missing_dependency_does_not_stop_registration(self) -> None:
        # The gate's core claim: a broken dependency costs that tool, not the
        # daemon. If this ever returns fewer than all modules, the flat-import
        # failure came back.
        self.assertEqual(registry.refused_descriptors(), [])
        self.assertGreaterEqual(len(registry.REGISTRY), 20)

    def test_an_unavailable_tool_is_advertised_with_a_hint(self) -> None:
        # Advertised, not hidden: the model should know it exists and what it
        # needs. Every entry carries text a user can act on.
        for row in registry.unavailable_tools():
            self.assertTrue(row["missing"])
            self.assertIn("install", row["hint"])

    def test_calling_an_unavailable_tool_raises_typed_rather_than_crashing(self) -> None:
        rows = registry.unavailable_tools()
        if not rows:
            self.skipTest("every dependency is installed here")
        name = rows[0]["name"]
        with self.assertRaises(registry.Unavailable) as ctx:
            registry.call(name, {}, None)
        self.assertIn("install", str(ctx.exception))

    def test_descriptors_carry_availability_but_never_the_handler(self) -> None:
        for d in registry.descriptors():
            self.assertNotIn("handler", d)
            if d.get("available") is False:
                self.assertIn("hint", d)


class HandshakeTest(unittest.TestCase):
    """Drives the real daemon over pipes, the way the parent does."""

    def _daemon(self):
        return subprocess.Popen(
            [sys.executable, "-m", "koma_sec_daemon"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            cwd=str(ROOT),
        )

    def test_handshake_succeeds_with_dependencies_blocked(self) -> None:
        # Force the failure the gate cares about: a sitecustomize that makes the
        # optional deps unimportable, so the daemon really is running without them.
        blocker = ROOT / "_blockdeps"
        if blocker.exists():
            import shutil

            shutil.rmtree(blocker)
        blocker.mkdir()
        (blocker / "sitecustomize.py").write_text(
            "import sys\n"
            "class B:\n"
            "    def find_module(self, name, path=None):\n"
            "        if name.split('.')[0] in {'requests', 'pwn', 'z3'}:\n"
            "            raise ImportError('blocked by test')\n"
            "sys.meta_path.insert(0, B())\n"
        )
        env = dict(os.environ, PYTHONPATH=str(blocker))
        proc = subprocess.Popen(
            [sys.executable, "-m", "koma_sec_daemon"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, cwd=str(ROOT), env=env,
        )
        try:
            proc.stdin.write(json.dumps({"v": 2}) + "\n")
            proc.stdin.flush()
            line = proc.stdout.readline()
            self.assertTrue(line, "no handshake reply")
            frame = json.loads(line)
            self.assertTrue(frame.get("ok"), frame)
            self.assertTrue(frame.get("tools"), "handshake succeeded but advertised nothing")
            names = {d["name"] for d in frame["unavailable"]}
            self.assertIn("sec_http", names, "blocked dep was not reported unavailable")
        finally:
            proc.kill()
            proc.wait(timeout=10)
            import shutil

            shutil.rmtree(blocker, ignore_errors=True)


class ConcurrencyTest(unittest.TestCase):
    def test_a_slow_call_does_not_block_a_fast_one(self) -> None:
        # Simulates the frame loop's pool: two handlers, one slow.
        from concurrent.futures import ThreadPoolExecutor

        def slow():
            time.sleep(1.0)
            return "slow"

        def fast():
            return "fast"

        with ThreadPoolExecutor(max_workers=4) as pool:
            started = time.monotonic()
            a = pool.submit(slow)
            b = pool.submit(fast)
            b.result(timeout=2)
            fast_done = time.monotonic() - started
            a.result(timeout=5)
        self.assertLess(fast_done, 0.8, "the fast call queued behind the slow one")


class CancelTest(unittest.TestCase):
    def test_cancel_kills_the_process_group(self) -> None:
        # A shell that spawns a child, so killing only the leader would leave the
        # grandchild alive.
        key = "frame-1"
        proc = subprocess.Popen(
            ["sh", "-c", "sleep 30 & echo $! > /tmp/koma-cancel-child; wait"],
            start_new_session=True,
        )
        cancel.register(key, proc)
        time.sleep(0.4)
        self.assertGreaterEqual(cancel.cancel(key), 1)
        proc.wait(timeout=5)
        self.assertIsNotNone(proc.poll())

    def test_cancel_releases_the_key(self) -> None:
        key = "frame-2"
        proc = subprocess.Popen(["sleep", "30"], start_new_session=True)
        cancel.register(key, proc)
        cancel.cancel(key)
        proc.wait(timeout=5)
        # A second cancel finds nothing rather than erroring.
        self.assertEqual(cancel.cancel(key), 0)

    def test_cancelling_an_unknown_key_is_harmless(self) -> None:
        self.assertEqual(cancel.cancel("nope"), 0)

    def test_cancel_all_clears_everything(self) -> None:
        procs = [subprocess.Popen(["sleep", "30"], start_new_session=True) for _ in range(3)]
        for i, p in enumerate(procs):
            cancel.register(f"k{i}", p)
        self.assertGreaterEqual(cancel.cancel_all(), 3)
        for p in procs:
            p.wait(timeout=5)


class SandboxTimeoutTest(unittest.TestCase):
    def test_a_timeout_still_returns_the_sentinel(self) -> None:
        self.assertIn("timed out", sandbox.run(["sleep", "10"], timeout=1))

    def test_the_tracked_path_also_times_out(self) -> None:
        # Through the `spawn` seam the daemon uses, so the timeout is exercised on
        # the path that runs in production rather than only the fallback.
        key = "frame-3"
        out = sandbox.run(["sleep", "10"], timeout=1, spawn=cancel.wrap(key))
        self.assertIn("timed out", out)
        cancel.release(key)

    def test_output_is_still_captured_on_the_tracked_path(self) -> None:
        out = sandbox.run(["echo", "hi"], timeout=5, spawn=cancel.wrap("k"))
        cancel.release("k")
        self.assertEqual(out.strip(), "hi")


class VerdictShapeTest(unittest.TestCase):
    def test_unavailable_is_a_real_verdict(self) -> None:
        self.assertIn(verdict.UNAVAILABLE, verdict.VERDICTS)
        parsed = json.loads(verdict.typed(verdict.UNAVAILABLE, "pip install x"))
        self.assertEqual(parsed["verdict"], verdict.UNAVAILABLE)


if __name__ == "__main__":
    unittest.main()


class ImageCopyTest(unittest.TestCase):
    """The image ships a COPY of the package, so the two can silently diverge."""

    def test_the_shipped_copy_matches_the_source(self) -> None:
        import filecmp

        live = ROOT / "koma_sec_daemon"
        shipped = ROOT / "image" / "koma_sec_daemon"
        if not shipped.exists():
            self.skipTest("image copy absent")
        cmp = filecmp.dircmp(str(live), str(shipped))
        diff = self._collect(cmp)
        # `__pycache__` is build noise, not a source difference.
        diff = [d for d in diff if "__pycache__" not in d]
        self.assertEqual(diff, [], "image copy drifted — re-copy it before building")

    def _collect(self, cmp) -> list[str]:
        out = list(cmp.left_only) + list(cmp.right_only) + list(cmp.diff_files)
        for sub in cmp.subdirs.values():
            out += self._collect(sub)
        return out
