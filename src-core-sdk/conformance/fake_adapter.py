#!/usr/bin/env python3
"""Deterministic, dependency-free JSONL peer. Never contacts Koma."""
import argparse
import copy
import hashlib
import json
import re
import sys
import os
import time
from urllib.parse import urlsplit, urlunsplit

MAX_LINE = 40 * 1024 * 1024
V1_METHODS = ['create', 'resume', 'state', 'submit', 'enqueue', 'interrupt',
              'interrupt_and_submit', 'approve', 'configure', 'detach']
CONFIG_METHODS = ['config.get', 'config.providers.delete', 'config.models.delete', 'config.roles.unassign',
                  'config.effort.get', 'config.effort.set', 'config.limits.get', 'config.limits.set']
METHODS = V1_METHODS + CONFIG_METHODS + ['sessions.info']
FEATURES = ['errors.reason', 'state.session_info', 'config.lock']
EVENTS = ['state', 'output_append', 'turn']
LIMITS = dict(frame_bytes=MAX_LINE, request_ids=65536, text_bytes=2097152,
              attachments=dict(count=16, pasted_bytes=2097152, total_bytes=33554432),
              config=dict(lock_wait_ms=3000), time=dict(ipc_ms=15000))
LEGACY_SESSION = ['state', 'submit', 'enqueue', 'interrupt', 'interrupt_and_submit', 'approve']
CODING_ONLY = ['config.effort.get', 'config.effort.set', 'config.limits.get', 'config.limits.set', 'sessions.info']
ANY_BINDING = ['unbound', 'coding', 'room_live', 'room_closed']
PUBLIC_ROLES = ['main', 'awareness', 'safeguard', 'compactor', 'planner']
EFFORT_OPTIONS = ['default', 'low', 'medium', 'high']
CONTROL = re.compile('[\x00-\x1f\x7f]')

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--scenario', choices=['normal', 'reject', 'partial', 'unknown',
    'eof', 'malformed', 'wrong_id', 'out_of_order', 'event_flood', 'approval',
    'bad_version', 'stderr', 'output', 'timeout', 'hidden', 'fragmented', 'missing_cap', 'output_gap', 'bad_offset', 'finish', 'bad_result', 'flood_finish', 'flood_output',
    'config', 'wrong_binding', 'legacy_hello'], default=os.environ.get('KOMA_FAKE_SCENARIO', 'normal'))
a = p.parse_args([] if sys.argv[1:] == ['sdk', 'serve'] else None)
LEGACY = a.scenario == 'legacy_hello'

def emit(value):
    raw = (json.dumps(value, ensure_ascii=False, separators=(',', ':')) + '\n').encode()
    if a.scenario == 'fragmented':
        for b in raw:
            sys.stdout.buffer.write(bytes([b])); sys.stdout.buffer.flush()
    else:
        sys.stdout.buffer.write(raw); sys.stdout.buffer.flush()

def without_detach(names):
    return [m for m in names if m != 'detach'] if a.scenario == 'missing_cap' else list(names)

hello = dict(type='hello', protocol=2 if a.scenario == 'bad_version' else 1,
             binary=dict(path='/fake/koma', version='0.0.0-fake'),
             diagnostics=['conformance fake; no daemon'], capabilities=without_detach(V1_METHODS))
if not LEGACY:
    hello.update(methods=without_detach(METHODS), features=list(FEATURES), events=list(EVENTS),
                 limits=copy.deepcopy(LIMITS))
emit(hello)
if a.scenario == 'stderr':
    sys.stderr.write('fake diagnostic\n' * 8192)
    sys.stderr.flush()
session = None
binding = 'unbound'
seq = 0
counter = 0
held = None

# In-memory stand-in for config.json. Secret values live here and are never projected.
if a.scenario == 'config':
    catalogue = dict(file='ok', providers=[
        dict(provider_uuid='prov-local', name='local', api_type='openai',
             endpoint='https://user:fake-pass@llm.example.test/v1?token=fake-query#frag',
             api_key='sk-fake-planted-0000000000', ext_id=None),
        dict(provider_uuid='prov-ext', name='extension', api_type='anthropic',
             endpoint='http://127.0.0.1:9/v1', api_key='', ext_id='ext.fake'),
    ], oauth=[
        dict(oauth_uuid='oauth-1', name='signed-in', provider='anthropic', token='fake-oauth-token-0000', expires_at=0),
    ], models=[
        dict(model_uuid='model-main', name='Main model', model_id='fake/main', provider_uuid='prov-local',
             route=None, roles=['main'], source_uuid=None),
        dict(model_uuid='model-planner', name='Planner', model_id='fake/planner', provider_uuid='prov-local',
             route=None, roles=['planner', 'cyber-red'], source_uuid=None),
        dict(model_uuid='model-ext', name='Extension model', model_id='fake/ext', provider_uuid='prov-ext',
             route=None, roles=[], source_uuid=None),
        dict(model_uuid='model-oauth', name='OAuth model', model_id='fake/oauth', provider_uuid='oauth-1',
             route='messages', roles=['awareness'], source_uuid=None),
        dict(model_uuid='model-orphan', name='Orphan', model_id='fake/orphan', provider_uuid='prov-gone',
             route=None, roles=[], source_uuid='model-main'),
        dict(model_uuid='model-unverified', name='Unverified delete', model_id='fake/unverified',
             provider_uuid='prov-ext', route=None, roles=[], source_uuid=None),
    ])
else:
    catalogue = dict(file='missing', providers=[], oauth=[], models=[])
effort_ready = set()
info_clock = 0

def event():
    global seq
    seq += 1
    emit(dict(type='event', event='state', seq=seq, state=copy.deepcopy(session['state']), resynced=False))

def response(rid, result=None, code=None, delivery='not_sent'):
    frame = dict(type='response', id=rid)
    if code:
        frame['error'] = dict(code=code, message='fake ' + code, delivery=delivery)
    else:
        frame['result'] = result
    return frame

def failure(rid, code, reason, message, delivery='not_sent', **extra):
    error = dict(code=code, message=message, delivery=delivery, reason=reason)
    error.update(extra)
    return dict(type='response', id=rid, error=error)

def invalid(rid, reason, message):
    return failure(rid, 'invalid_request', reason, message)

def revision():
    if catalogue['file'] == 'missing':
        return 'absent'
    raw = json.dumps(catalogue, sort_keys=True, separators=(',', ':')).encode()
    return hashlib.sha256(raw).hexdigest()[:16]

def strip_endpoint(endpoint):
    parts = urlsplit(endpoint)
    if parts.scheme and parts.netloc:
        clean = urlunsplit((parts.scheme, parts.netloc.rpartition('@')[2], parts.path, '', ''))
    else:
        clean = re.sub(r'^([A-Za-z][A-Za-z0-9+.-]*://)[^/@]*@', r'\1', endpoint)
        clean = re.split('[?#]', clean, maxsplit=1)[0]
    return clean, clean != endpoint

def lookup_provider(uuid):
    for item in catalogue['providers']:
        if item['provider_uuid'] == uuid:
            return 'provider', item
    for item in catalogue['oauth']:
        if item['oauth_uuid'] == uuid:
            return 'oauth', item
    return 'missing', None

def find_model(uuid):
    return next((m for m in catalogue['models'] if m['model_uuid'] == uuid), None)

def holder(role):
    return next((m for m in catalogue['models'] if role in m['roles']), None)

def route(role, origin, model):
    kind, prov = lookup_provider(model['provider_uuid'])
    return dict(role=role, origin=origin, model_uuid=model['model_uuid'], source_uuid=model['source_uuid'],
                configured_model=model['model_id'], provider_uuid=model['provider_uuid'],
                provider_name=prov['name'] if prov else None,
                effective_model=model['model_id'] if prov else None,
                reason=None if prov else 'provider_missing')

def roles_view(view):
    pinned = find_model(session['main_pin']) if view == 'session' and session['main_pin'] else None
    main = pinned or holder('main')
    routes = []
    for role in PUBLIC_ROLES:
        model = main if role == 'main' else holder(role)
        if role == 'main' and model is not None:
            routes.append(route(role, 'session' if pinned else 'global', model))
        elif model is not None:
            routes.append(route(role, 'global', model))
        elif role not in ('main', 'safeguard') and main is not None and route(role, 'x', main)['reason'] is None:
            # Unassigned roles other than safeguard use Main's route. Safeguard
            # fails closed and never inherits Main.
            inherited = route(role, 'inherit_main', main)
            inherited.update(model_uuid=None, source_uuid=None)
            routes.append(inherited)
        else:
            routes.append(dict(role=role, origin='default', model_uuid=None, source_uuid=None, configured_model=None,
                               provider_uuid=None, provider_name=None, effective_model=None, reason='unassigned'))
    return routes, main

def main_fallback(main):
    if main is None:
        return 'unconfigured'
    kind, prov = lookup_provider(main['provider_uuid'])
    if prov is None:
        return 'provider_removed'
    if kind == 'provider' and not prov['api_key']:
        return 'no_key'
    if kind == 'oauth' and not prov['token']:
        return 'not_signed_in'
    return None

def config_view(view):
    providers = []
    for item in catalogue['providers']:
        endpoint, redacted = strip_endpoint(item['endpoint'])
        providers.append(dict(provider_uuid=item['provider_uuid'], name=item['name'], api_type=item['api_type'],
            endpoint=endpoint, endpoint_redacted=redacted, managed=item['ext_id'] is not None,
            has_key=bool(item['api_key']), key_state='set' if item['api_key'] else 'missing',
            model_count=sum(1 for m in catalogue['models'] if m['provider_uuid'] == item['provider_uuid'])))
    oauth = [dict(oauth_uuid=o['oauth_uuid'], name=o['name'], provider=o['provider'], has_token=bool(o['token']),
                  expires_at=o['expires_at']) for o in catalogue['oauth']]
    models = []
    for m in catalogue['models']:
        kind, prov = lookup_provider(m['provider_uuid'])
        models.append(dict(model_uuid=m['model_uuid'], name=m['name'], model_id=m['model_id'],
            provider_uuid=m['provider_uuid'], provider_kind=kind, provider_name=prov['name'] if prov else None,
            route=m['route'], roles=[r for r in PUBLIC_ROLES if r in m['roles']],
            other_roles=any(r not in PUBLIC_ROLES for r in m['roles']), source_uuid=m['source_uuid']))
    roles, main = roles_view(view)
    return dict(revision=revision(), file=catalogue['file'], view=view,
                providers=providers, providers_truncated=False, oauth=oauth, oauth_truncated=False,
                models=models, models_truncated=False, roles=roles, main_fallback=main_fallback(main))

def main_uuid():
    _, main = roles_view('session')
    return main['model_uuid'] if main else None

def touch():
    global info_clock
    info_clock = max(int(time.time() * 1000), info_clock + 1)
    session['observed_at_ms'] = info_clock

def session_info():
    routes, _ = roles_view('session')
    return dict(name=session['name'], effort=session['effort'], routes=routes, limits=dict(session['limits']),
                usage=None, daemon=dict(version='0.0.0-fake', ipc_protocol=1, capabilities=['fake']),
                observed_at_ms=session['observed_at_ms'])

def state_result():
    result = dict(state=copy.deepcopy(session['state']))
    if not LEGACY:
        result['session_info'] = session_info()
    return result

def with_info(result):
    if binding == 'coding':
        result['session_info'] = session_info()
    return result

def written_propagation():
    if binding == 'coding':
        return dict(bound='applied_by_daemon', broadcast='sent_by_daemon', rebind='done_by_daemon')
    # A delete always removes something, so consumers are always rebound.
    return dict(bound='not_bound', broadcast='sent', rebind='done', rooms='next_start')

def allowed_for(method, params):
    if method in LEGACY_SESSION or method in CODING_ONLY:
        return ['coding']
    if method in ('create', 'resume'):
        return ['unbound']
    if method == 'config.get' and params.get('view', 'session' if binding == 'coding' else 'global') == 'session':
        return ['coding']
    if method == 'config.roles.unassign' and params.get('scope', 'global') == 'session':
        return ['coding']
    return list(ANY_BINDING)

def binding_denial(rid, method, params):
    """The E1-E3 error for a call made in the wrong binding, or None when the call is allowed."""
    if method == 'detach':
        if binding != 'unbound':
            return None
        # Unbound detach keeps its v1 error, with no added keys.
        if LEGACY:
            return response(rid, code='invalid_state')
        return dict(type='response', id=rid,
                    error=dict(code='invalid_state', message='no session bound', delivery='not_sent'))
    allowed = allowed_for(method, params)
    if binding in allowed:
        return None
    if LEGACY:
        return response(rid, code='invalid_state')
    extra = dict(binding=binding, allowed=allowed)
    if method in ('create', 'resume'):
        return failure(rid, 'invalid_state', 'already_bound', 'session already bound', **extra)
    if binding == 'unbound' and method in LEGACY_SESSION:
        return failure(rid, 'invalid_state', 'wrong_binding', 'no session bound', **extra)
    message = '%s requires binding %s; this adapter is %s' % (method, ' or '.join(allowed), binding)
    return failure(rid, 'invalid_state', 'wrong_binding', message, **extra)

def is_id(value):
    return isinstance(value, str) and 1 <= len(value) <= 64

def is_int(value):
    return isinstance(value, int) and not isinstance(value, bool)

def check_common(rid, params):
    """Validate if_revision and dry_run; returns an error frame or None."""
    if 'if_revision' in params and not isinstance(params['if_revision'], str):
        return invalid(rid, 'out_of_range', 'if_revision must be a string')
    if 'dry_run' in params and not isinstance(params['dry_run'], bool):
        return invalid(rid, 'out_of_range', 'dry_run must be a boolean')
    return None

def check_stale(rid, params):
    current = revision()
    if 'if_revision' in params and params['if_revision'] != current:
        return failure(rid, 'invalid_state', 'stale', 'config.json changed; current revision is ' + current)
    return None

def drop_models(uuids):
    catalogue['models'] = [m for m in catalogue['models'] if m['model_uuid'] not in uuids]
    if session is not None and session['main_pin'] in uuids:
        session['main_pin'] = None
        session['effort'] = 'default'
        touch()

def providers_delete(rid, params):
    uuid = params.get('provider_uuid')
    if not is_id(uuid):
        return invalid(rid, 'out_of_range', 'provider_uuid must be a string of 1 to 64 characters')
    error = check_common(rid, params) or check_stale(rid, params)
    if error:
        return error
    kind, prov = lookup_provider(uuid)
    if kind != 'provider':
        return failure(rid, 'not_found', 'not_found', 'no provider ' + uuid)
    if prov['ext_id'] is not None:
        return failure(rid, 'invalid_state', 'managed', 'provider is managed by extension ' + prov['ext_id'])
    removed = [m for m in catalogue['models'] if m['provider_uuid'] == uuid]
    summary = dict(provider_uuid=uuid, models_removed=[m['model_uuid'] for m in removed],
                   main_reset=any('main' in m['roles'] for m in removed))
    if params.get('dry_run', False):
        return response(rid, with_info(dict(dry_run=True, removed=summary, config=config_view('global'),
                                             propagation={})))
    catalogue['providers'] = [p for p in catalogue['providers'] if p['provider_uuid'] != uuid]
    drop_models(set(summary['models_removed']))
    catalogue['file'] = 'ok'
    return response(rid, with_info(dict(dry_run=False, removed=summary, config=config_view('global'),
                                         propagation=written_propagation())))

def models_delete(rid, params):
    if 'scope' in params and params['scope'] != 'global':
        return invalid(rid, 'out_of_range', 'config.models.delete deletes global entries only')
    uuid = params.get('model_uuid')
    if not is_id(uuid):
        return invalid(rid, 'out_of_range', 'model_uuid must be a string of 1 to 64 characters')
    error = check_common(rid, params) or check_stale(rid, params)
    if error:
        return error
    model = find_model(uuid)
    if model is None:
        return failure(rid, 'not_found', 'not_found', 'no model ' + uuid)
    summary = dict(model_uuid=uuid, roles_released=[r for r in PUBLIC_ROLES if r in model['roles']],
                   main_reset='main' in model['roles'])
    if params.get('dry_run', False):
        return response(rid, with_info(dict(dry_run=True, removed=summary, config=config_view('global'),
                                             propagation={})))
    drop_models({uuid})
    catalogue['file'] = 'ok'
    if a.scenario == 'config' and uuid == 'model-unverified':
        return failure(rid, 'unavailable', 'unverified', 'config.json does not match the intended document',
                       delivery='unknown', written=True, detail=dict(diff=[uuid]))
    return response(rid, with_info(dict(dry_run=False, removed=summary, config=config_view('global'),
                                         propagation=written_propagation())))

def roles_unassign(rid, params):
    role, scope = params.get('role'), params.get('scope', 'global')
    if role not in PUBLIC_ROLES:
        return invalid(rid, 'out_of_range', 'role must be one of ' + ', '.join(PUBLIC_ROLES))
    if scope not in ('global', 'session'):
        return invalid(rid, 'out_of_range', 'scope must be global or session')
    error = check_common(rid, params)
    if error:
        return error
    if scope == 'session':
        # The adapter's order: the role, then if_revision, then the binding.
        if role != 'main':
            return invalid(rid, 'out_of_range', 'scope session accepts only role main')
        if 'if_revision' in params:
            return invalid(rid, 'conflicting_params', 'if_revision applies to global scope only')
        denied = binding_denial(rid, 'config.roles.unassign', params)
        if denied:
            return denied
        previous = session['main_pin']
        if previous is None:
            return response(rid, with_info(dict(changed=False, previous_model_uuid=None,
                                                 config=config_view('session'), propagation={})))
        session['main_pin'] = None
        session['effort'] = 'default'
        touch()
        return response(rid, with_info(dict(changed=True, previous_model_uuid=previous,
            config=config_view('session'), propagation=dict(bound='applied_by_daemon'))))
    error = check_stale(rid, params)
    if error:
        return error
    model = holder(role)
    if model is None:
        return response(rid, with_info(dict(changed=False, previous_model_uuid=None,
                                             config=config_view('global'), propagation={})))
    model['roles'] = [r for r in model['roles'] if r != role]
    catalogue['file'] = 'ok'
    if session is not None:
        touch()
    propagation = dict(bound='reloaded' if binding == 'coding' else 'not_bound', broadcast='sent',
                       rebind='not_needed', rooms='next_start')
    return response(rid, with_info(dict(changed=True, previous_model_uuid=model['model_uuid'],
                                         config=config_view('global'), propagation=propagation)))

def effort_get(rid, params):
    wait_ms = params.get('wait_ms', 3000)
    if not is_int(wait_ms) or not 0 <= wait_ms <= 10000:
        return invalid(rid, 'out_of_range', 'wait_ms must be an integer from 0 to 10000')
    main = main_uuid()
    result = dict(selected=session['effort'], main_model_uuid=main)
    if main is None:
        result.update(state='unsupported', options=['default'], note='no Main model')
    elif main in effort_ready or wait_ms > 0:
        effort_ready.add(main)
        result.update(state='ready', options=list(EFFORT_OPTIONS), note=None)
    else:
        result.update(state='loading', options=['default'], note='effort options are loading')
    return response(rid, result)

def effort_set(rid, params):
    effort = params.get('effort')
    if not isinstance(effort, str) or not 1 <= len(effort) <= 64 or re.search(r'\s', effort) or CONTROL.search(effort):
        return invalid(rid, 'out_of_range', 'effort must be 1 to 64 characters without whitespace')
    if 'if_main_model' in params and params['if_main_model'] is not None and not isinstance(params['if_main_model'], str):
        return invalid(rid, 'out_of_range', 'if_main_model must be a string or null')
    main = main_uuid()
    if 'if_main_model' in params and params['if_main_model'] != main:
        return failure(rid, 'invalid_state', 'main_changed', 'the Main model changed')
    if main is not None:
        effort_ready.add(main)
    options = list(EFFORT_OPTIONS) if main is not None else ['default']
    if effort not in options:
        return invalid(rid, 'not_offered', 'effort %s is not offered for this model' % effort)
    session['effort'] = effort
    touch()
    return response(rid, with_info(dict(effort=effort, main_model_uuid=main, superseded=False)))

def limits_set(rid, params):
    bounds = dict(max_output_tokens=(256, 1000000), context_window_limit=(1024, 100000000))
    keys = [k for k in ('max_output_tokens', 'context_window_limit', 'context_model_alias') if k in params]
    if not keys:
        return invalid(rid, 'missing_params',
                       'give max_output_tokens, context_window_limit or context_model_alias')
    for key in keys:
        value = params[key]
        if key == 'context_model_alias':
            if not isinstance(value, str) or len(value) > 200:
                return invalid(rid, 'out_of_range', 'context_model_alias must be at most 200 characters')
        elif not is_int(value) or not (value == 0 or bounds[key][0] <= value <= bounds[key][1]):
            return invalid(rid, 'out_of_range', '%s must be 0 or %d..%d' % ((key,) + bounds[key]))
    for key in keys:
        session['limits'][key] = params[key]
    touch()
    return response(rid, with_info(dict(session['limits'])))

def config_call(rid, method, params):
    if method == 'config.get':
        view = params.get('view', 'session' if binding == 'coding' else 'global')
        if view not in ('global', 'session'):
            return invalid(rid, 'out_of_range', 'view must be global or session')
        denied = binding_denial(rid, method, params)
        return denied or response(rid, config_view(view))
    if method == 'config.providers.delete':
        return providers_delete(rid, params)
    if method == 'config.models.delete':
        return models_delete(rid, params)
    if method == 'config.roles.unassign':
        return roles_unassign(rid, params)
    denied = binding_denial(rid, method, params)
    if denied:
        return denied
    if method == 'config.effort.get':
        return effort_get(rid, params)
    if method == 'config.effort.set':
        return effort_set(rid, params)
    if method == 'config.limits.get':
        return response(rid, dict(session['limits']))
    if method == 'config.limits.set':
        return limits_set(rid, params)
    return response(rid, dict(session_info=session_info()))

def apply_configure(params):
    for item in params.get('providers', []):
        name = item['name'].strip()
        existing = next((p for p in catalogue['providers'] if p['name'] == name), None)
        if existing is None:
            existing = dict(provider_uuid='prov-' + name, name=name, api_key='', ext_id=None)
            catalogue['providers'].append(existing)
        existing.update(api_type=item.get('api_type', 'openai'), endpoint=item.get('endpoint', ''))
        if isinstance(item.get('api_key'), str):
            existing['api_key'] = item['api_key']
    for role, spec in params.get('models', {}).items():
        provider = spec['provider'].strip()
        owner = next((p for p in catalogue['providers'] if p['name'] == provider), None)
        catalogue['models'] = [m for m in catalogue['models'] if m['model_uuid'] != 'model-' + role]
        for m in catalogue['models']:
            m['roles'] = [r for r in m['roles'] if r != role]
        catalogue['models'].append(dict(model_uuid='model-' + role, name=spec['model_id'].strip(),
            model_id=spec['model_id'].strip(), provider_uuid=owner['provider_uuid'] if owner else 'prov-' + provider,
            route=spec.get('route'), roles=[role], source_uuid=None))
    catalogue['file'] = 'ok'
    if session is not None:
        touch()

def configure_result(params):
    raw_providers = params.get('providers', [])
    raw_models = params.get('models', {})
    if not isinstance(raw_providers, list) or not isinstance(raw_models, dict):
        return None
    providers = []
    for item in raw_providers:
        if not isinstance(item, dict) or not isinstance(item.get('name'), str) or not item['name'].strip():
            return None
        providers.append(dict(name=item['name'].strip(), uuid='prov-' + item['name'].strip()))
    models = {}
    for role, spec in raw_models.items():
        if not isinstance(spec, dict):
            return None
        model_id, provider = spec.get('model_id'), spec.get('provider')
        if not isinstance(model_id, str) or not model_id.strip() or not isinstance(provider, str) or not provider.strip():
            return None
        models[role] = dict(uuid='model-' + role, model_id=model_id.strip(), provider=provider.strip())
    if not providers and not models:
        return None
    apply_configure(params)
    return dict(providers=providers, models=models)

def outcome(disposition, turn_id=None, code=None, replacement=False, interrupted=None):
    value = dict(disposition=disposition, turn_id=turn_id,
                 reason=None if code is None else dict(code=code, message='fake ' + code))
    if replacement:
        value['interrupted_turn_id'] = interrupted
    return value

while True:
    raw = sys.stdin.buffer.readline(MAX_LINE + 2)
    if not raw:
        break
    if len(raw.rstrip(b'\n')) > MAX_LINE or not raw.endswith(b'\n'):
        sys.stderr.write('invalid or oversized frame\n')
        sys.exit(2)
    try:
        request = json.loads(raw.decode('utf-8'))
        rid, method, params = request['id'], request['method'], request['params']
        if not isinstance(rid, str) or not rid or not isinstance(params, dict):
            raise ValueError('bad request')
    except (ValueError, KeyError, TypeError):
        sys.stderr.write('malformed request\n')
        sys.exit(2)
    if os.environ.get('KOMA_FAKE_LOG'):
        with open(os.environ['KOMA_FAKE_LOG'], 'a') as log: log.write(json.dumps(request) + '\n')
    if a.scenario == 'timeout' and method == 'submit':
        time.sleep(2)
    if a.scenario == 'eof' and method in ('submit', 'interrupt_and_submit'):
        sys.exit(3)
    if a.scenario == 'malformed':
        print('{bad json', flush=True)
        sys.exit(3)
    known = V1_METHODS if LEGACY else METHODS
    if method not in known:
        frame = response(rid, code='unsupported_method')
    elif method == 'configure':
        result = configure_result(params)
        frame = response(rid, result) if result is not None else response(rid, code='invalid_request')
    elif method in CONFIG_METHODS or method == 'sessions.info':
        # Host methods work in every binding, so they are answered before any session check.
        frame = config_call(rid, method, params)
        if frame.get('error', {}).get('delivery') == 'unknown' and binding == 'coding':
            # An unknown outcome on the bound coding connection ends the adapter.
            emit(frame)
            break
    elif binding_denial(rid, method, params) is not None:
        frame = binding_denial(rid, method, params)
        if method == 'detach':
            emit(frame)
            break
    elif method in ('create', 'resume'):
        session = dict(state=dict(session_id=params.get('session_id', 'session-1'),
            cwd=params.get('cwd', '/fake/workspace'), model_id=params.get('model_id', 'fake/model'),
            policy=params.get('policy', dict(plan='forbid', approval='reject')),
            revision=1, external_approvals=params.get('external_approvals', False), pending_approval=None, gate=None,
            turn=None, working=False, awaiting_approval=False, approval_reason=None,
            pending_count=0, messages=[], streaming=None),
            name='fake session', effort='default', main_pin=None,
            limits=dict(max_output_tokens=0, context_window_limit=0, context_model_alias='', effective_window=128000))
        if a.scenario == 'config' and find_model('model-main') is not None:
            session['main_pin'] = 'model-main'
        binding = 'coding'
        touch()
        frame = response(rid, state_result())
    elif method == 'detach':
        emit(response(rid, dict(detached=True)))
        break
    elif method == 'state':
        state = session['state']
        if a.scenario in ('finish', 'flood_finish') and state['turn'] and state['turn']['state'] == 'running':
            state['turn']['state'] = 'finished'
            state['working'] = False
            state['streaming'] = None
            state['messages'].append(dict(role='assistant',content='Fake answer é雪'))
            state['revision'] += 1
        frame = response(rid, state_result())
    elif method == 'approve':
        state = session['state']
        if (not state['external_approvals'] or not state['pending_approval'] or params.get('call_id') != state['pending_approval']['call_id']):
            frame = response(rid, code='invalid_state')
        else:
            state.update(awaiting_approval=False, approval_reason=None, pending_approval=None, gate=None)
            state["revision"] += 1
            frame = response(rid, dict(approved=params['approve']))
            event()
    else:
        state = session['state']
        replacement = method == 'interrupt_and_submit'
        active = state['turn'] if state['turn'] and state['turn']['state'] == 'running' else None
        target = params.get('turn_id')
        code = None
        if method in ('interrupt', 'interrupt_and_submit') and (not active or active['id'] != target):
            code = 'stale_turn'
        elif method == 'submit' and active:
            code = 'busy'
        elif method == 'enqueue' and not active:
            code = 'busy'
        elif method == 'enqueue' and state['pending_count'] >= 5:
            code = 'queue_full'
        elif method != 'interrupt' and not params.get('text', '').strip():
            code = 'invalid_input'
        elif a.scenario == 'reject':
            code = 'invalid_input'
        if code:
            value = outcome('rejected', target if method == 'interrupt' else None, code, replacement)
        elif a.scenario == 'unknown':
            emit(response(rid, code='unavailable', delivery='unknown'))
            continue
        elif method == 'enqueue':
            state['pending_count'] += 1
            value = outcome('queued')
        elif method == 'interrupt':
            state['turn']['state'] = 'interrupted'
            state['working'] = False
            value = outcome('interrupted', target)
        elif replacement and a.scenario == 'partial':
            state['turn']['state'] = 'interrupted'
            state['working'] = False
            value = outcome('rejected', code='internal', replacement=True, interrupted=target)
        else:
            counter += 1
            tid = 'turn-' + str(counter)
            state.update(turn=dict(id=tid, state='running', error=None), working=True, streaming=None,
                         awaiting_approval=a.scenario == 'approval',
                         approval_reason='fake tool' if a.scenario == 'approval' else None)
            state['messages'].append(dict(role='user', content=params['text']))
            value = outcome('started', tid, replacement=replacement, interrupted=target if replacement else None)
        state['revision'] += 1
        if a.scenario == 'approval' and state['awaiting_approval']:
            state['pending_approval'] = dict(call_id='call-1', reason='fake tool')
            state['gate'] = dict(kind='tool', reason='fake tool', supported=state['external_approvals'])
        frame = response(rid, value)
        event()  # Deliberately interleave event BEFORE correlated result.
    if a.scenario == 'hidden' and method == 'state' and binding == 'coding':
        session['state']['turn'] = dict(id='newer', state='running', error=None)
        session['state']['revision'] += 1
        frame = response(rid, state_result())
    if a.scenario in ('output','output_gap','bad_offset','flood_output') and method == 'submit' and binding == 'coding':
        session['state']['streaming'] = 'é雪'
        session['state']['revision'] += 1
        seq += 2 if a.scenario == 'output_gap' else 1
        emit(dict(type='event', event='output_append', seq=seq, revision=session['state']['revision'], offset=999 if a.scenario == 'bad_offset' else 0, text='é雪'))
    if a.scenario in ('flood_finish', 'flood_output') and method == 'submit' and (a.scenario == 'flood_finish' or counter == 1):
        for _ in range(300):
            event()
    if a.scenario == 'bad_result' and method == 'submit':
        frame['result']['turn_id'] = 123
    if a.scenario == 'wrong_id':
        emit(response('unrecognized-id', dict(detached=True)))
    if a.scenario == 'event_flood' and session and method == 'state':
        for _ in range(300):
            event()
    if a.scenario == 'out_of_order':
        if held is None:
            held = frame
            continue
        emit(frame)
        emit(held)
        held = None
    else:
        emit(frame)
