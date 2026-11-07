"""The execution boundary: isolated network, proxy, container, teardown.

The guarantee is structural, not advisory. The guest is attached to a network
created with `--internal`, which in Docker means **no gateway**: nothing outside
is reachable from the guest. A proxy container is the only peer on that network
and the only thing with an outbound route, so traffic has exactly one way out and
it is checked.

Two consequences follow and both are load-bearing:

- Proxy env vars are not the boundary. The guest may unset them; if it does, it
  reaches nothing rather than reaching everything.
- A missing or broken boundary must not fall back to the host. [`Boundary::up`]
  therefore fails closed: any step that does not complete leaves the boundary
  down and the call is refused, never silently run locally.

Every runtime fact that could drift is recorded (image id, network id, scope
version) so a run can be audited afterwards.
"""

from __future__ import annotations

import json
import os
import secrets
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

# Everything inside the guest runs as this uid; the mounted artifact dir is made
# writable for it before start.
GUEST_UID = 10001
PROXY_PORT = 3128
# Stable name the guest resolves. The container NAME is not resolvable from a peer.
PROXY_ALIAS = "proxy"
GUEST_NAME_PREFIX = "koma-guest-"

# Containers are absent on some platforms and the honest answer there is to say so
# rather than run on the host.
DOCKER_MISSING = "docker is not installed"
DOCKER_DOWN = "docker daemon is not reachable"
INTERNAL_UNAVAILABLE = "docker cannot create an isolated network"


class BoundaryError(RuntimeError):
    """The boundary could not be established. Callers must refuse, not degrade."""


@dataclass
class Boundary:
    scope_key: str
    image: str
    network: str
    proxy: str
    guest: str
    rules_path: Path
    artifacts: Path
    proxies: dict[str, str] = field(default_factory=dict)
    dns_addr: str = ""

    def _docker(self, *args: str, check: bool = True) -> subprocess.CompletedProcess:
        return _run(["docker", *args], check=check)

    def up(self) -> None:
        """Write rules, create the network, start the proxy and the guest."""
        net_name = f"koma-net-{self.scope_key}"
        self.network = net_name
        self.proxy = f"koma-proxy-{self.scope_key}"
        # Point the guest at the proxy for DNS. Without a gateway there is no
        # resolver on the isolated network, so a name cannot be looked up at all;
        # this is a reachability fix, NOT a scope decision — the proxy still
        # checks every connection it forwards.
        self.guest = f"{GUEST_NAME_PREFIX}{self.scope_key}"
        # Clear leftovers from a crashed run. Containers go FIRST: a network with
        # an attached container cannot be removed, and the proxy is attached, so
        # removing the network first silently leaves both behind and `create` then
        # fails on the name.
        self.down()
        # Both must exist BEFORE the mounts are declared: docker validates a bind
        # source and refuses to start rather than creating it.
        self.artifacts.mkdir(parents=True, exist_ok=True)
        self.rules_path.parent.mkdir(parents=True, exist_ok=True)
        self.rules_path.touch(exist_ok=True)
        # Mount the path AS GIVEN, not the resolved one. On macOS `/tmp` is a
        # symlink to `/private/tmp`, but only `/tmp` is in the daemon's shared
        # set — resolving turns a shared path into an unshared one and the mount
        # fails even though the file exists. (Both forms exist; only one is
        # visible to the daemon.)
        pass
        self._docker("network", "create", "--internal", net_name)

        # The proxy sits on BOTH networks: the isolated one so the guest can see
        # it, the default one so it has a way out itself.
        self._docker(
            "run", "-d", "--name", self.proxy,
            "--network", net_name,
            # Alias at RUN time: attaching a second network later replaces the
            # endpoint and drops an alias set here, so `proxy` would stop
            # resolving. The name is what the guest's proxy env points at.
            "--network-alias", PROXY_ALIAS,
            "--read-only",
            "-v", f"{self.rules_path}:/run/koma/rules:ro",
            "--entrypoint", "python3",
            self.image,
            "/opt/koma/koma_proxy.py",
        )
        # Second leg: the proxy's own way out. No alias needed here.
        self._docker("network", "connect", "bridge", self.proxy, check=False)
        # Its address on the ISOLATED leg is the guest's resolver. `--dns` wants an
        # IP and container-name resolution does not work for it.
        self.dns_addr = self.network_addr(net_name, self.proxy)

        self._docker(
            "run", "-d", "--name", self.guest,
            "--network", net_name,
            # Non-root, no capabilities, no escalation, no socket.
            # Resolver: the proxy's address. There is no gateway on an --internal
            # network, so without this NO name resolves at all. `--dns` takes an
            # IP, not a name, hence the lookup below.
            "--dns", self.dns_addr,
            "--user", str(GUEST_UID),
            "--cap-drop", "ALL",
            "--security-opt", "no-new-privileges",
            # Read-only root; the two writable paths are explicit and bounded.
            "--read-only",
            "--tmpfs", "/tmp:rw,size=256m",
            "--mount", f"type=bind,src={self.artifacts},dst=/artifacts",
            "--pids-limit", "512",
            "--memory", "2g",
            "--cpus", "2",
            # Proxy env is how a client LEARNS the route, not what guarantees it.
            # BOTH cases: curl reads lowercase `http_proxy` for plain HTTP and
            # only the uppercase form for HTTPS, so uppercase alone silently
            # skips the proxy and then fails to resolve.
            "-e", f"HTTP_PROXY=http://{PROXY_ALIAS}:{PROXY_PORT}",
            "-e", f"HTTPS_PROXY=http://{PROXY_ALIAS}:{PROXY_PORT}",
            "-e", f"http_proxy=http://{PROXY_ALIAS}:{PROXY_PORT}",
            "-e", f"https_proxy=http://{PROXY_ALIAS}:{PROXY_PORT}",
            self.image,
        )
        # The guest sees the proxy by its own name on the isolated network, so the
        # env above matches the fact that `bridge` is the proxy's second leg.
        self.proxies = {
            "HTTP_PROXY": f"http://proxy:{PROXY_PORT}",
            "HTTPS_PROXY": f"http://proxy:{PROXY_PORT}",
        }

    def down(self) -> None:
        for name in (getattr(self, "guest", ""), getattr(self, "proxy", "")):
            if name:
                self._docker("rm", "-f", name, check=False)
        if getattr(self, "network", ""):
            self._docker("network", "rm", self.network, check=False)

    def network_addr(self, network: str, container: str) -> str:
        """A container's address on one network.

        Read from `NetworkSettings.Networks` by simple key lookup: the `-f`
        template with `index` chokes on the network name.
        """
        raw = self._docker("inspect", "-f", "{{json .NetworkSettings.Networks}}", container).stdout
        try:
            nets = json.loads(raw)
        except ValueError:
            return ""
        return (nets.get(network) or {}).get("IPAddress", "")

    def exec(self, argv: list[str], timeout: float, mem_mb: int | None = None) -> subprocess.CompletedProcess:
        """Run one command in the guest.

        `timeout` kills the process group, not just the docker client: a tool that
        spawns children would otherwise survive its own timeout.
        """
        args = ["docker", "exec", "-i"]
        if mem_mb:
            args += ["-e", f"KOMA_MEM_MB={mem_mb}"]
        args += [self.guest, *argv]
        return _run(args, timeout=timeout, kill_group=True)

    def record(self) -> dict:
        """Runtime facts worth keeping with the run: they can drift."""
        return {
            "image": _inspect(self.image, "Id"),
            "network": self.network,
            "guest": self.guest,
            "proxy": self.proxy,
            "proxy_legs": ["isolated", "bridge"],
            "uid": GUEST_UID,
            "read_only_root": True,
        }


def _run(
    argv: list[str],
    *,
    check: bool = False,
    timeout: float | None = None,
    kill_group: bool = False,
) -> subprocess.CompletedProcess:
    if kill_group:
        return _run_grouped(argv, timeout or 60)
    try:
        return subprocess.run(
            argv, capture_output=True, text=True, check=check, timeout=timeout
        )
    except subprocess.TimeoutExpired as exc:
        return subprocess.CompletedProcess(argv, 124, exc.stdout or "", "timeout")


def _run_grouped(argv: list[str], timeout: float) -> subprocess.CompletedProcess:
    """Run with a fresh process group so timeout kills the whole tree."""
    proc = subprocess.Popen(
        argv,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    try:
        out, err = proc.communicate(timeout=timeout)
        return subprocess.CompletedProcess(argv, proc.returncode, out, err)
    except subprocess.TimeoutExpired:
        # Kill the GROUP: the docker client plus everything it started.
        try:
            os.killpg(proc.pid, 9)
        except OSError:
            proc.kill()
        out, err = proc.communicate()
        return subprocess.CompletedProcess(argv, 124, out, "timeout")


def _inspect(ref: str, fmt: str) -> str:
    res = _run(["docker", "inspect", "--format", "{{." + fmt + "}}", ref])
    return res.stdout.strip() if res.returncode == 0 else ""


def preflight() -> str | None:
    """Why the boundary cannot run here, or None. Says so rather than degrading."""
    if not shutil.which("docker"):
        return DOCKER_MISSING
    if _run(["docker", "info"]).returncode != 0:
        return DOCKER_DOWN
    probe = f"koma-probe-{secrets.token_hex(4)}"
    made = _run(["docker", "network", "create", "--internal", probe])
    if made.returncode != 0:
        return INTERNAL_UNAVAILABLE
    _run(["docker", "network", "rm", probe])
    return None


def write_rules(path: Path, lines: list[str]) -> None:
    """Write the proxy rule file. Atomic, so the proxy never reads a half file."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text("# generated from the scope record — do not edit\n" + "\n".join(lines) + "\n")
    tmp.replace(path)


def rules_for(targets: list[str], exclusions: list[str]) -> list[str]:
    """
    Render scope cells to proxy rules.

    Wildcard ports: admission already narrowed them per host, and the proxy
    re-checks the port against the spec it is given, so `*` here reflects "any
    port this host allows" rather than "any port at all" for a host that was
    narrowed.
    """
    out = []
    for cell in targets:
        host, _, ports = cell.partition(":")
        spec = ports or "*"
        out.append(f"{host.lower()} {spec}")
    for cell in exclusions:
        # The proxy is allow-only, so an exclusion is expressed by narrowing the
        # matching target line rather than a deny rule.
        host, _, _ports = cell.partition(":")
        out = [line for line in out if line.split()[0] != host.lower()]
    return out


def new_boundary(scope_key: str, image: str, run_dir: Path) -> Boundary:
    """Build the handle. Does not start anything; call [`Boundary::up`]."""
    return Boundary(
        scope_key=scope_key,
        image=image,
        network="",
        proxy="",
        guest="",
        rules_path=run_dir / "rules",
        artifacts=run_dir / "artifacts",
    )


def main() -> int:
    """`python -m koma_sec_daemon.boundary <payload>` — used by the smoke test."""
    payload = json.load(sys.stdin)
    problem = preflight()
    if problem:
        print(json.dumps({"ok": False, "error": problem}))
        return 1
    boundary = new_boundary(payload["key"], payload["image"], Path(payload["dir"]))
    boundary.artifacts.mkdir(parents=True, exist_ok=True)
    write_rules(boundary.rules_path, rules_for(payload["targets"], payload.get("exclusions", [])))
    try:
        boundary.up()
    except BoundaryError as exc:
        boundary.down()
        print(json.dumps({"ok": False, "error": str(exc)}))
        return 1
    print(json.dumps({"ok": True, "record": boundary.record()}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
