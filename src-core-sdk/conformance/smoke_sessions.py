#!/usr/bin/env python3
"""End-to-end smoke for `sessions.*` and `create{name}`.

Runs two real adapters (`koma sdk serve`) in an isolated data directory with a
fake provider whose key must never appear on stdout, and sends raw JSON lines.
Every frame is validated against schema.json when jsonschema is importable.
The first failed step exits non-zero; the adapters and the session daemon this
script started are stopped at the end. No model is called.

usage: smoke_sessions.py [path/to/koma]   (default: target/debug/koma)
"""
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "src-core-sdk" / "protocol" / "schema.json"
PLANTED_KEY = "sk-smoke-0000000000"
# A pidfile with no socket beside it, as a daemon that is still starting leaves.
ORPHAN_ID = "5e551075-0000-4000-8000-000000000000"
STEP = 0


def fail(message):
    print(f"FAIL step {STEP + 1}: {message}", flush=True)
    raise SystemExit(1)


def step(name):
    global STEP
    STEP += 1
    print(f"ok   step {STEP}: {name}", flush=True)


def check(cond, message):
    if not cond:
        fail(message)


class Adapter:
    def __init__(self, binary, env, validator, stdout_log):
        self.validator = validator
        self.stdout_log = stdout_log
        self.proc = subprocess.Popen(
            [binary, "sdk", "serve"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            text=True,
        )
        self.err = []
        threading.Thread(target=lambda: self.err.extend(self.proc.stderr), daemon=True).start()
        self.hello = self.frame()
        check(self.hello.get("type") == "hello", "first frame is not hello")
        self.n = 0

    def frame(self):
        line = self.proc.stdout.readline()
        check(line, "adapter closed stdout: " + "".join(self.err)[-500:])
        self.stdout_log.append(line)
        check(PLANTED_KEY not in line, "the planted key appeared on stdout")
        frame = json.loads(line)
        if self.validator:
            errors = list(self.validator.iter_errors(frame))
            check(not errors, f"schema: {errors[0].message if errors else ''} in {line[:300]}")
        return frame

    def call(self, method, params, well_formed=True):
        self.n += 1
        request = {"id": str(self.n), "method": method, "params": params}
        if self.validator and well_formed:
            errors = list(self.validator.iter_errors(request))
            check(not errors, f"request schema: {errors[0].message if errors else ''}")
        self.proc.stdin.write(json.dumps(request) + "\n")
        self.proc.stdin.flush()
        while True:
            frame = self.frame()
            if frame.get("id") == request["id"]:
                return frame

    def ok(self, method, params):
        frame = self.call(method, params)
        check("result" in frame, f"{method} failed: {frame.get('error')}")
        return frame["result"]

    def err_of(self, method, params, well_formed=True):
        frame = self.call(method, params, well_formed)
        check("error" in frame, f"{method} should fail, got {frame.get('result')}")
        return frame["error"]

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=5)
        except Exception:
            self.proc.kill()


def reason(error, code, why):
    check(
        (error["code"], error.get("reason"), error["delivery"]) == (code, why, "not_sent"),
        f"expected {code}/{why}, got {error}",
    )


def stop_daemons(data):
    """Stop the session daemons this script started (their pid files are in its own data dir)
    and wait until they are gone, so none writes into the directory after it is removed."""
    run = data / "run"
    pids = []
    for pidfile in run.glob("*.pid") if run.is_dir() else []:
        try:
            pids.append(int(pidfile.read_text().strip()))
        except ValueError:
            continue
    for pid in pids:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass
    deadline = time.time() + 10
    for pid in pids:
        while time.time() < deadline:
            try:
                os.kill(pid, 0)
            except OSError:
                break
            time.sleep(0.1)


def rows(adapter, **params):
    return adapter.ok("sessions.list", params)["sessions"]


def find(rows_, sid):
    return next((r for r in rows_ if r["session_id"] == sid), None)


def main():
    binary = sys.argv[1] if len(sys.argv) > 1 else str(ROOT / "target" / "debug" / "koma")
    check(os.path.exists(binary), f"no binary at {binary}")
    validator = None
    try:
        import jsonschema

        validator = jsonschema.Draft202012Validator(json.loads(SCHEMA.read_text()))
    except ImportError:
        print("note jsonschema not importable: frames are not schema-validated")

    # Short, so the daemon socket path stays within the Unix limit.
    data = Path(tempfile.mkdtemp(prefix="ks.", dir="/tmp" if os.path.isdir("/tmp") else None))
    work = Path(tempfile.mkdtemp(prefix="kw."))
    env = dict(os.environ, KOMA_TEST_DATA_DIR=str(data), KOMA_TELEMETRY="0", KOMA_TEST_OFFLINE="1")
    stdout_log = []
    a = b = None
    orphan = data / "run" / f"{ORPHAN_ID}.pid"
    try:
        a = Adapter(binary, env, validator, stdout_log)
        hello = a.hello
        for method in ("sessions.list", "sessions.rename", "sessions.delete", "sessions.stop"):
            check(method in hello["methods"], f"hello.methods lacks {method}")
        check("create.name" in hello["features"], "hello.features lacks create.name")
        check(hello["limits"]["list"]["sessions"] == 500, "limits.list.sessions")
        check(hello["limits"]["time"]["session_stop_ms"] == 8000, "limits.time.session_stop_ms")
        step("hello advertises the session methods, create.name and their limits")

        a.ok("configure", {
            "providers": [{"name": "smoke", "endpoint": "http://127.0.0.1:9/v1",
                           "api_type": "openai", "api_key": PLANTED_KEY}],
            "models": {"main": {"provider": "smoke", "model_id": "smoke/model"}},
        })
        reason(a.err_of("create", {"cwd": str(work), "name": "bad\u0001name"}, False),
               "invalid_request", "invalid_name")
        check(not (data / "run").is_dir() or not list((data / "run").glob("*.sock")),
              "a refused create started a daemon")
        created = a.ok("create", {"cwd": str(work), "name": "smoke"})
        sid = created["state"]["session_id"]
        check(created["session_info"]["name"] == "smoke", f"name {created['session_info']['name']}")
        check("warnings" not in created, f"warnings {created.get('warnings')}")
        step("A: an invalid name spawns nothing; create{cwd, name:'smoke'} names the session")

        (data / "run").mkdir(exist_ok=True)
        orphan.write_text("2147483646")
        b = Adapter(binary, env, validator, stdout_log)
        row = find(rows(b), sid)
        check(row is not None, "B: the session is not listed")
        check((row["name"], row["kind"], row["live"], row["locked"], row["bound"])
              == ("smoke", "coding", True, True, False), f"B row {row}")
        check(row["working"] is False and row["room_session_id"] is None, f"B row {row}")
        check(find(rows(a), sid)["bound"] is True, "A: the row is not bound")
        check(orphan.exists(), "sessions.list removed an orphan pidfile")
        step("B: sessions.list shows it live, locked, named 'smoke'; no pidfile was removed")

        reason(b.err_of("sessions.delete", {"session_id": sid}), "invalid_state", "live")
        reason(b.err_of("sessions.rename", {"session_id": sid, "name": "smoke2"}), "invalid_state", "live")
        reason(a.err_of("sessions.delete", {"session_id": sid}), "invalid_state", "bound_self")
        reason(a.err_of("sessions.stop", {"session_id": sid}), "invalid_state", "bound_self")
        check(find(rows(b), sid)["name"] == "smoke", "a refused rename changed the name")
        step("B: delete and rename are refused as live; A: delete and stop of its own session are bound_self")

        renamed = a.ok("sessions.rename", {"session_id": sid, "name": "smoke2"})
        check(renamed["session"]["name"] == "smoke2" and renamed["session"]["bound"], f"{renamed}")
        check(a.ok("sessions.info", {})["session_info"]["name"] == "smoke2", "A: sessions.info name")
        check(find(rows(b), sid)["name"] == "smoke2", "B: the listing does not show the new name")
        step("A: the bound rename lands in sessions.info and in B's listing")

        page = b.ok("sessions.list", {"limit": 1})
        check(len(page["sessions"]) == 1 and page["next_cursor"] is None, f"page {page}")
        reason(b.err_of("sessions.list", {"cursor": "%%%"}), "invalid_request", "bad_cursor")
        reason(b.err_of("sessions.list", {"limit": 501}, False), "invalid_request", "out_of_range")
        check(b.ok("sessions.list", {"kind": "room"})["sessions"] == [], "kind room is not empty")
        step("paging: one row, no next cursor; bad cursor and limit refused; kind filter")

        check(a.ok("detach", {}) == {"detached": True}, "A: detach")
        a.close()
        a = None
        stopped = b.ok("sessions.stop", {"session_id": sid})
        check(stopped["stopped"] is True and isinstance(stopped["was_live"], bool), f"{stopped}")
        row = find(rows(b), sid)
        check(row is not None and row["live"] is False and row["locked"] is False, f"after stop {row}")
        again = b.ok("sessions.stop", {"session_id": sid})
        check(again == {"stopped": True, "was_live": False}, f"second stop {again}")
        step(f"A: detach; B: sessions.stop (was_live={stopped['was_live']}), a second stop is a no-op")

        deleted = b.ok("sessions.delete", {"session_id": sid})
        check(deleted == {"deleted": True, "kind": "coding", "orphaned_room_session_id": None},
              f"{deleted}")
        check(find(rows(b), sid) is None, "the deleted session is still listed")
        reason_nf = b.err_of("sessions.delete", {"session_id": sid})
        check((reason_nf["code"], reason_nf.get("reason")) == ("not_found", "not_found"), f"{reason_nf}")
        reason_nf = b.err_of("sessions.delete", {"session_id": "../../etc"})
        check(reason_nf["code"] == "not_found", f"{reason_nf}")
        step("B: sessions.delete removes it; a retry and a path-like id are not_found")

        check(not any(PLANTED_KEY in line for line in stdout_log), "the planted key appeared on stdout")
        print("PASS")
    finally:
        for adapter in (a, b):
            if adapter:
                adapter.close()
        if orphan.exists():
            orphan.unlink()
        stop_daemons(data)
        shutil.rmtree(data, ignore_errors=True)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
