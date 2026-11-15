#!/usr/bin/env python3
"""End-to-end smoke for the ROOM binding, the room reads and the team methods.

Runs real adapters (`koma sdk serve`) and a real secure room daemon in an
isolated data directory with a fake provider whose key must never appear on
stdout, and sends raw JSON lines. No model is called: the parent's Main route
counts as usable because it has a key, and the room is driven with shell lines
and team writes only. Every frame is validated against schema.json when
jsonschema is importable. The first failed step exits non-zero; every adapter
and daemon this script started is stopped at the end.

usage: smoke_room.py [path/to/koma]   (default: target/debug/koma)
"""
import json
import os
import queue
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
# The room adds the exit code on a line of its own after the output.
SHELL_TEXT = "$ echo smoke\nsmoke\n"
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
    """One `koma sdk serve` process. Frames are read on a thread, so events can
    be awaited with a deadline."""

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
        self.lines = queue.Queue()
        self.events = []
        threading.Thread(target=lambda: self.err.extend(self.proc.stderr), daemon=True).start()
        threading.Thread(target=self._read, daemon=True).start()
        self.hello = self.frame()
        check(self.hello.get("type") == "hello", "first frame is not hello")
        self.n = 0

    def _read(self):
        for line in self.proc.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def frame(self, timeout=60):
        try:
            line = self.lines.get(timeout=timeout)
        except queue.Empty:
            fail(f"no frame within {timeout} s")
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
            if frame.get("type") == "event":
                self.events.append(frame)
            elif frame.get("id") == request["id"]:
                return frame

    def ok(self, method, params):
        frame = self.call(method, params)
        check("result" in frame, f"{method} failed: {frame.get('error')}")
        return frame["result"]

    def err_of(self, method, params, well_formed=True):
        frame = self.call(method, params, well_formed)
        check("error" in frame, f"{method} should fail, got {str(frame.get('result'))[:300]}")
        return frame["error"]

    def wait_event(self, pred, timeout=20):
        """The first event, seen or still to come, that `pred` accepts."""
        deadline = time.time() + timeout
        while True:
            for event in self.events:
                if pred(event):
                    return event
            left = deadline - time.time()
            check(left > 0, "the expected event did not come")
            frame = self.frame(timeout=left)
            check(frame.get("type") == "event", f"unexpected frame {str(frame)[:200]}")
            self.events.append(frame)

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
    """Stop the daemons this script started (their pid files are in its own data dir) and wait
    until they are gone, so none writes into the directory after it is removed."""
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


def avatar(team, aid):
    found = next((a for a in team["avatars"] if a["id"] == aid), None)
    check(found is not None, f"the team has no {aid}")
    return found


def lane(team, aid, name):
    found = next((l for l in avatar(team, aid)["lanes"] if l["lane"] == name), None)
    check(found is not None, f"{aid} has no {name} lane")
    return found


def is_shell_line(line):
    return line["kind"] == "shell" and (line["text"] or "").startswith(SHELL_TEXT)


def main():
    binary = sys.argv[1] if len(sys.argv) > 1 else str(ROOT / "target" / "debug" / "koma")
    check(os.path.exists(binary), f"no binary at {binary}")
    validator = None
    try:
        import jsonschema

        validator = jsonschema.Draft202012Validator(json.loads(SCHEMA.read_text()))
    except ImportError:
        print("note jsonschema not importable: frames are not schema-validated")

    # Short, so the daemon socket paths stay within the Unix limit.
    data = Path(tempfile.mkdtemp(prefix="kr.", dir="/tmp" if os.path.isdir("/tmp") else None))
    work = Path(tempfile.mkdtemp(prefix="kw."))
    env = dict(os.environ, KOMA_TEST_DATA_DIR=str(data), KOMA_TELEMETRY="0", KOMA_TEST_OFFLINE="1")
    stdout_log = []
    adapters = []

    def spawn():
        adapter = Adapter(binary, env, validator, stdout_log)
        adapters.append(adapter)
        return adapter

    try:
        a = spawn()
        hello = a.hello
        for method in ("room.open", "room.attach", "room.shelf.list", "room.item", "room.trajectory",
                       "room.reports", "room.spend", "room.team.get", "room.models.set",
                       "room.avatars.upsert", "room.avatars.remove"):
            check(method in hello["methods"], f"hello.methods lacks {method}")
        for feature in ("bindings.room", "events.room", "room.template"):
            check(feature in hello["features"], f"hello.features lacks {feature}")
        a.ok("configure", {
            "providers": [{"name": "smoke", "endpoint": "http://127.0.0.1:9/v1",
                           "api_type": "openai", "api_key": PLANTED_KEY}],
            "models": {"main": {"provider": "smoke", "model_id": "smoke/model"}},
        })
        step("configure the fake provider as Main; hello lists the room methods and room.template")

        created = a.ok("create", {"cwd": str(work)})
        parent = created["state"]["session_id"]
        check(a.ok("detach", {}) == {"detached": True}, "A: detach")
        a.close()
        step("A: create{cwd}, then detach: the parent session")

        b = spawn()
        opened = b.ok("room.open", {"parent_session_id": parent, "tail": 200})
        room = opened["room"]["session_id"]
        epoch = opened["epoch"]
        check(opened["started"] is True, f"started {opened['started']}")
        check({"sdk_user", "sdk_reload"} <= set(opened["daemon"]["features"]),
              f"features {opened['daemon']['features']}")
        check(opened["room"]["parent_session_id"] == parent, f"room {opened['room']}")
        status = opened["status"] or b.ok("room.status", {})
        check(status["live"] is True, f"status {status}")
        check(all(l["phase"] != "running" for l in status["lanes"]), f"lanes {status['lanes']}")
        step(f"B: room.open starts the room (features {sorted(opened['daemon']['features'])}), live and idle")

        team = b.ok("room.team.get", {"target": "room"})
        web = avatar(team, "red-web")
        check((web["display_effective"], web["alias_effective"]) == ("Lab Red Web", "LW"), f"red-web {web}")
        check(team["choices_source"] == "room_daemon" and team["choices"], f"choices {team['choices']}")
        step("B: room.team.get shows red-web with its base display and alias")

        choice = team["choices"][0]["id"]
        written = b.ok("room.models.set", {"target": "room", "avatar": "red-web", "lane": "work",
                                           "model": choice, "if_revision": team["revision"]})
        check(written["applies"] == "live" and written["propagation"]["room"] == "reloaded", f"{written['applies']} {written['propagation']}")
        web = avatar(written["team"], "red-web")
        check(web["display_effective"] == "Lab Red Web", f"display {web['display_effective']}")
        check(lane(written["team"], "red-web", "work")["model"] == {"value": choice, "origin": "session"},
              f"lane {lane(written['team'], 'red-web', 'work')}")
        step("B: room.models.set red-web work applies live and keeps the display")

        reason(b.err_of("room.models.set", {"target": "room", "avatar": "chief", "lane": "work", "model": choice}),
               "invalid_request", "no_such_lane")
        step("B: room.models.set chief work is no_such_lane")

        written = b.ok("room.avatars.upsert", {"target": "room", "id": "red-mobile", "role": "red", "alias": "RM"})
        check(written["applies"] == "live", f"applies {written['applies']}")
        check(avatar(written["team"], "red-mobile")["alias_effective"] == "RM", "red-mobile alias")
        step("B: room.avatars.upsert red-mobile applies live")

        sent = b.ok("room.send", {"text": "echo smoke", "shell": True})
        check(sent["acked"] is True, f"send {sent}")
        line = b.wait_event(lambda e: e["event"] == "room.line" and is_shell_line(e["line"]))
        check(line["epoch"] == epoch, "the shell line carries another epoch")
        step("B: room.send{shell} is acked and a shell room.line follows")

        history = b.ok("room.history", {"epoch": epoch})
        check(any(is_shell_line(l) for l in history["lines"]), "room.history lacks the shell line")
        shelf = b.ok("room.shelf.list", {})
        check(shelf["rows"] == [] and shelf["total"] == 0, f"shelf {shelf}")
        check(b.ok("room.trajectory", {})["goals"] == [], "trajectory is not empty")
        check(b.ok("room.reports", {})["reports"] == [], "reports are not empty")
        spend = b.ok("room.spend", {})
        check(spend["goals"] == [], f"spend {spend}")
        step(f"B: history has it; shelf, trajectory, reports and spend are empty (ledger {spend['ledger']['ledger']})")

        c = spawn()
        taken = c.ok("room.attach", {"session_id": room})
        check("red-mobile" in taken["roster"], f"roster {taken['roster']}")
        closed = b.wait_event(lambda e: e["event"] == "room.closed")
        check(closed["reason"] == "taken_over", f"closed {closed}")
        step("C: room.attach takes over (roster has red-mobile); B gets room.closed{taken_over}")

        check(b.ok("room.shelf.list", {})["rows"] == [], "closed shelf.list")
        check(any(is_shell_line(l) for l in b.ok("room.history", {"epoch": epoch})["lines"]), "closed history")
        reason(b.err_of("room.send", {"text": "hi"}), "invalid_state", "room_closed")
        written = b.ok("room.avatars.upsert", {"target": "room", "id": "red-ops", "role": "red"})
        check(written["applies"] == "next_reload" and written["propagation"]["room"] == "not_live",
              f"{written['applies']} {written['propagation']}")
        step("B in ROOM(closed): reads work, room.send is room_closed, a team write is next_reload")

        check(c.ok("room.close", {"stop": True}) == {"closed": True, "stopped": True}, "C: room.close")
        step("C: room.close{stop:true}")

        reason(b.err_of("room.attach", {"session_id": room}), "invalid_state", "not_running")
        reopened = b.ok("room.open", {"parent_session_id": parent, "tail": 200})
        check(reopened["started"] is True, "the room was not started again")
        check(any(is_shell_line(l) and l["replay"] for l in reopened["tail"]),
              "the replay lacks the shell line")
        check("red-ops" in reopened["roster"], f"roster {reopened['roster']}")
        step("B: room.attach is not_running; room.open replays the shell line and red-ops is on the team")

        written = b.ok("room.avatars.upsert", {"target": "template", "id": "red-cli", "role": "red"})
        check(written["applies"] == "next_start", f"applies {written['applies']}")
        check((data / "room" / "avatars.toml").is_file(), "no template file")
        team = b.ok("room.team.get", {"target": "room"})
        check(team["template_pending_restart"] is True, "template_pending_restart is not set")
        check(avatar(team, "red-cli")["defined_in"] == "template", "red-cli is not from the template")
        step("B: a template write is next_start, and the room view says a restart is pending")

        check(b.ok("room.close", {"stop": True})["stopped"] is True, "B: room.close")
        rows = b.ok("sessions.list", {"kind": "room"})["sessions"]
        row = next((r for r in rows if r["session_id"] == room), None)
        check(row is not None and row["live"] is False and row["parent_session_id"] == parent, f"room row {row}")
        step("B: room.close{stop:true}; sessions.list shows the room stopped, with its parent")

        check(not any(PLANTED_KEY in line for line in stdout_log), "the planted key appeared on stdout")
        print("PASS")
    finally:
        for adapter in adapters:
            adapter.close()
        stop_daemons(data)
        shutil.rmtree(data, ignore_errors=True)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
