#!/usr/bin/env python3
"""End-to-end smoke for `usage.query` and `session_info.usage`.

Runs the real adapter (`koma sdk serve`) in an isolated data directory, seeds a
usage ledger and a session registry with Python's sqlite3, and sends raw JSON
lines. Every frame is validated against schema.json when jsonschema is
importable. The first failed step exits non-zero; the adapter and the session
daemon this script started are stopped at the end.

usage: smoke_usage.py [path/to/koma]   (default: target/debug/koma)
"""
import json
import os
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "src-core-sdk" / "protocol" / "schema.json"
DAY = 86400
DAY0 = 1699920000  # 2023-11-14T00:00:00Z
STEP = 0


def fail(message):
    print(f"FAIL step {STEP}: {message}", flush=True)
    raise SystemExit(1)


def step(name):
    global STEP
    STEP += 1
    print(f"ok   step {STEP}: {name}", flush=True)


def check(cond, message):
    if not cond:
        fail(message)


def pwd_hash(path):
    return uuid.uuid5(uuid.NAMESPACE_OID, os.path.realpath(path)).hex


def seed_ledger(path, w1, w2):
    h1, h2 = pwd_hash(w1), pwd_hash(w2)
    # Costs are powers of two so every sum is exact in binary floating point.
    rows = [
        (DAY0 + 1 * 3600, "model-a", "main", "sess-1", h1, 100, 10, 5, 0.125),
        (DAY0 + 10 * 3600, "model-b", "sub:x", "sess-1", h1, 200, 20, 10, 0.25),
        (DAY0 + 18 * 3600, "model-a", "main", "sess-2", h2, 300, 30, 15, 0.5),
        (DAY0 + 30 * 3600, "model-b", "sub:x", "sess-2", h2, 400, 40, 20, 1.0),
        (DAY0 + 40 * 3600, "model-a", "main", "sess-1", h2, 500, 50, 25, 2.0),
        (DAY0 + 42 * 3600, "model-a", "sub:y", "sess-3", h2, 600, 60, 30, 4.0),
    ]
    conn = sqlite3.connect(path)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS usage (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL,"
        " model_id TEXT, role TEXT, session_uuid TEXT, pwd_hash TEXT, tokens_in INTEGER,"
        " tokens_cached INTEGER, tokens_out INTEGER, cost REAL)"
    )
    conn.executemany(
        "INSERT INTO usage (ts, model_id, role, session_uuid, pwd_hash, tokens_in,"
        " tokens_cached, tokens_out, cost) VALUES (?,?,?,?,?,?,?,?,?)",
        rows,
    )
    conn.commit()
    conn.close()


def seed_registry(data, w):
    """sess-1 is a coding session, sess-2 a room, sess-3 is not registered."""
    conn = sqlite3.connect(data / "session.sqlite")
    conn.execute(
        "CREATE TABLE IF NOT EXISTS sessions (uuid TEXT PRIMARY KEY, pwd_hash TEXT NOT NULL,"
        " name TEXT NOT NULL, workdir TEXT NOT NULL, created_at INTEGER NOT NULL,"
        " updated_at INTEGER NOT NULL)"
    )
    h = pwd_hash(w)
    for sid, name in (("sess-1", "Coding work"), ("sess-2", "Audit room")):
        conn.execute(
            "INSERT INTO sessions VALUES (?,?,?,?,?,?)", (sid, h, name, str(w), DAY0, DAY0)
        )
    conn.commit()
    conn.close()
    room = data / "sessions" / h / "sess-2"
    room.mkdir(parents=True)
    (room / "room.sqlite").write_bytes(b"")


class Adapter:
    def __init__(self, binary, env, validator):
        self.validator = validator
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
        frame = json.loads(line)
        if self.validator and frame.get("type") != "event":
            errors = list(self.validator.iter_errors(frame))
            check(not errors, f"schema: {errors[0].message if errors else ''}")
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

    def err_of(self, method, params):
        # Requests that break the schema are sent on purpose here.
        frame = self.call(method, params, well_formed=False)
        check("error" in frame, f"{method} should fail")
        return frame["error"]

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=5)
        except Exception:
            self.proc.kill()


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


def near(a, b):
    return abs(a - b) < 1e-9


def totals(t, calls, tin, cached, tout, cost):
    check(
        (t["calls"], t["tokens_in"], t["tokens_cached"], t["tokens_out"]) == (calls, tin, cached, tout)
        and near(t["cost"], cost),
        f"totals {t} != {(calls, tin, cached, tout, cost)}",
    )


def main():
    binary = sys.argv[1] if len(sys.argv) > 1 else str(ROOT / "target" / "debug" / "koma")
    check(os.path.exists(binary), f"no binary at {binary}")
    validator = None
    try:
        import jsonschema

        schema = json.loads(SCHEMA.read_text())
        validator = jsonschema.Draft202012Validator(schema)
    except ImportError:
        print("note jsonschema not importable: frames are not schema-validated")

    data = Path(tempfile.mkdtemp(prefix="ku.", dir="/tmp" if os.path.isdir("/tmp") else None))
    work = Path(tempfile.mkdtemp(prefix="kw."))
    w1, w2 = work / "one", work / "two"
    w1.mkdir()
    w2.mkdir()
    env = dict(os.environ, KOMA_TEST_DATA_DIR=str(data), KOMA_TELEMETRY="0", KOMA_TEST_OFFLINE="1")
    adapter = None
    try:
        ledger = data / "usage.sqlite"
        seed_ledger(ledger, w1, w2)
        seed_registry(data, w2)
        adapter = Adapter(binary, env, validator)
        hello = adapter.hello
        check("usage.query" in hello["methods"], "hello.methods lacks usage.query")
        check(hello["limits"]["list"]["usage_groups"] == 500, "usage_groups limit")
        check(hello["limits"]["list"]["usage_buckets"] == 1000, "usage_buckets limit")
        check(hello["limits"]["time"]["usage_query_ms"] == 2000, "usage_query_ms limit")
        step("ledger seeded with 6 rows; hello advertises usage.query and its limits")

        window = {"from_at": DAY0, "to_at": DAY0 + 3 * DAY}
        r = adapter.ok("usage.query", window)
        check(r["ledger"] == "ok" and (r["from_at"], r["to_at"]) == (DAY0, DAY0 + 3 * DAY), "window")
        totals(r["totals"], 6, 2100, 210, 105, 7.875)
        for key in ("groups", "groups_truncated", "buckets"):
            check(key not in r, f"{key} present without being asked for")
        step("totals over the window")

        r = adapter.ok("usage.query", {**window, "group_by": "model"})
        g = r["groups"]
        check([x["key"] for x in g] == ["model-a", "model-b"], f"model order {g}")
        totals(g[0], 4, 1500, 150, 75, 6.625)
        totals(g[1], 2, 600, 60, 30, 1.25)
        check(r["groups_truncated"] is False and g[0]["label"] is None, "model group shape")
        step("group_by model")

        r = adapter.ok("usage.query", {**window, "group_by": "role"})
        g = r["groups"]
        check([x["key"] for x in g] == ["sub:y", "main", "sub:x"], f"role order {g}")
        totals(g[0], 1, 600, 60, 30, 4.0)
        totals(g[1], 3, 900, 90, 45, 2.625)
        totals(g[2], 2, 600, 60, 30, 1.25)
        check([(x["role_kind"], x["agent"]) for x in g] == [("sub", "y"), ("main", None), ("sub", "x")],
              "role kinds")
        step("group_by role")

        r = adapter.ok("usage.query", {**window, "group_by": "session"})
        g = r["groups"]
        check([x["key"] for x in g] == ["sess-3", "sess-1", "sess-2"], f"session order {g}")
        totals(g[0], 1, 600, 60, 30, 4.0)
        totals(g[1], 3, 800, 80, 40, 2.375)
        totals(g[2], 2, 700, 70, 35, 1.5)
        check((g[0]["label"], g[0]["kind"], g[0]["deleted"]) == (None, None, True), f"deleted {g[0]}")
        check((g[1]["label"], g[1]["kind"], g[1]["deleted"]) == ("Coding work", "coding", False), f"{g[1]}")
        check((g[2]["label"], g[2]["kind"], g[2]["deleted"]) == ("Audit room", "room", False), f"{g[2]}")
        step("group_by session joins names; an unregistered session is deleted")

        r = adapter.ok("usage.query", {**window, "group_by": "model", "limit": 1})
        check(len(r["groups"]) == 1 and r["groups_truncated"] is True, "limit truncates")
        r = adapter.ok("usage.query", {**window, "session_id": "sess-2"})
        totals(r["totals"], 2, 700, 70, 35, 1.5)
        r = adapter.ok("usage.query", {**window, "workdir": str(w1)})
        totals(r["totals"], 2, 300, 30, 15, 0.375)
        step("limit, session_id and workdir filters")

        r = adapter.ok("usage.query", {**window, "bucket": "day", "tz_offset_minutes": 420})
        b = r["buckets"]
        origin = DAY0 - 7 * 3600
        check([x["start_at"] for x in b] == [origin + i * DAY for i in range(4)], f"bucket starts {b}")
        totals(b[0], 2, 300, 30, 15, 0.375)
        totals(b[1], 3, 1200, 120, 60, 3.5)
        totals(b[2], 1, 600, 60, 30, 4.0)
        totals(b[3], 0, 0, 0, 0, 0.0)
        step("bucket day at UTC+7, empty bucket included")

        for params, code, reason in (
            ({"limit": 501}, "invalid_request", "out_of_range"),
            ({"tz_offset_minutes": 900}, "invalid_request", "out_of_range"),
            ({"group_by": "team"}, "invalid_request", "out_of_range"),
            ({"workdir": "relative"}, "invalid_request", "out_of_range"),
            ({"from_at": 100, "to_at": 100}, "invalid_request", "out_of_range"),
            ({"from_at": 0, "to_at": 401 * DAY}, "invalid_request", "range_too_large"),
            ({"from_at": 0, "to_at": 60 * DAY, "bucket": "hour"}, "invalid_request", "range_too_large"),
        ):
            e = adapter.err_of("usage.query", params)
            check((e["code"], e.get("reason"), e["delivery"]) == (code, reason, "not_sent"),
                  f"{params}: {e}")
        step("bounds: out_of_range and range_too_large, delivery not_sent")

        ledger.unlink()
        r = adapter.ok("usage.query", {**window, "group_by": "model", "bucket": "day", "tz_offset_minutes": 420})
        check(r["ledger"] == "missing", "ledger should be missing")
        totals(r["totals"], 0, 0, 0, 0, 0.0)
        check(r["groups"] == [] and len(r["buckets"]) == 4, "missing ledger shape")
        check(all(x["calls"] == 0 for x in r["buckets"]), "missing ledger buckets are zero")
        check(not ledger.exists(), "the query created the ledger")
        step("missing ledger: zeros, no error, no file created")

        r = adapter.ok("create", {"cwd": str(w1)})
        usage = r["session_info"]["usage"]
        check(isinstance(usage, dict), f"session_info.usage is {usage!r}")
        for key in ("last_prompt_tokens", "last_cached_tokens", "output_tokens_total", "cost_total"):
            check(key in usage, f"usage lacks {key}")
        check("usage" not in r["state"] and "session_info" not in r["state"], "state object changed")
        step("create: session_info.usage is present")

        print("PASS")
    finally:
        if adapter:
            adapter.close()
        stop_daemons(data)
        shutil.rmtree(data, ignore_errors=True)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
