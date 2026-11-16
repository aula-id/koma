#!/usr/bin/env python3
"""End-to-end smoke for the read paths on a koma home that holds nothing yet.

Runs the real adapter (`koma sdk serve`) twice: first with a data directory
that does not exist, then with one that exists and is empty. Every read answers
with an empty or missing result, and every lookup of a session answers as it
does for an unknown id; none answers `io_error`. After each call the data
directory must still be absent (or still empty): reads create nothing. Every
frame is validated against schema.json when jsonschema is importable. The first
failed step exits non-zero. No model is called and no daemon is started.

usage: smoke_fresh_home.py [path/to/koma]   (default: target/debug/koma)
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "src-core-sdk" / "protocol" / "schema.json"
UNKNOWN = "00000000-0000-4000-8000-000000000000"
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
        if self.validator:
            errors = list(self.validator.iter_errors(frame))
            check(not errors, f"schema: {errors[0].message if errors else ''} in {line[:300]}")
        return frame

    def call(self, method, params):
        self.n += 1
        request = {"id": str(self.n), "method": method, "params": params}
        if self.validator:
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
        frame = self.call(method, params)
        check("error" in frame, f"{method} should fail, got {frame.get('result')}")
        return frame["error"]

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=5)
        except Exception:
            self.proc.kill()
            self.proc.wait()


def expect(error, code, why):
    check(
        (error["code"], error.get("reason"), error["delivery"]) == (code, why, "not_sent"),
        f"expected {code}/{why}, got {error}",
    )


def snapshot(data):
    if not data.exists():
        return None
    return sorted(str(p.relative_to(data)) for p in data.rglob("*"))


def run_pass(binary, validator, data, label):
    before = snapshot(data)
    env = dict(os.environ, KOMA_TEST_DATA_DIR=str(data), KOMA_TELEMETRY="0", KOMA_TEST_OFFLINE="1")
    adapter = Adapter(binary, env, validator)

    def unchanged(what):
        now = snapshot(data)
        check(now == before, f"{what} created {now} under the data dir")
        step(f"{label}: {what}")

    try:
        unchanged("hello")

        view = adapter.ok("config.get", {})
        check(view["file"] == "missing" and view["revision"] == "absent", f"config.get {view['file']}")
        check(view["providers"] == [] and view["models"] == [], "config.get lists entries")
        unchanged("config.get answers file:missing")
        view = adapter.ok("config.get", {"view": "global"})
        check(view["file"] == "missing", f"config.get global {view['file']}")
        unchanged("config.get{view:global} answers file:missing")
        for method, params in (("config.effort.get", {"wait_ms": 0}), ("config.limits.get", {})):
            expect(adapter.err_of(method, params), "invalid_state", "wrong_binding")
            unchanged(f"{method} unbound answers wrong_binding")

        listed = adapter.ok("sessions.list", {})
        check(listed == {"sessions": [], "next_cursor": None}, f"sessions.list {listed}")
        unchanged("sessions.list is empty")
        listed = adapter.ok("sessions.list", {"kind": "room", "live": False})
        check(listed["sessions"] == [], f"sessions.list room {listed}")
        unchanged("sessions.list{kind:room, live:false} is empty")
        expect(adapter.err_of("sessions.rename", {"session_id": UNKNOWN, "name": "x"}),
               "not_found", "not_found")
        unchanged("sessions.rename of an unknown id is not_found")
        expect(adapter.err_of("sessions.delete", {"session_id": UNKNOWN}), "not_found", "not_found")
        unchanged("sessions.delete of an unknown id is not_found")
        stopped = adapter.ok("sessions.stop", {"session_id": UNKNOWN})
        check(stopped == {"stopped": True, "was_live": False}, f"sessions.stop {stopped}")
        unchanged("sessions.stop of an unknown id is was_live:false")

        usage = adapter.ok("usage.query", {})
        check(usage["ledger"] == "missing" and usage["totals"]["calls"] == 0, f"usage {usage}")
        unchanged("usage.query answers ledger:missing with zeros")
        usage = adapter.ok("usage.query", {"group_by": "session"})
        check(usage["ledger"] == "missing" and usage["groups"] == [], f"usage by session {usage}")
        unchanged("usage.query{group_by:session} has no groups")

        for scope in ("all", "computer"):
            skills = adapter.ok("skills.list", {"scope": scope})
            check(skills["skills"] == [] and skills["next_cursor"] is None, f"skills.list {skills}")
            unchanged(f"skills.list{{scope:{scope}}} is empty")
        expect(adapter.err_of("skills.get", {"name": "nothing-here"}), "not_found", "not_found")
        unchanged("skills.get is not_found")
        expect(adapter.err_of("skills.read_file", {"name": "nothing-here", "path": "notes.md"}),
               "not_found", "not_found")
        unchanged("skills.read_file is not_found")

        rooms = adapter.ok("room.list", {})
        check(rooms == {"rooms": [], "next_cursor": None}, f"room.list {rooms}")
        unchanged("room.list is empty")
        rooms = adapter.ok("room.list", {"parent_session_id": UNKNOWN})
        check(rooms["rooms"] == [], f"room.list parent {rooms}")
        unchanged("room.list{parent_session_id} is empty")
        team = adapter.ok("room.team.get", {"target": "template"})
        check(isinstance(team.get("avatars"), list), f"room.team.get {str(team)[:200]}")
        unchanged("room.team.get{target:template} answers the built-in team")
        expect(adapter.err_of("room.open", {"parent_session_id": UNKNOWN, "wait_ms": 0}),
               "not_found", "not_found")
        unchanged("room.open of an unknown parent is not_found")
        expect(adapter.err_of("room.attach", {"session_id": UNKNOWN, "wait_ms": 0}),
               "invalid_state", "not_running")
        unchanged("room.attach of an unknown id is not_running")
    finally:
        adapter.close()
    unchanged("the adapter exits")


def main():
    binary = sys.argv[1] if len(sys.argv) > 1 else str(ROOT / "target" / "debug" / "koma")
    check(os.path.exists(binary), f"no binary at {binary}")
    validator = None
    try:
        import jsonschema

        validator = jsonschema.Draft202012Validator(json.loads(SCHEMA.read_text()))
    except ImportError:
        print("note jsonschema not importable: frames are not schema-validated")

    parent = Path(tempfile.mkdtemp(prefix="kf."))
    try:
        absent = parent / "absent"
        run_pass(binary, validator, absent, "no data dir")
        check(not absent.exists(), "the data dir was created")
        empty = parent / "empty"
        empty.mkdir()
        run_pass(binary, validator, empty, "empty data dir")
        check(not any(empty.iterdir()), "the empty data dir is no longer empty")
        print("PASS")
    finally:
        shutil.rmtree(parent, ignore_errors=True)


if __name__ == "__main__":
    main()
