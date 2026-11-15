#!/usr/bin/env python3
"""End-to-end smoke for `skills.*`.

Runs the real adapter (`koma sdk serve`) in an isolated data directory with a
fake provider whose key must never appear on stdout, and sends raw JSON lines.
Every frame is validated against schema.json when jsonschema is importable.
The first failed step exits non-zero; the adapter and the session daemon this
script started are stopped at the end. No model is called.

usage: smoke_skills.py [path/to/koma]   (default: target/debug/koma)
"""
import base64
import io
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "src-core-sdk" / "protocol" / "schema.json"
PLANTED_KEY = "sk-smoke-0000000000"
METHODS = ("skills.list", "skills.get", "skills.read_file", "skills.install",
           "skills.remove", "skills.create", "skills.update")
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
        # Events seen while waiting for a response.
        self.events = []

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
            self.events.append(frame)

    def ok(self, method, params):
        frame = self.call(method, params)
        check("result" in frame, f"{method} failed: {frame.get('error')}")
        return frame["result"]

    def err_of(self, method, params, well_formed=True):
        frame = self.call(method, params, well_formed)
        check("error" in frame, f"{method} should fail, got {frame.get('result')}")
        return frame["error"]

    def mutate(self, method, params):
        """A skills mutation, then a read: no event may arrive in between."""
        before = len(self.events)
        result = self.ok(method, params)
        self.ok("skills.list", {"scope": "computer"})
        check(len(self.events) == before, f"an event followed {method}: {self.events[before:]}")
        return result

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


def names(listing, tier=None):
    return [r["name"] for r in listing["skills"] if tier is None or r["tier"] == tier]


def package(files):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, text in files.items():
            archive.writestr(name, text)
    return base64.b64encode(buffer.getvalue()).decode("ascii")


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
    work = Path(tempfile.mkdtemp(prefix="kw.")).resolve()
    env = dict(os.environ, KOMA_TEST_DATA_DIR=str(data), KOMA_TELEMETRY="0", KOMA_TEST_OFFLINE="1")
    stdout_log = []
    a = b = None
    try:
        a = Adapter(binary, env, validator, stdout_log)
        hello = a.hello
        for method in METHODS:
            check(method in hello["methods"], f"hello.methods lacks {method}")
        check(hello["capabilities"] == ["create", "resume", "state", "submit", "enqueue", "interrupt",
                                        "interrupt_and_submit", "approve", "configure", "detach"],
              "hello.capabilities changed")
        check(hello["limits"]["list"]["skills"] == 1000, "limits.list.skills")
        check(hello["limits"]["skills"]["inline_zip_bytes"] == 29360128, "limits.skills.inline_zip_bytes")
        check(hello["limits"]["skills"]["zip_bytes"] == 67108864, "limits.skills.zip_bytes")
        check("skills_changed" not in hello["events"], "hello.events has a skills event")
        step("hello advertises the skills methods and limits, and no skills event")

        a.ok("configure", {
            "providers": [{"name": "smoke", "endpoint": "http://127.0.0.1:9/v1",
                           "api_type": "openai", "api_key": PLANTED_KEY}],
            "models": {"main": {"provider": "smoke", "model_id": "smoke/model"}},
        })
        created = a.mutate("skills.create", {
            "scope": "computer", "name": "demo", "description": "Smoke demo",
            "instruction": "Say hello.", "triggers": ["hello", "greet"],
        })
        skill = created["skill"]
        check((skill["tier"], skill["scope"], skill["raw"]) == ("global", "computer", None), f"{skill}")
        check(created["propagation"] == {"skills": "next_rebuild"}, f"{created['propagation']}")
        check((data / "skills" / "demo" / "SKILL.md").is_file(), "demo is not in the data dir's skills")
        listing = a.ok("skills.list", {})
        check(names(listing) == ["demo"] and listing["project_workdir"] is None, f"{listing}")
        reason(a.err_of("skills.create", {"scope": "computer", "name": "demo",
                                          "description": "x", "instruction": "y"}),
               "invalid_state", "exists")
        reason(a.err_of("skills.create", {"scope": "computer", "name": "readme",
                                          "description": "x", "instruction": "y"}, False),
               "invalid_request", "invalid_name")
        reason(a.err_of("skills.list", {"scope": "project"}), "invalid_request", "workdir_required")
        step("unbound: skills.create{computer, demo} then skills.list; exists, invalid_name, workdir_required")

        updated = a.mutate("skills.update", {
            "tier": "global", "name": "demo", "generation": skill["generation"],
            "patch": {"name": "demo2"},
        })
        check(updated["renamed"] is True and updated["skill"]["name"] == "demo2", f"{updated}")
        check(updated["skill"]["instruction"] == "Say hello.", "the rename changed the body")
        listing = a.ok("skills.list", {})
        check(names(listing) == ["demo2"], f"after rename {names(listing)}")
        edited = a.mutate("skills.update", {"tier": "global", "name": "demo2",
                                            "generation": updated["skill"]["generation"],
                                            "patch": {"description": "Edited"}})
        check(edited["skill"]["description"] == "Edited" and edited["renamed"] is False, f"{edited}")
        reason(a.err_of("skills.update", {"tier": "global", "name": "demo2",
                                          "generation": updated["skill"]["generation"],
                                          "patch": {"description": "stale"}}),
               "invalid_state", "stale")
        step("skills.update{patch:{name:'demo2'}}: the list shows demo2 and no demo; an old generation is stale")

        data_b64 = package({
            "packed/SKILL.md": "---\ndescription: Packed smoke skill\n---\nUse the notes.\n",
            "packed/notes/guide.txt": "companion text\n",
        })
        installed = a.mutate("skills.install", {"scope": "computer", "data_b64": data_b64})
        check(installed["skill"]["name"] == "packed", f"{installed['skill']}")
        check(installed["skill"]["companion_files"] == ["notes/guide.txt"], f"{installed['skill']}")
        text = a.ok("skills.read_file", {"tier": "global", "name": "packed", "path": "notes/guide.txt"})
        check(text == {"path": "notes/guide.txt", "text": "companion text\n", "bytes": 15}, f"{text}")
        reason(a.err_of("skills.read_file", {"name": "packed", "path": "../demo2/SKILL.md"}),
               "invalid_request", "out_of_range")
        raw = a.ok("skills.get", {"name": "packed", "include_raw": True})["raw"]
        check(raw == "---\ndescription: Packed smoke skill\n---\nUse the notes.\n", f"raw {raw!r}")
        reason(a.err_of("skills.install", {"scope": "computer", "data_b64": package({
            "mac/SKILL.md": "---\ndescription: x\n---\nx\n", "__MACOSX/mac/._SKILL.md": "x"})}),
            "invalid_request", "invalid_package")
        reason(a.err_of("skills.install", {"scope": "computer", "data_b64": "UEs=",
                                           "path": str(work / "x.zip")}),
               "invalid_request", "conflicting_params")
        removed = a.mutate("skills.remove", {"tier": "global", "name": "packed"})
        check(removed == {"removed": True, "now_effective": None,
                          "propagation": {"skills": "next_rebuild"}}, f"{removed}")
        check(not (data / "skills" / "packed").exists(), "packed is still on disk")
        reason(a.err_of("skills.remove", {"tier": "global", "name": "packed"}), "not_found", "not_found")
        step("skills.install{data_b64} with a built zip, then read_file, get{include_raw}, then remove")

        broken = data / "skills" / "broken"
        broken.mkdir(parents=True)
        (broken / "SKILL.md").write_text("no frontmatter here\n")
        claude = work / ".claude" / "skills" / "imported"
        claude.mkdir(parents=True)
        (claude / "SKILL.md").write_text("---\ndescription: From claude\n---\nx\n")
        listing = a.ok("skills.list", {"workdir": str(work), "include_diagnostics": True})
        check(listing["project_workdir"] == str(work), f"workdir {listing['project_workdir']}")
        check(names(listing, "claude") == ["imported"], f"{listing['skills']}")
        paths = [d["path"] for d in listing["diagnostics"]]
        check(str(broken / "SKILL.md") in paths, f"diagnostics {listing['diagnostics']}")
        reason(a.err_of("skills.update", {"tier": "claude", "workdir": str(work), "name": "imported",
                                          "patch": {"description": "x"}}),
               "invalid_state", "read_only")
        step("a skipped skill is a diagnostic; the claude tier is listed and read-only")

        a.close()
        a = None

        b = Adapter(binary, env, validator, stdout_log)
        b.ok("create", {"cwd": str(work)})
        project = b.ok("skills.create", {"scope": "project", "name": "bound-skill",
                                         "description": "Made while bound", "instruction": "Body."})
        check(project["propagation"] == {"skills": "next_rebuild"}, f"{project['propagation']}")
        check(project["skill"]["tier"] == "project-agents", f"{project['skill']}")
        check((work / ".agents" / "skills" / "bound-skill" / "SKILL.md").is_file(),
              "the project skill is not in the bound cwd")
        listing = b.ok("skills.list", {"scope": "project"})
        check(listing["project_workdir"] == str(work), f"{listing['project_workdir']}")
        check(names(listing) == ["bound-skill"], f"{names(listing)}")
        state = b.ok("state", {})
        check("state" in state, "state after skills calls")
        check(b.ok("detach", {}) == {"detached": True}, "B: detach")
        b.close()
        b = None
        step("bound: create{cwd}, then skills.create{scope:project}: propagation.skills is next_rebuild")

        check(not any(PLANTED_KEY in line for line in stdout_log), "the planted key appeared on stdout")
        print("PASS")
    finally:
        for adapter in (a, b):
            if adapter:
                adapter.close()
        stop_daemons(data)
        shutil.rmtree(data, ignore_errors=True)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
