"""End-to-end smoke of the config methods against the real adapter (`koma sdk serve`).

Usage: smoke_config.py [path/to/koma]   (default: target/debug/koma under the repository root)

The binary must be a debug build: the smoke isolates koma under a fresh temporary
data dir through KOMA_TEST_DATA_DIR, which only debug builds honour. The provider is
a fake endpoint, so no model is called, and the update check and telemetry are off.
The session daemon would refresh koma's public model catalogues at start; the smoke
seeds a fresh, empty catalogue cache (the binary's bundled default is empty too) and
points the OAuth catalogue and every HTTP proxy at a closed loopback port, so nothing
leaves the machine, and it fails if the daemon's log names a public catalogue host.
Every frame the adapter writes is checked against protocol/schema.json when
jsonschema is importable, and the fake key must never appear in adapter output.
Prints one line per step and exits non-zero on the first failed expectation.
"""
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SCHEMA = HERE.parent / 'protocol' / 'schema.json'
KEY = 'sk-smoke-0000000000'
ENDPOINT = 'http://127.0.0.1:9/v1'
PROVIDER = 'smoke'
MODEL_ID = 'smoke-model'
V1 = ['create', 'resume', 'state', 'submit', 'enqueue', 'interrupt', 'interrupt_and_submit', 'approve',
      'configure', 'detach']
CONFIG_METHODS = ['config.get', 'config.providers.delete', 'config.models.delete', 'config.roles.unassign',
                  'config.effort.get', 'config.effort.set', 'config.limits.get', 'config.limits.set']
# Port 9 (discard) on loopback: connections are refused and never leave the machine.
CLOSED = 'http://127.0.0.1:9'
# Hosts of koma's public catalogues; the daemon's log must never name them.
CATALOGUE_HOSTS = ('github.com', 'koma.run')
REPLY_TIMEOUT = 30.0
EXIT_TIMEOUT = 15.0


class SmokeFailure(Exception):
    pass


def expect(condition, what):
    if not condition:
        raise SmokeFailure(what)


def step(text):
    print(f'ok   {text}', flush=True)


def validator():
    try:
        import jsonschema
    except ImportError:
        return None
    return jsonschema.Draft202012Validator(json.loads(SCHEMA.read_text()))


def short_temp_dir():
    # Unix socket paths are limited to about 100 bytes, and the per-session socket
    # lives at <data dir>/run/<uuid>.sock, so prefer the short /tmp over $TMPDIR.
    base = '/tmp' if os.name == 'posix' and os.path.isdir('/tmp') else None
    return tempfile.mkdtemp(prefix='ks-', dir=base)


class Adapter:
    """One `koma sdk serve` process spoken to in raw JSON lines."""

    def __init__(self, name, binary, env, cwd, schema):
        self.name = name
        self.binary = binary
        self.env = env
        self.schema = schema
        self.seq = 0
        self.frames = queue.Queue()
        self.stdout = []
        self.stderr = []
        self.events = []
        self.proc = subprocess.Popen(
            [binary, 'sdk', 'serve'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, env=env, cwd=cwd)
        threading.Thread(target=self._read_stdout, daemon=True).start()
        threading.Thread(target=self._read_stderr, daemon=True).start()
        self.hello = self._next_frame(REPLY_TIMEOUT)
        expect(self.hello.get('type') == 'hello', f'{name}: first frame is hello, got {self.hello}')

    def _read_stdout(self):
        assert self.proc.stdout is not None
        for raw in self.proc.stdout:
            line = raw.decode('utf-8', errors='replace')
            self.stdout.append(line)
            self.frames.put(line)
        self.frames.put(None)

    def _read_stderr(self):
        assert self.proc.stderr is not None
        for raw in self.proc.stderr:
            self.stderr.append(raw.decode('utf-8', errors='replace'))

    def _next_frame(self, timeout):
        try:
            line = self.frames.get(timeout=timeout)
        except queue.Empty:
            raise SmokeFailure(f'{self.name}: no frame within {timeout:.0f} s')
        if line is None:
            raise SmokeFailure(f'{self.name}: stdout closed (exit {self.proc.poll()}); '
                               f'stderr tail: {"".join(self.stderr)[-2000:]}')
        expect(KEY not in line, f'{self.name}: the provider key appears in adapter output')
        expect(line.endswith('\n') and line.strip() == line.rstrip('\r\n'),
               f'{self.name}: frame is one JSON line: {line!r}')
        try:
            frame = json.loads(line)
        except json.JSONDecodeError as error:
            raise SmokeFailure(f'{self.name}: stdout line is not JSON ({error}): {line!r}')
        if self.schema is not None:
            errors = sorted(self.schema.iter_errors(frame), key=lambda e: list(e.path))
            expect(not errors, f'{self.name}: frame does not match schema.json: '
                               f'{errors[0].message if errors else ""}: {line.strip()}')
        return frame

    def call(self, method, **params):
        self.seq += 1
        rid = f'{self.name}-{self.seq}'
        request = json.dumps({'id': rid, 'method': method, 'params': params}) + '\n'
        assert self.proc.stdin is not None
        self.proc.stdin.write(request.encode())
        self.proc.stdin.flush()
        deadline = time.monotonic() + REPLY_TIMEOUT
        while True:
            frame = self._next_frame(max(0.1, deadline - time.monotonic()))
            if frame.get('type') == 'event':
                self.events.append(frame)
                continue
            expect(frame.get('type') == 'response', f'{self.name}: unexpected frame {frame}')
            expect(frame.get('id') == rid, f'{self.name}: response to {frame.get("id")}, expected {rid}')
            return frame

    def result(self, method, **params):
        frame = self.call(method, **params)
        expect('result' in frame, f'{self.name}: {method} failed: {frame.get("error")}')
        return frame['result']

    def error(self, method, **params):
        frame = self.call(method, **params)
        expect('error' in frame, f'{self.name}: {method} should fail, got {frame.get("result")}')
        return frame['error']

    def close_stdin(self):
        if self.proc.stdin is not None and not self.proc.stdin.closed:
            try:
                self.proc.stdin.close()
            except OSError:
                pass

    def wait_exit(self):
        self.close_stdin()
        try:
            code = self.proc.wait(timeout=EXIT_TIMEOUT)
        except subprocess.TimeoutExpired:
            raise SmokeFailure(f'{self.name}: adapter did not exit after stdin EOF')
        # Check what is left on stdout too, so the secret and schema checks see every frame.
        while True:
            try:
                line = self.frames.get(timeout=EXIT_TIMEOUT)
            except queue.Empty:
                raise SmokeFailure(f'{self.name}: stdout still open after exit')
            if line is None:
                break
            self.frames.put(line)
            frame = self._next_frame(0.1)
            if frame.get('type') == 'event':
                self.events.append(frame)
        return code

    def kill(self):
        if self.proc.poll() is None:
            self.close_stdin()
            try:
                self.proc.wait(timeout=EXIT_TIMEOUT)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()

    def leaked(self):
        return KEY in ''.join(self.stdout) or KEY in ''.join(self.stderr)


def global_view(adapter):
    view = adapter.result('config.get', view='global')
    expect(view['view'] == 'global', f'config.get view is global: {view["view"]}')
    return view


def role(view, name):
    return next(r for r in view['roles'] if r['role'] == name)


def check_hello(adapter):
    hello = adapter.hello
    expect(hello['capabilities'] == V1, f'hello.capabilities is the ten v1 names: {hello["capabilities"]}')
    methods = hello.get('methods', [])
    missing = [m for m in V1 + CONFIG_METHODS + ['sessions.info'] if m not in methods]
    expect(not missing, f'hello.methods lists the config methods; missing {missing}')
    for feature in ('errors.reason', 'state.session_info', 'config.lock'):
        expect(feature in hello.get('features', []), f'hello.features has {feature}')


def run_unbound(adapter):
    check_hello(adapter)
    step(f'{adapter.name}: hello advertises the config methods and features')

    view = global_view(adapter)
    expect(view['file'] == 'missing' and view['revision'] == 'absent',
           f'fresh data dir has no config.json: file={view["file"]} revision={view["revision"]}')
    expect(view['providers'] == [] and view['models'] == [], 'fresh view is empty')
    step('1 unbound config.get: file "missing", revision "absent"')

    adapter.result('configure', providers=[{
        'name': PROVIDER, 'endpoint': ENDPOINT, 'api_type': 'openai', 'api_key': KEY}],
        models={'main': {'provider': PROVIDER, 'model_id': MODEL_ID}})
    view = global_view(adapter)
    expect(view['file'] == 'ok' and view['revision'] != 'absent', 'config.json exists after configure')
    providers = [p for p in view['providers'] if p['name'] == PROVIDER]
    expect(len(providers) == 1, f'one provider named {PROVIDER}: {view["providers"]}')
    provider = providers[0]
    expect(provider['key_state'] == 'set' and provider['has_key'], f'key is set: {provider}')
    expect(provider['endpoint'] == ENDPOINT and not provider['endpoint_redacted'],
           f'endpoint is shown unredacted: {provider["endpoint"]} redacted={provider["endpoint_redacted"]}')
    main = role(view, 'main')
    expect(main['origin'] == 'global' and main['model_uuid'] is not None, f'main is assigned globally: {main}')
    expect(view['main_fallback'] is None, f'main has no fallback: {view["main_fallback"]}')
    step('2 configure + config.get: key_state "set", endpoint not redacted, main assigned')

    result = adapter.result('config.roles.unassign', role='planner')
    expect(result['changed'] is False and result['previous_model_uuid'] is None,
           f'planner was not held: {result}')
    expect(result['config']['revision'] == view['revision'], 'unassigning an unheld role writes nothing')
    step('3 config.roles.unassign planner: changed false, revision unchanged')

    result = adapter.result('config.roles.unassign', role='main')
    expect(result['changed'] is True and result['previous_model_uuid'] == main['model_uuid'],
           f'main was released: {result}')
    view = global_view(adapter)
    expect(view['main_fallback'] == 'unconfigured', f'main_fallback is "unconfigured": {view["main_fallback"]}')
    step('4 config.roles.unassign main + config.get: main_fallback "unconfigured"')

    adapter.result('configure', models={'main': {'provider': PROVIDER, 'model_id': MODEL_ID}})
    view = global_view(adapter)
    expect(view['main_fallback'] is None and role(view, 'main')['origin'] == 'global',
           f'main is assigned again: {role(view, "main")}')
    provider = next(p for p in view['providers'] if p['name'] == PROVIDER)
    expect(provider['key_state'] == 'set', 'configure without api_key keeps the stored key')
    step('5 configure main again: main assigned, key kept')

    before = view['revision']
    result = adapter.result('config.providers.delete', provider_uuid=provider['provider_uuid'], dry_run=True)
    expect(result['dry_run'] is True, 'dry_run echoed')
    expect(result['removed']['provider_uuid'] == provider['provider_uuid'], f'dry run names the provider: {result}')
    expect(result['removed']['models_removed'], f'dry run lists the models it would remove: {result["removed"]}')
    expect(result['config']['revision'] == before, 'dry run result carries the current revision')
    after = global_view(adapter)
    expect(after['revision'] == before, f'dry run wrote nothing: {before} -> {after["revision"]}')
    expect(any(p['provider_uuid'] == provider['provider_uuid'] for p in after['providers']),
           'provider still present after dry run')
    step('6 config.providers.delete dry_run: models_removed non-empty, revision unchanged')

    stale = '0000000000000000' if before != '0000000000000000' else 'ffffffffffffffff'
    error = adapter.error('config.providers.delete', provider_uuid=provider['provider_uuid'], if_revision=stale)
    expect((error['code'], error.get('reason'), error['delivery']) == ('invalid_state', 'stale', 'not_sent'),
           f'stale if_revision is refused: {error}')
    after = global_view(adapter)
    expect(after['revision'] == before and any(p['provider_uuid'] == provider['provider_uuid']
                                              for p in after['providers']),
           'stale delete removed nothing')
    step('7 config.providers.delete with a stale if_revision: invalid_state/stale, not_sent')

    code = adapter.wait_exit()
    expect(code == 0, f'{adapter.name}: exit 0 on stdin EOF, got {code}')
    step(f'{adapter.name}: stdin EOF ends the unbound adapter with exit 0')


def run_coding(adapter, workdir, sessions):
    check_hello(adapter)
    created = adapter.result('create', cwd=workdir)
    session_id = created['state']['session_id']
    sessions.append(session_id)
    info = created.get('session_info')
    expect(isinstance(info, dict), f'create result carries session_info: {sorted(created)}')
    limits = info['limits']
    expect(isinstance(limits['max_output_tokens'], int), f'session_info.limits from create: {limits}')
    step(f'8a {adapter.name}: create bound session {session_id}; session_info.limits {limits}')

    target = 4096 if limits['max_output_tokens'] != 4096 else 8192
    result = adapter.result('config.limits.set', max_output_tokens=target)
    expect(result['max_output_tokens'] == target, f'config.limits.set read back {target}: {result}')
    step(f'8b config.limits.set max_output_tokens {target}: read back')

    revision = created['state']['revision']
    state = adapter.result('state')
    expect(state['session_info']['limits']['max_output_tokens'] == target,
           f'state session_info.limits shows {target}: {state["session_info"]["limits"]}')
    step(f'8c state: session_info.limits {target} (state.revision {revision} -> {state["state"]["revision"]})')

    info = adapter.result('sessions.info')['session_info']
    expect(info['limits']['max_output_tokens'] == target, f'sessions.info shows {target}: {info["limits"]}')
    step(f'8d sessions.info: limits {target}')

    effort = adapter.result('config.effort.get', wait_ms=0)
    expect(effort['state'] in ('ready', 'loading', 'unsupported'), f'effort state: {effort}')
    expect(effort['options'][:1] == ['default'], f'effort options start with "default": {effort["options"]}')
    step(f'8e config.effort.get wait_ms 0: state "{effort["state"]}"')

    error = adapter.error('room.status')
    if 'room.status' in adapter.hello.get('methods', []):
        expect((error['code'], error.get('reason'), error.get('binding'), error.get('allowed'))
               == ('invalid_state', 'wrong_binding', 'coding', ['room_live', 'room_closed']),
               f'room.status in coding is refused for its binding: {error}')
        step('8f room.status: invalid_state/wrong_binding, allowed room_live, room_closed')
    else:
        expect((error['code'], error['delivery']) == ('unsupported_method', 'not_sent'),
               f'room.status is not advertised, so it is unsupported_method: {error}')
        step('8f room.status: not in hello.methods, unsupported_method')

    expect(adapter.result('detach') == {'detached': True}, 'detach returns {detached:true}')
    code = adapter.wait_exit()
    expect(code == 0, f'{adapter.name}: exit 0 after detach, got {code}')
    step(f'8g detach: {{detached:true}}, exit 0')
    for event in adapter.events:
        expect('session_info' not in event, f'events never carry session_info: {event}')


def stop_daemons(binary, env, data_dir, sessions):
    """Stop each session daemon this smoke started with `koma daemon kill --session`,
    which is scoped to the data dir in env and leaves every other process alone."""
    failures = []
    for session_id in sessions:
        subprocess.run([binary, 'daemon', 'kill', '--session', session_id], env=env,
                       cwd=data_dir, stdin=subprocess.DEVNULL, capture_output=True,
                       timeout=60)
        if os.name == 'posix':
            sock = Path(data_dir) / 'run' / f'{session_id}.sock'
            deadline = time.monotonic() + EXIT_TIMEOUT
            while sock.exists() and time.monotonic() < deadline:
                time.sleep(0.1)
            if sock.exists():
                failures.append(session_id)
    status = subprocess.run([binary, 'daemon', 'status'], env=env, cwd=data_dir,
                            stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=60)
    return failures, status.stdout.strip()


def isolate_catalogues(data_dir, env):
    """Keep the session daemon's catalogue refresh off the network. A catalogue
    cache younger than its TTL is not refreshed, and the OAuth catalogue base and
    the HTTP proxies point at a closed loopback port."""
    (Path(data_dir) / 'models.json').write_text('{}')
    env['KOMA_OA_BASE'] = CLOSED
    for name in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'http_proxy', 'https_proxy', 'all_proxy'):
        env[name] = CLOSED
    for name in ('NO_PROXY', 'no_proxy'):
        env.pop(name, None)


def catalogue_fetches(data_dir):
    """Lines of koma's logs under data_dir that name a public catalogue host."""
    hits = []
    for log in Path(data_dir).rglob('*.log'):
        try:
            text = log.read_text(errors='replace')
        except OSError:
            continue
        hits.extend(line for line in text.splitlines() if any(host in line for host in CATALOGUE_HOSTS))
    return hits


def main(argv):
    binary = Path(argv[1]) if len(argv) > 1 else ROOT / 'target' / 'debug' / 'koma'
    if os.name == 'nt' and not binary.suffix and binary.with_suffix('.exe').exists():
        binary = binary.with_suffix('.exe')
    binary = binary.resolve()
    if not binary.is_file():
        print(f'FAIL koma binary not found: {binary}', file=sys.stderr)
        return 2
    schema = validator()
    if schema is None:
        print('note jsonschema is not importable; frames are not validated against schema.json', flush=True)

    data_dir = short_temp_dir()
    workdir = os.path.join(data_dir, 'work')
    os.mkdir(workdir)
    # Inherited KOMA_* settings could point koma elsewhere; the smoke sets its own.
    env = {k: v for k, v in os.environ.items() if not k.startswith('KOMA_')}
    env.update({
        'KOMA_TEST_DATA_DIR': data_dir,
        'KOMA_TELEMETRY': '0',
        'KOMA_TEST_OFFLINE': '1',
    })
    isolate_catalogues(data_dir, env)
    adapters = []
    sessions = []
    code = 0
    try:
        print(f'note binary {binary}', flush=True)
        print(f'note data dir {data_dir}', flush=True)
        first = Adapter('unbound', str(binary), env, workdir, schema)
        adapters.append(first)
        run_unbound(first)
        second = Adapter('coding', str(binary), env, workdir, schema)
        adapters.append(second)
        run_coding(second, workdir, sessions)
    except SmokeFailure as failure:
        print(f'FAIL {failure}', flush=True)
        code = 1
    finally:
        for adapter in adapters:
            adapter.kill()
        failures, status = stop_daemons(str(binary), env, data_dir, sessions)
        if failures:
            print(f'FAIL session daemon still running after kill: {failures}', flush=True)
            code = code or 1
        elif sessions:
            step(f'session daemon stopped ({status.splitlines()[0] if status else "no status"})')
        fetches = catalogue_fetches(data_dir)
        if fetches:
            print(f'FAIL koma reached a public catalogue: {fetches[:3]}', flush=True)
            code = code or 1
        for adapter in adapters:
            if adapter.leaked():
                print(f'FAIL {adapter.name}: the provider key appears in adapter output', flush=True)
                code = code or 1
        shutil.rmtree(data_dir, ignore_errors=True)
    if code == 0:
        step('smoke passed; no frame carried the provider key')
    return code


if __name__ == '__main__':
    sys.exit(main(sys.argv))
