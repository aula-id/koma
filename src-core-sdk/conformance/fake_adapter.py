#!/usr/bin/env python3
"""Deterministic, dependency-free JSONL peer. Never contacts Koma."""
import argparse
import base64
import copy
import hashlib
import io
import json
import re
import sys
import os
import time
import uuid
import zipfile
from urllib.parse import urlsplit, urlunsplit

MAX_LINE = 40 * 1024 * 1024
V1_METHODS = ['create', 'resume', 'state', 'submit', 'enqueue', 'interrupt',
              'interrupt_and_submit', 'approve', 'configure', 'detach']
CONFIG_METHODS = ['config.get', 'config.providers.delete', 'config.models.delete', 'config.roles.unassign',
                  'config.tool_policy.set', 'config.effort.get', 'config.effort.set', 'config.limits.get', 'config.limits.set']
SESSION_METHODS = ['sessions.info', 'sessions.list', 'sessions.rename', 'sessions.delete', 'sessions.stop',
                   'sessions.transcript']
SKILL_METHODS = ['skills.list', 'skills.get', 'skills.read_file', 'skills.install', 'skills.remove', 'skills.create',
                 'skills.update']
ROOM_BINDERS = ['room.open', 'room.attach']
ROOM_SOCKET_WRITES = ['room.send', 'room.interrupt', 'room.control']
ROOM_OTHERS = ['room.close', 'room.status', 'room.history', 'room.shelf.list', 'room.item', 'room.trajectory',
               'room.reports', 'room.spend']
TEAM_METHODS = ['room.team.get', 'room.models.set', 'room.avatars.upsert', 'room.avatars.remove']
METHODS = (V1_METHODS + CONFIG_METHODS + SESSION_METHODS + ['usage.query'] + SKILL_METHODS + ['room.list']
           + ROOM_BINDERS + ['room.close'] + ROOM_SOCKET_WRITES + ROOM_OTHERS[1:] + TEAM_METHODS)
FEATURES = ['bindings.room', 'errors.reason', 'state.session_info', 'create.name', 'config.lock', 'events.room',
            'room.template']
EVENTS = ['state', 'output_append', 'turn', 'room.line', 'room.delta', 'room.status', 'room.notice', 'room.closed',
          'room.gap']
LIMITS = dict(frame_bytes=MAX_LINE, request_ids=65536, text_bytes=2097152,
              attachments=dict(count=16, pasted_bytes=2097152, total_bytes=33554432),
              list=dict(sessions=500, rooms=500, skills=1000, usage_groups=500, usage_buckets=1000, shelf_rows=500,
                        reports=100, history_lines=200, trajectory_goals=200, trajectory_nodes=2000,
                        trajectory_edges=8000),
              room=dict(ring_lines=8000, ring_bytes=67108864, page_bytes=8388608, event_text_bytes=1048576,
                        status_text_bytes=262144, item_bytes=16777216, tail_lines=200),
              skills=dict(inline_zip_bytes=29360128, zip_bytes=67108864, entry_bytes=2097152,
                          companion_bytes=8388608, files=1000, depth=16, name_bytes=64),
              config=dict(lock_wait_ms=3000),
              time=dict(ipc_ms=15000, room_start_ms=30000, usage_query_ms=2000, session_stop_ms=8000))
NEW_CALLS = (SESSION_METHODS[1:] + ['usage.query'] + SKILL_METHODS + ['room.list'] + ROOM_BINDERS + ROOM_SOCKET_WRITES
             + ROOM_OTHERS + TEAM_METHODS)
WORLD_SCENARIOS = ('sessions', 'usage', 'room', 'room_gap', 'room_takeover')
LEGACY_SESSION = ['state', 'submit', 'enqueue', 'interrupt', 'interrupt_and_submit', 'approve']
CODING_ONLY = ['config.effort.get', 'config.effort.set', 'config.limits.get', 'config.limits.set', 'sessions.info']
ANY_BINDING = ['unbound', 'coding', 'room_live', 'room_closed']
ROOM_BINDINGS = ['room_live', 'room_closed']
PUBLIC_ROLES = ['main', 'awareness', 'safeguard', 'compactor', 'planner']
EFFORT_OPTIONS = ['default', 'low', 'medium', 'high']
CONTROL = re.compile('[\x00-\x1f\x7f]')

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--scenario', choices=['normal', 'reject', 'partial', 'unknown',
    'eof', 'malformed', 'wrong_id', 'out_of_order', 'event_flood', 'approval',
    'bad_version', 'stderr', 'output', 'timeout', 'hidden', 'fragmented', 'missing_cap', 'output_gap', 'bad_offset', 'finish', 'bad_result', 'flood_finish', 'flood_output',
    'config', 'wrong_binding', 'legacy_hello', 'sessions', 'usage', 'skills', 'room', 'room_gap',
    'room_takeover'], default=os.environ.get('KOMA_FAKE_SCENARIO', 'normal'))
p.add_argument('--binding', choices=['unbound', 'room_closed'], default='unbound',
               help='initial binding; room_closed is accepted with --scenario wrong_binding only')
a = p.parse_args([] if sys.argv[1:] == ['sdk', 'serve'] else None)
if a.binding != 'unbound' and a.scenario != 'wrong_binding':
    p.error('--binding room_closed needs --scenario wrong_binding')
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
binding = a.binding
seq = 0
counter = 0
held = None
after = []  # frames written right after the current response

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
tool_deny = []
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
    document = dict(catalogue, tool_policy=dict(deny=tool_deny)) if tool_deny else catalogue
    raw = json.dumps(document, sort_keys=True, separators=(',', ':')).encode()
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
    if method in ROOM_BINDERS:
        return ['unbound', 'room_closed']
    if method in TEAM_METHODS:
        target = params.get('target', 'room' if method == 'room.team.get' and binding in ROOM_BINDINGS else None)
        return list(ROOM_BINDINGS) if target == 'room' else list(ANY_BINDING)
    if method in ROOM_SOCKET_WRITES:
        return ['room_live']
    if method in ROOM_OTHERS:
        return list(ROOM_BINDINGS)
    if method == 'config.get' and params.get('view', 'session' if binding == 'coding' else 'global') == 'session':
        return ['coding']
    if method == 'config.roles.unassign' and params.get('scope', 'global') == 'session':
        return ['coding']
    return list(ANY_BINDING)

def binding_denial(rid, method, params):
    """The E1-E4 error for a call made in the wrong binding, or None when the call is allowed."""
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
    if method in ('create', 'resume') + tuple(ROOM_BINDERS):
        return failure(rid, 'invalid_state', 'already_bound', 'session already bound', **extra)
    if method in ROOM_SOCKET_WRITES and binding == 'room_closed':
        return failure(rid, 'invalid_state', 'room_closed', 'the room connection is closed', **extra)
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

def tool_policy_set(rid, params):
    if 'deny' not in params:
        return invalid(rid, 'missing_params', 'deny is required')
    deny = params['deny']
    if not isinstance(deny, list) or len(deny) > 256:
        return invalid(rid, 'out_of_range', 'deny must be a list of at most 256 tool names or globs')
    for item in deny:
        if not isinstance(item, str) or not 1 <= len(item) <= 200 or re.search(r'[\s,]', item) or CONTROL.search(item):
            return invalid(rid, 'out_of_range', 'a deny entry is 1 to 200 characters with no spaces, commas or control characters')
    error = check_common(rid, params)
    if error:
        return error
    error = check_stale(rid, params)
    if error:
        return error
    stored = list(dict.fromkeys(deny))
    if stored == tool_deny:
        return response(rid, with_info(dict(changed=False, deny=stored, config=config_view('global'),
                                             propagation={})))
    tool_deny[:] = stored
    catalogue['file'] = 'ok'
    if session is not None:
        touch()
    propagation = dict(bound='reloaded' if binding == 'coding' else 'not_bound', broadcast='sent',
                       rebind='not_needed', rooms='next_start')
    return response(rid, with_info(dict(changed=True, deny=stored, config=config_view('global'),
                                         propagation=propagation)))

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
    if method == 'config.tool_policy.set':
        return tool_policy_set(rid, params)
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
# Fixed clock: every default window, timestamp and age below is relative to it, so runs are reproducible.
NOW = 1767225600
NOW_MS = NOW * 1000
DAY = 86400
KNOWN_WORKDIRS = ('/fake/workspace', '/fake/other', '/fake/empty')
WITH_WORLD = a.scenario in WORLD_SCENARIOS or a.binding == 'room_closed'
WITH_LEDGER = WITH_WORLD

def room_session_id(parent):
    return uuid.uuid5(uuid.NAMESPACE_OID, 'secure-room:' + parent).hex

def registry_row(sid, name, workdir, kind='coding', age=0, live=False, working=False, locked=False, parent=None,
                 registered=True, **flags):
    row = dict(id=sid, name=name, workdir=workdir, kind=kind, created=NOW - 30 * DAY if registered else None,
               updated=NOW - age if registered else None, live=live, working=working, locked=locked,
               parent=parent, registered=registered)
    row.update(flags)
    return row

def seed_registry():
    if not WITH_WORLD:
        return []
    return [
        registry_row('sess-coding-a', 'Alpha', '/fake/workspace', age=60, live=True, working=True, locked=True),
        registry_row('sess-coding-b', 'Beta', '/fake/other', age=3600),
        registry_row('sess-coding-c', 'sess-coding-c', '/fake/other', age=7200, main_ok=False),
        registry_row('sess-locked', 'Locked', '/fake/other', age=8000, locked=True),
        registry_row('sess-stuck', 'Stuck', '/fake/workspace', age=120, live=True, stuck=True),
        registry_row('sess-partial', 'Partial', '/fake/other', age=9000, partial=True),
        registry_row('sess-busy', 'Busy', '/fake/other', age=9500, busy=True),
        registry_row('sess-nostart', 'Nostart', '/fake/other', age=9800, no_start=True),
        registry_row('sess-clash', 'Clash', '/fake/other', age=9900),
        registry_row(room_session_id('sess-clash'), 'Clash victim', '/fake/other', age=9950),
        registry_row('sess-live-nr', None, None, live=True, registered=False),
        registry_row(room_session_id('sess-coding-a'), 'Alpha room', '/fake/workspace', kind='room', age=30,
                     live=True, locked=True, parent='sess-coding-a', modern=True),
        registry_row(room_session_id('sess-coding-b'), 'Beta room', '/fake/other', kind='room', age=3000,
                     parent='sess-coding-b', modern=False),
    ]

registry = seed_registry()

def find_row(sid):
    return next((r for r in registry if r['id'] == sid), None)

def is_bound(sid):
    if binding == 'coding' and session is not None:
        return session['state']['session_id'] == sid
    return binding in ROOM_BINDINGS and bound_room is not None and bound_room['row']['id'] == sid

def session_row(r):
    coding = r['kind'] == 'coding'
    rid = room_session_id(r['id']) if coding else None
    room_row = find_row(rid) if coding else None
    return dict(session_id=r['id'], name=r['name'], workdir=r['workdir'], kind=r['kind'], created_at=r['created'],
                updated_at=r['updated'], live=r['live'], working=r['working'] if coding else None,
                locked=r['locked'], bound=is_bound(r['id']), parent_session_id=None if coding else r['parent'],
                room_session_id=rid, room_live=bool(room_row and room_row['kind'] == 'room' and room_row['live']))

def room_info(r):
    parent = find_row(r['parent']) if r['parent'] else None
    return dict(session_id=r['id'], parent_session_id=r['parent'], parent_name=parent['name'] if parent else None,
                name=r['name'], workdir=r['workdir'], live=r['live'], locked=r['locked'], bound=is_bound(r['id']),
                created_at=r['created'], updated_at=r['updated'])

def register_session(sid, name, workdir):
    row = find_row(sid)
    if row is None:
        row = registry_row(sid, name, workdir, live=True, locked=True)
        row['created'] = row['updated'] = NOW
        registry.append(row)
    row.update(live=True, locked=True)
    return row

def encode_cursor(key):
    return base64.urlsafe_b64encode(json.dumps(key).encode()).decode()

def decode_cursor(cursor):
    try:
        key = json.loads(base64.urlsafe_b64decode(cursor.encode()).decode())
    except (ValueError, AttributeError, UnicodeError):
        return None
    return key if isinstance(key, list) else None

def paged(rid, items, sort_key, params, default, maximum):
    """One page of `items` in `sort_key` order: (rows, next_cursor, None), or (None, None, error frame)."""
    limit = params.get('limit', default)
    if not is_int(limit) or not 1 <= limit <= maximum:
        return None, None, invalid(rid, 'out_of_range', 'limit must be an integer from 1 to %d' % maximum)
    ordered = sorted(items, key=sort_key)
    if 'cursor' in params:
        key = decode_cursor(params['cursor'])
        try:
            if key is None:
                raise TypeError('cursor')
            ordered = [i for i in ordered if list(sort_key(i)) > key]
        except TypeError:
            return None, None, invalid(rid, 'bad_cursor', 'the cursor does not decode')
    rows = ordered[:limit]
    more = len(ordered) > limit
    return rows, encode_cursor(list(sort_key(rows[-1]))) if more else None, None

def session_sort(r):
    return (-(r['updated'] or 0), r['id'])

def checked_name(rid, value, field='name'):
    """The trimmed name, or an error frame. Control characters are refused
    before trimming, as the adapter does."""
    if not isinstance(value, str):
        return invalid(rid, 'invalid_name', field + ' must be a string')
    if CONTROL.search(value):
        return invalid(rid, 'invalid_name', field + ' must not contain control characters')
    value = value.strip()
    if not 1 <= len(value) <= 200:
        return invalid(rid, 'invalid_name', field + ' must be 1 to 200 characters after trimming')
    return value

def sessions_list(rid, params):
    kind = params.get('kind', 'all')
    if kind not in ('coding', 'room', 'all'):
        return invalid(rid, 'out_of_range', 'kind must be coding, room or all')
    if 'live' in params and not isinstance(params['live'], bool):
        return invalid(rid, 'out_of_range', 'live must be a boolean')
    rows = [r for r in registry if kind in ('all', r['kind']) and params.get('live', r['live']) == r['live']]
    page, cursor, error = paged(rid, rows, session_sort, params, 100, 500)
    if error:
        return error
    return response(rid, dict(sessions=[session_row(r) for r in page], next_cursor=cursor))

def sessions_rename(rid, params):
    sid = params.get('session_id')
    if not is_id(sid):
        return invalid(rid, 'out_of_range', 'session_id must be a string of 1 to 64 characters')
    name = checked_name(rid, params.get('name'))
    if isinstance(name, dict):
        return name
    row = find_row(sid)
    if row is None or not row['registered']:
        return failure(rid, 'not_found', 'not_found', 'no session ' + sid)
    bound_coding = binding == 'coding' and is_bound(sid)
    if row['kind'] == 'coding' and row['live'] and not bound_coding:
        return failure(rid, 'invalid_state', 'live',
                       'rename it from an adapter bound to it, or stop it first')
    if row.get('busy'):
        return failure(rid, 'unavailable', 'sqlite_busy', 'the registry is busy')
    row['name'] = name
    row['updated'] = NOW
    if bound_coding:
        session['name'] = name
        touch()
    return response(rid, dict(session=session_row(row)))

def sessions_delete(rid, params):
    sid = params.get('session_id')
    if not is_id(sid):
        return invalid(rid, 'out_of_range', 'session_id must be a string of 1 to 64 characters')
    row = find_row(sid)
    if row is None or not row['registered']:
        return failure(rid, 'not_found', 'not_found', 'no session ' + sid)
    if is_bound(sid):
        return failure(rid, 'invalid_state', 'bound_self', 'this adapter is bound to that session')
    if row['live']:
        return failure(rid, 'invalid_state', 'live', 'the session is live; stop it first')
    if row['locked']:
        return failure(rid, 'invalid_state', 'session_locked', 'the session lock is held by a live process')
    if row.get('partial'):
        row['partial'] = False
        return failure(rid, 'unavailable', 'partial_delete', 'removal failed midway', delivery='unknown',
                       written=True)
    registry.remove(row)
    orphan = None
    if row['kind'] == 'coding':
        child = find_row(room_session_id(sid))
        orphan = child['id'] if child is not None and child['kind'] == 'room' else None
    return response(rid, dict(deleted=True, kind=row['kind'], orphaned_room_session_id=orphan))

def sessions_stop(rid, params):
    sid = params.get('session_id')
    if not is_id(sid):
        return invalid(rid, 'out_of_range', 'session_id must be a string of 1 to 64 characters')
    if is_bound(sid):
        return failure(rid, 'invalid_state', 'bound_self', 'this adapter is bound to that session')
    row = find_row(sid)
    if row is None or not row['live']:
        return response(rid, dict(stopped=True, was_live=False))
    if row.get('stuck'):
        return failure(rid, 'unavailable', 'still_running', 'the daemon is still running after the stop',
                       delivery='unknown')
    row.update(live=False, working=False, locked=False)
    return response(rid, dict(stopped=True, was_live=True))

# Stored messages by session: (id, role, text, created_at, truncated). Text is scrubbed, as the adapter scrubs it.
TRANSCRIPTS = {
    'sess-coding-a': [(1, 'user', 'run the tests', NOW - 600, False), (2, 'assistant', 'Running them now.', NOW - 590, False)],
    'sess-coding-b': [(1, 'user', 'read the config', NOW - 7200, False),
                      (2, 'tool', 'key = [redacted]', NOW - 7190, False),
                      (3, 'tool', 'x' * 64 + ' (cut)', NOW - 7180, True),
                      (4, 'assistant', 'Done.', NOW - 7170, False)],
}

def sessions_transcript(rid, params):
    sid = params.get('session_id')
    if not is_id(sid):
        return invalid(rid, 'out_of_range', 'session_id must be a string of 1 to 64 characters')
    since = params.get('since', 0)
    if not is_int(since) or since < 0:
        return invalid(rid, 'out_of_range', 'since must be an integer of 0 or more')
    limit = params.get('limit', 100)
    if not is_int(limit) or not 1 <= limit <= 500:
        return invalid(rid, 'out_of_range', 'limit must be an integer from 1 to 500')
    row = find_row(sid)
    if row is None or not row['registered']:
        return failure(rid, 'not_found', 'not_found', 'no session ' + sid)
    if row['kind'] == 'room':
        return invalid(rid, 'out_of_range', 'the session is a room; read it with room.history')
    if row.get('busy'):
        return failure(rid, 'unavailable', 'sqlite_busy', 'the session archive is busy')
    later = [m for m in TRANSCRIPTS.get(sid, []) if m[0] > since]
    page = later[:limit]
    messages = []
    for mid, role, text, at, truncated in page:
        message = dict(id=mid, role=role, text=text, created_at=at)
        if truncated:
            message['truncated'] = True
        messages.append(message)
    more = len(later) > limit
    return response(rid, dict(session_id=sid, live=row['live'], messages=messages,
                              next_since=page[-1][0] if more else None))

def sessions_call(rid, method, params):
    if method == 'sessions.transcript':
        return sessions_transcript(rid, params)
    if method == 'sessions.list':
        return sessions_list(rid, params)
    if method == 'sessions.rename':
        return sessions_rename(rid, params)
    if method == 'sessions.delete':
        return sessions_delete(rid, params)
    return sessions_stop(rid, params)

# In-memory usage ledger: (at, session, model, role, tokens_in, tokens_cached, tokens_out, cost).
def seed_ledger():
    if not WITH_LEDGER:
        return []
    a_id, b_id, room_id = 'sess-coding-a', 'sess-coding-b', room_session_id('sess-coding-a')
    return [
        (NOW - 40 * DAY, a_id, 'fake/main', 'main', 100, 0, 10, 0.001),
        (NOW - 3 * DAY + 3600, a_id, 'fake/main', 'main', 1000, 200, 300, 0.01),
        (NOW - 3 * DAY + 7200, a_id, 'fake/main', 'main', 2000, 500, 400, 0.02),
        (NOW - 2 * DAY + 3600, a_id, 'fake/main', 'sub:explorer', 800, 0, 200, 0.005),
        (NOW - 2 * DAY + 36000, b_id, 'fake/planner', 'main', 1500, 300, 500, 0.03),
        (NOW - DAY + 100, b_id, 'fake/main', 'main', 1200, 0, 350, 0.015),
        (NOW - DAY + 43200, 'sess-deleted', 'fake/main', 'main', 500, 0, 100, 0.004),
        (NOW - 3600, a_id, 'fake/main', 'main', 3000, 1000, 600, 0.04),
        (NOW - 1800, room_id, 'fake/main', 'main', 4000, 0, 800, 0.05),
        (NOW - 1700, room_id, 'fake/fast', 'sub:scout', 2500, 0, 500, 0.01),
        (NOW - 1600, room_id, 'fake/fast', 'sub:analyst', 1800, 0, 300, 0.008),
        (NOW - 1500, room_id, 'fake/fast', 'sub:scout', 900, 0, 150, 0.004),
    ]

ledger = seed_ledger()

def is_abs(path):
    return isinstance(path, str) and (path.startswith('/') or re.match(r'^[A-Za-z]:[\\/]', path) is not None)

def totals_of(rows):
    return dict(calls=len(rows), tokens_in=sum(r[4] for r in rows), tokens_cached=sum(r[5] for r in rows),
                tokens_out=sum(r[6] for r in rows), cost=round(sum(r[7] for r in rows), 6))

def bucket_start(at, bucket, offset):
    local = at + offset * 60
    if bucket == 'hour':
        return local // 3600 * 3600 - offset * 60
    if bucket == 'day':
        return local // DAY * DAY - offset * 60
    return ((local // DAY + 3) // 7 * 7 - 3) * DAY - offset * 60

def usage_rows(params, from_at, to_at):
    rows = [r for r in ledger if from_at <= r[0] < to_at]
    if 'session_id' in params:
        rows = [r for r in rows if r[1] == params['session_id']]
    if 'workdir' in params:
        ids = {x['id'] for x in registry if x['workdir'] == params['workdir'].rstrip('/')}
        rows = [r for r in rows if r[1] in ids]
    return rows

def usage_group(group_by, key, rows):
    group = dict(key=key, label=None)
    group.update(totals_of(rows))
    if group_by == 'role':
        kind, _, agent = key.partition(':')
        group.update(label=agent or 'main', role_kind='sub' if agent else 'main', agent=agent or None)
    elif group_by == 'session':
        row = find_row(key)
        group.update(label=row['name'] if row else None, kind=row['kind'] if row else None, deleted=row is None)
    return group

def usage_groups(rows, group_by, limit):
    index = {'model': 2, 'role': 3, 'session': 1}[group_by]
    keys = {}
    for r in rows:
        keys.setdefault(r[index], []).append(r)
    groups = sorted((usage_group(group_by, k, v) for k, v in keys.items()), key=lambda g: (-g['cost'], g['key']))
    return groups[:limit], len(groups) > limit

def usage_query(rid, params):
    for key in ('from_at', 'to_at'):
        if key in params and not is_int(params[key]):
            return invalid(rid, 'out_of_range', key + ' must be an integer')
    from_at, to_at = params.get('from_at', NOW - 30 * DAY), params.get('to_at', NOW)
    for key, at in (('to_at', to_at), ('from_at', from_at)):
        if not 0 <= at <= 253402300799:
            return invalid(rid, 'out_of_range', key + ' must be between 0 and 253402300799')
    if 'session_id' in params and not is_id(params['session_id']):
        return invalid(rid, 'out_of_range', 'session_id must be a string of 1 to 64 characters')
    if 'workdir' in params and not is_abs(params['workdir']):
        return invalid(rid, 'out_of_range', 'workdir must be an absolute path')
    group_by, bucket = params.get('group_by', 'none'), params.get('bucket', 'none')
    if group_by not in ('none', 'model', 'role', 'session'):
        return invalid(rid, 'out_of_range', 'group_by must be none, model, role or session')
    if bucket not in ('none', 'hour', 'day', 'week'):
        return invalid(rid, 'out_of_range', 'bucket must be none, hour, day or week')
    offset = params.get('tz_offset_minutes', 0)
    if not is_int(offset) or not -720 <= offset <= 840:
        return invalid(rid, 'out_of_range', 'tz_offset_minutes must be an integer from -720 to 840')
    limit = params.get('limit', 100)
    if not is_int(limit) or not 1 <= limit <= 500:
        return invalid(rid, 'out_of_range', 'limit must be an integer from 1 to 500')
    if to_at <= from_at:
        return invalid(rid, 'out_of_range', 'to_at must be greater than from_at')
    if to_at - from_at > 400 * DAY:
        return invalid(rid, 'range_too_large', 'the range is longer than 400 days')
    step = dict(hour=3600, day=DAY, week=7 * DAY).get(bucket)
    first = bucket_start(from_at, bucket, offset) if step else None
    count = (bucket_start(to_at - 1, bucket, offset) - first) // step + 1 if step else 0
    if count > 1000:
        return invalid(rid, 'range_too_large', 'the range needs more than 1000 buckets')
    if params.get('session_id') == 'sess-slow':
        return failure(rid, 'unavailable', 'query_timeout', 'the query passed its 2 s deadline')
    rows = usage_rows(params, from_at, to_at)
    result = dict(from_at=from_at, to_at=to_at, ledger='ok' if WITH_LEDGER else 'missing', totals=totals_of(rows))
    if group_by != 'none':
        result['groups'], result['groups_truncated'] = usage_groups(rows, group_by, limit)
    if step:
        buckets = []
        for n in range(count):
            start = first + n * step
            part = [r for r in rows if bucket_start(r[0], bucket, offset) == start]
            buckets.append(dict(start_at=start, **totals_of(part)))
        result['buckets'] = buckets
    return response(rid, result)

# In-memory skill roots. The global root is shared; project roots are keyed by canonical workdir.
TIER_RE = re.compile(r'^(global|claude|project-agent|project-agents|extra-root:[0-9]+)$')
SKILL_NAME = re.compile(r'^[a-z0-9]+(-[a-z0-9]+)*$')
SKILL_HOME = '/fake/home/.koma/skills'

def skill_rec(name, description, instruction='Do the thing.', triggers=(), tools=(), companions=None, unsafe=False):
    return dict(name=name, description=description, instruction=instruction, triggers=list(triggers),
                tools=list(tools), companions=companions or {}, unsafe=unsafe)

def seed_skills():
    if a.scenario != 'skills':
        return dict(global_={}, projects={})
    glob = {s['name']: s for s in [
        skill_rec('code-review', 'Review a diff', 'Read the diff and list defects.', ['review', 'diff'], ['read_file'],
                  {'notes/checklist.md': 'check names\ncheck tests\n', 'bin/tool.bin': b'\x00\xff\xfe'}),
        skill_rec('deploy-notes', 'Write deployment notes'),
        skill_rec('odd-yaml', 'Frontmatter that cannot be saved safely', unsafe=True),
    ]}
    work = {tier: {} for tier in ('claude', 'project-agent', 'project-agents', 'extra-root:0')}
    work['claude']['claude-helper'] = skill_rec('claude-helper', 'Helper from .claude')
    work['project-agent']['legacy-skill'] = skill_rec('legacy-skill', 'Older project skill')
    work['project-agents']['code-review'] = skill_rec('code-review', 'Project review rules', 'Follow the repo rules.')
    work['project-agents']['proj-helper'] = skill_rec('proj-helper', 'Project helper', triggers=['help'])
    work['extra-root:0']['extra-skill'] = skill_rec('extra-skill', 'From an extra root')
    return dict(global_=glob, projects={'/fake/workspace': work})

skill_store = seed_skills()
SKILL_DIAGNOSTICS = [dict(path=SKILL_HOME + '/broken/SKILL.md', error='missing description')]

def tier_rank(tier):
    ranks = dict(zip(('global', 'claude', 'project-agent', 'project-agents'), range(4)))
    return ranks[tier] if tier in ranks else 4 + int(tier.split(':')[1])

def tier_scope(tier):
    return 'computer' if tier == 'global' else 'project' if tier.startswith('project') else 'external'

def tier_store(tier, workdir, create=True):
    if tier == 'global':
        return skill_store['global_']
    projects = skill_store['projects']
    if workdir not in projects:
        if not create:
            return {}
        projects[workdir] = {t: {} for t in ('claude', 'project-agent', 'project-agents', 'extra-root:0')}
    return projects[workdir].setdefault(tier, {})

def tiers_present(workdir):
    names = ['global']
    if workdir is not None:
        names += ['claude', 'project-agent', 'project-agents']
        names += sorted((t for t in skill_store['projects'].get(workdir, {}) if t.startswith('extra-root:')),
                        key=tier_rank)
    return names

def skill_generation(rec):
    raw = json.dumps([rec['description'], rec['triggers'], rec['tools'], rec['instruction'], rec['name']])
    return hashlib.sha256(raw.encode()).hexdigest()[:16]

def skill_path(tier, workdir, name):
    base = SKILL_HOME if tier == 'global' else '%s/%s' % (workdir.rstrip('/'), {
        'claude': '.claude/skills', 'project-agent': '.agent/skills',
        'project-agents': '.agents/skills'}.get(tier, 'extra-root-%s/skills' % tier.split(':')[-1]))
    return '%s/%s/SKILL.md' % (base, name)

def all_skills(workdir):
    """(tier, record) pairs for every tier visible from `workdir`, in discovery order."""
    pairs = []
    for tier in tiers_present(workdir):
        for name, rec in sorted(tier_store(tier, workdir, create=False).items()):
            pairs.append((tier, rec))
    return pairs

def winner_tier(workdir, name):
    tiers = [t for t, r in all_skills(workdir) if r['name'] == name]
    return max(tiers, key=tier_rank) if tiers else None

def skill_row(tier, rec, workdir):
    win = winner_tier(workdir, rec['name'])
    return dict(name=rec['name'], scope=tier_scope(tier), tier=tier, description=rec['description'],
                triggers=rec['triggers'][:64], triggers_truncated=len(rec['triggers']) > 64, tags=[],
                tags_truncated=False, pinned=False, editable=tier in ('global', 'project-agent', 'project-agents'),
                generation=skill_generation(rec), path=skill_path(tier, workdir, rec['name']),
                effective=win == tier, shadowed_by=None if win == tier else win)

def skill_raw(rec):
    head = ['---', 'name: ' + rec['name'], 'description: ' + rec['description']]
    if rec['triggers']:
        head.append('triggers: ' + json.dumps(rec['triggers']))
    return '\n'.join(head + ['---', rec['instruction']])

def skill_detail(tier, rec, workdir, raw=False):
    detail = skill_row(tier, rec, workdir)
    files = sorted(rec['companions'])
    detail.update(instruction=rec['instruction'], declared_tools=rec['tools'][:64],
                  declared_tools_truncated=len(rec['tools']) > 64, companion_files=files[:1000],
                  companion_files_truncated=len(files) > 1000, structured_save_supported=not rec['unsafe'],
                  raw=skill_raw(rec) if raw else None)
    return detail

def skills_propagation():
    return dict(skills='room_restart' if binding in ROOM_BINDINGS else 'next_rebuild')

def skills_workdir(rid, params):
    """(canonical workdir or None, error frame or None)."""
    value = params.get('workdir')
    if value is not None:
        if not is_abs(value):
            return None, invalid(rid, 'out_of_range', 'workdir must be an absolute path')
        canon = value.rstrip('/') or '/'
        if canon not in KNOWN_WORKDIRS and not (binding == 'coding' and canon == session['state']['cwd']):
            return None, invalid(rid, 'out_of_range', 'workdir must be an absolute existing directory')
        return canon, None
    if binding == 'coding':
        return session['state']['cwd'].rstrip('/') or '/', None
    if binding in ROOM_BINDINGS and bound_room is not None:
        return bound_room['row']['workdir'], None
    return None, None

def need_workdir(rid, workdir, tier=None, scope=None):
    project = (tier is not None and tier != 'global') or scope in ('project', 'external')
    if project and workdir is None:
        return failure(rid, 'invalid_request', 'workdir_required', 'this call needs a workdir')
    return None

def check_tier(rid, params, required):
    tier = params.get('tier')
    if tier is None:
        return (None, invalid(rid, 'out_of_range', 'tier is required')) if required else (None, None)
    if not isinstance(tier, str) or not TIER_RE.match(tier):
        return None, invalid(rid, 'out_of_range', 'tier is not a known tier')
    return tier, None

def check_skill_name(rid, name, strict):
    if not isinstance(name, str) or not name.strip() or CONTROL.search(name.strip()):
        return invalid(rid, 'invalid_name', 'name must be a nonempty name without control characters')
    if strict and (not SKILL_NAME.match(name) or len(name.encode()) > 64 or name in ('skill', 'readme')):
        return invalid(rid, 'invalid_name', 'name must be lowercase words joined by hyphens, at most 64 bytes')
    return None

def find_skill(rid, tier, name, workdir):
    """(tier, record, error): the skill of (tier, name), or the winner of name."""
    tier = tier or winner_tier(workdir, name)
    rec = tier_store(tier, workdir, create=False).get(name) if tier else None
    if rec is None:
        return None, None, failure(rid, 'not_found', 'not_found', 'no skill ' + name)
    return tier, rec, None

def read_only(rid, tier):
    return failure(rid, 'invalid_state', 'read_only', 'tier %s is read only' % tier)

def check_generation_param(rid, params):
    value = params.get('generation')
    if 'generation' in params and (not isinstance(value, str) or not value):
        return invalid(rid, 'out_of_range', 'generation must be a nonempty string')
    return None

def check_generation(rid, params, rec):
    if 'generation' in params and params['generation'] != skill_generation(rec):
        return failure(rid, 'invalid_state', 'stale', 'the skill changed; its generation is ' + skill_generation(rec))
    return None

def skills_list(rid, params):
    scope = params.get('scope', 'all')
    if scope not in ('computer', 'project', 'external', 'all'):
        return invalid(rid, 'out_of_range', 'scope must be computer, project, external or all')
    tier, error = check_tier(rid, params, False)
    if error:
        return error
    if 'include_diagnostics' in params and not isinstance(params['include_diagnostics'], bool):
        return invalid(rid, 'out_of_range', 'include_diagnostics must be a boolean')
    workdir, error = skills_workdir(rid, params)
    error = error or need_workdir(rid, workdir, tier, scope)
    if error:
        return error
    pairs = [(t, r) for t, r in all_skills(workdir) if (scope == 'all' or tier_scope(t) == scope)
             and (tier is None or t == tier)]
    page, cursor, error = paged(rid, pairs, lambda p: (tier_rank(p[0]), p[1]['name']), params, 500, 1000)
    if error:
        return error
    diagnostics = list(SKILL_DIAGNOSTICS) if a.scenario == 'skills' and params.get('include_diagnostics') else []
    return response(rid, dict(project_workdir=workdir, skills=[skill_row(t, r, workdir) for t, r in page],
                              diagnostics=diagnostics, diagnostics_truncated=False, next_cursor=cursor))

def skills_get(rid, params):
    # The adapter's order: workdir, tier, name, flags, then the tier's workdir need.
    workdir, error = skills_workdir(rid, params)
    if error:
        return error
    tier, error = check_tier(rid, params, False)
    error = error or check_skill_name(rid, params.get('name'), False)
    if error:
        return error
    if 'include_raw' in params and not isinstance(params['include_raw'], bool):
        return invalid(rid, 'out_of_range', 'include_raw must be a boolean')
    error = need_workdir(rid, workdir, tier)
    if error:
        return error
    tier, rec, error = find_skill(rid, tier, params['name'].strip(), workdir)
    return error or response(rid, skill_detail(tier, rec, workdir, params.get('include_raw', False)))

def skills_read_file(rid, params):
    workdir, error = skills_workdir(rid, params)
    if error:
        return error
    tier, error = check_tier(rid, params, False)
    error = error or check_skill_name(rid, params.get('name'), False)
    if error:
        return error
    path = params.get('path')
    if not isinstance(path, str) or not path:
        return invalid(rid, 'out_of_range', 'path is required')
    if path.startswith('/') or '..' in path.split('/') or is_abs(path) or CONTROL.search(path):
        return invalid(rid, 'out_of_range', 'path must be relative and inside the skill')
    error = check_generation_param(rid, params) or need_workdir(rid, workdir, tier)
    if error:
        return error
    tier, rec, error = find_skill(rid, tier, params['name'].strip(), workdir)
    error = error or check_generation(rid, params, rec)
    if error:
        return error
    if path not in rec['companions']:
        return failure(rid, 'not_found', 'not_found', 'no companion file ' + path)
    data = rec['companions'][path]
    raw = data if isinstance(data, bytes) else data.encode()
    try:
        text = raw.decode('utf-8')
    except UnicodeDecodeError:
        return invalid(rid, 'binary', path + ' is not UTF-8 text')
    return response(rid, dict(path=path, text=text, bytes=len(raw)))

def check_text_list(rid, value, field, count, size):
    if not isinstance(value, list) or len(value) > count or not all(
            isinstance(v, str) and 1 <= len(v) <= size for v in value):
        return invalid(rid, 'out_of_range', '%s must be at most %d strings of 1 to %d characters' % (field, count, size))
    return None

def check_skill_fields(rid, params):
    """Validate description, instruction, triggers and allowed_tools where present."""
    desc, instr = params.get('description'), params.get('instruction')
    if 'description' in params and (not isinstance(desc, str) or not 1 <= len(desc) <= 2000):
        return invalid(rid, 'out_of_range', 'description must be 1 to 2000 characters')
    if 'instruction' in params and (not isinstance(instr, str) or not 1 <= len(instr.encode()) <= 2097152):
        return invalid(rid, 'out_of_range', 'instruction must be 1 byte to 2 MiB')
    if 'triggers' in params:
        error = check_text_list(rid, params['triggers'], 'triggers', 32, 200)
        if error:
            return error
    if 'allowed_tools' in params:
        return check_text_list(rid, params['allowed_tools'], 'allowed_tools', 64, 100)
    return None

def new_skill_target(rid, params):
    """(tier, workdir, error) for skills.create and skills.install."""
    workdir, error = skills_workdir(rid, params)
    if error:
        return None, None, error
    scope = params.get('scope')
    if scope not in ('computer', 'project'):
        return None, None, invalid(rid, 'out_of_range', 'scope must be computer or project')
    tier = 'global' if scope == 'computer' else 'project-agents'
    return tier, workdir, need_workdir(rid, workdir, tier)

def skills_create(rid, params):
    tier, workdir, error = new_skill_target(rid, params)
    if error:
        return error
    error = check_skill_name(rid, params.get('name'), True)
    for key in ('description', 'instruction'):
        if key not in params and not error:
            error = invalid(rid, 'out_of_range', key + ' is required')
    error = error or check_skill_fields(rid, params)
    if error:
        return error
    store = tier_store(tier, workdir)
    if params['name'] in store:
        return failure(rid, 'invalid_state', 'exists', 'a skill named %s exists in tier %s' % (params['name'], tier))
    rec = skill_rec(params['name'], params['description'], params['instruction'], params.get('triggers', []),
                    params.get('allowed_tools', []))
    store[rec['name']] = rec
    return response(rid, dict(skill=skill_detail(tier, rec, workdir), propagation=skills_propagation()))

def address_writable(rid, params):
    """(tier, workdir, error) of an update or remove: workdir, tier, read-only,
    the tier's workdir need, then name and generation, in the adapter's order."""
    workdir, error = skills_workdir(rid, params)
    if error:
        return None, None, error
    tier, error = check_tier(rid, params, True)
    if error:
        return None, None, error
    if tier_scope(tier) == 'external':
        return None, None, read_only(rid, tier)
    error = (need_workdir(rid, workdir, tier) or check_skill_name(rid, params.get('name'), False)
             or check_generation_param(rid, params))
    return tier, workdir, error

def skills_update(rid, params):
    tier, workdir, error = address_writable(rid, params)
    if error:
        return error
    patch = params.get('patch')
    if 'patch' not in params or patch == {}:
        return invalid(rid, 'missing_params', 'patch must change at least one field')
    if not isinstance(patch, dict):
        return invalid(rid, 'out_of_range', 'patch must be an object')
    if 'name' in patch:
        error = check_skill_name(rid, patch['name'], True)
    error = error or check_skill_fields(rid, patch)
    if error:
        return error
    tier, rec, error = find_skill(rid, tier, params['name'].strip(), workdir)
    error = error or check_generation(rid, params, rec)
    if error:
        return error
    if rec['unsafe']:
        return failure(rid, 'invalid_state', 'unsafe_yaml', 'the frontmatter cannot be saved safely')
    store = tier_store(tier, workdir)
    new_name = patch.get('name', rec['name'])
    if new_name != rec['name'] and new_name in store:
        return failure(rid, 'invalid_state', 'exists', 'a skill named %s exists in tier %s' % (new_name, tier))
    updated = dict(rec, name=new_name, description=patch.get('description', rec['description']),
                   instruction=patch.get('instruction', rec['instruction']),
                   triggers=patch.get('triggers', rec['triggers']), tools=patch.get('allowed_tools', rec['tools']))
    del store[rec['name']]
    store[new_name] = updated
    return response(rid, dict(skill=skill_detail(tier, updated, workdir), renamed=new_name != rec['name'],
                              propagation=skills_propagation()))

def skills_remove(rid, params):
    tier, workdir, error = address_writable(rid, params)
    if error:
        return error
    tier, rec, error = find_skill(rid, tier, params['name'].strip(), workdir)
    error = error or check_generation(rid, params, rec)
    if error:
        return error
    del tier_store(tier, workdir)[rec['name']]
    now = winner_tier(workdir, rec['name'])
    effective = skill_row(now, tier_store(now, workdir)[rec['name']], workdir) if now else None
    return response(rid, dict(removed=True, now_effective=effective, propagation=skills_propagation()))

def read_package(rid, params):
    """The zip bytes of a skills.install call, or an error frame."""
    has_data, has_path = 'data_b64' in params, 'path' in params
    if has_data == has_path:
        return invalid(rid, 'conflicting_params', 'give exactly one of data_b64 and path')
    if has_data:
        data = params['data_b64']
        if not isinstance(data, str):
            return invalid(rid, 'out_of_range', 'data_b64 must be a string')
        if len(data) > 39146840:
            return invalid(rid, 'too_large', 'data_b64 is too long')
        try:
            return base64.b64decode(data, validate=True)
        except ValueError:
            return invalid(rid, 'invalid_package', 'data_b64 is not valid base64')
    path = params['path']
    if not is_abs(path):
        return invalid(rid, 'out_of_range', 'path must be absolute')
    if os.path.islink(path) or not os.path.isfile(path):
        return failure(rid, 'not_found', 'not_found', 'no regular file ' + path)
    if os.path.getsize(path) > 67108864:
        return invalid(rid, 'too_large', 'the package is larger than 64 MiB')
    with open(path, 'rb') as handle:
        return handle.read()

def parse_package(rid, raw, name):
    """(record or None, error frame or None) for a zip holding one skill directory."""
    try:
        archive = zipfile.ZipFile(io.BytesIO(raw))
    except zipfile.BadZipFile:
        return None, invalid(rid, 'invalid_package', 'the package is not a zip file')
    names = [n for n in archive.namelist() if not n.endswith('/')]
    if any(n.startswith('__MACOSX/') for n in names):
        return None, invalid(rid, 'invalid_package', 'remove the __MACOSX/ directory from the package')
    if 'skill.md' in names:
        return None, invalid(rid, 'invalid_package', 'the manifest must be named SKILL.md, not skill.md')
    tops = sorted({n.split('/')[0] for n in names if '/' in n})
    if len(tops) != 1:
        if name is None:
            return None, invalid(rid, 'invalid_name', 'the package needs exactly one top-level directory')
        return None, invalid(rid, 'invalid_package', 'the package needs exactly one top-level directory')
    top = tops[0]
    manifest = top + '/SKILL.md'
    if manifest not in names:
        return None, invalid(rid, 'invalid_package', 'the package has no ' + manifest)
    text = archive.read(manifest).decode('utf-8', 'replace')
    match = re.match(r'^---\n(.*?)\n---\n?(.*)$', text, re.S)
    head = dict(re.findall(r'^([a-z_]+):\s*(.*)$', match.group(1), re.M)) if match else {}
    if not head.get('description'):
        return None, invalid(rid, 'invalid_package', 'the manifest has no description')
    companions = {n[len(top) + 1:]: archive.read(n) for n in names if n.startswith(top + '/') and n != manifest}
    return skill_rec(name or top, head['description'], match.group(2), companions=companions), None

def skills_install(rid, params):
    tier, workdir, error = new_skill_target(rid, params)
    if error:
        return error
    if 'name' in params:
        error = check_skill_name(rid, params['name'], True)
        if error:
            return error
    raw = read_package(rid, params)
    if isinstance(raw, dict):
        return raw
    rec, error = parse_package(rid, raw, params.get('name'))
    if error:
        return error
    error = check_skill_name(rid, rec['name'], True)
    if error:
        return error
    store = tier_store(tier, workdir)
    if rec['name'] in store:
        return failure(rid, 'invalid_state', 'exists', 'a skill named %s exists in tier %s' % (rec['name'], tier))
    store[rec['name']] = rec
    return response(rid, dict(skill=skill_detail(tier, rec, workdir), propagation=skills_propagation()))

SKILL_HANDLERS = {'skills.list': skills_list, 'skills.get': skills_get, 'skills.read_file': skills_read_file,
                  'skills.install': skills_install, 'skills.remove': skills_remove, 'skills.create': skills_create,
                  'skills.update': skills_update}

# The room's shelf, as the adapter would read it from disk.
def seed_shelf():
    shelf = {}
    def put(key, kind, title, status, content, record=None, age=0):
        shelf[key] = dict(key=key, revision=1 + len(shelf) % 3, updated_at=NOW - age, kind=kind, title=title,
                          status=status, content=content, json=record)
    put('findings/f-001', 'finding', 'SQL injection in /login', 'verified', 'Payload: a\' OR 1=1 --', None, 900)
    put('findings/f-002', 'finding', 'Open redirect on /out', 'open', 'The next parameter is not checked.', None, 800)
    put('findings/f-003', 'finding', 'Weak cipher suite', 'returned', 'TLS 1.0 is enabled.', None, 700)
    put('notes/readme', 'note', 'Scope notes', '', 'Only the web tier is in scope.', None, 2000)
    put('notes/spend/web-recon', 'spend', 'Spend for web-recon', '', '', dict(goal='web-recon'), 600)
    put('reports/r-001', 'report', 'Web recon report', 'final', '# Web recon\n\nOne high finding.\n',
        dict(title='Web recon report', audience='client', severity_summary='1 high, 1 low', go_no_go='no-go',
             finding_keys=['findings/f-001', 'findings/f-003']), 500)
    put('reports/r-002', 'report', 'Interim note', 'draft', '# Interim\n',
        dict(title='Interim note', audience='team', severity_summary='none yet', go_no_go='open',
             finding_keys=[]), 1500)
    put('trajectory/web-recon', 'goal', 'Map the web tier', 'active', '', dict(goal='web-recon'), 3000)
    return shelf

SHELF = seed_shelf()
GOAL = dict(goal_key='trajectory/web-recon', counts=dict(open=1, verified=1, returned=1, reports=2), columns=3, rows=2,
            nodes=[dict(id='n1', label='Map the web tier', kind='goal', status='active', severity=None, note=None,
                        resource_key='trajectory/web-recon', revision=1, rank=0, row=0),
                   dict(id='n2', label='SQL injection in /login', kind='finding', status='verified',
                        severity='high', note='confirmed by hand', resource_key='findings/f-001', revision=1,
                        rank=1, row=0),
                   dict(id='n3', label='Open redirect on /out', kind='finding', status='open', severity='low',
                        note=None, resource_key='findings/f-002', revision=2, rank=1, row=1)],
            nodes_truncated=False,
            edges=[dict(**{'from': 'n1', 'to': 'n2', 'rel': 'found'}), dict(**{'from': 'n1', 'to': 'n3', 'rel': None})],
            edges_truncated=False)
GOAL_SPEND = dict(goal_key='trajectory/web-recon',
                  total=dict(calls=6, prompt_tokens=9000, completion_tokens=1700, cost=0.11),
                  by_avatar=[dict(avatar='chief', calls=2, prompt_tokens=5000, completion_tokens=900, cost=0.07),
                             dict(avatar='scout', calls=4, prompt_tokens=4000, completion_tokens=800, cost=0.04)],
                  by_avatar_truncated=False)

def shelf_fingerprint():
    return hashlib.sha256(json.dumps(SHELF, sort_keys=True).encode()).hexdigest()[:16]

def shelf_row(item):
    return {k: item[k] for k in ('key', 'revision', 'updated_at', 'kind', 'title', 'status')}

# Room team: three layers over embedded roles. A layer is {"avatars": {id: {role, display, alias, lanes}}}.
BASE_ROLES = {
    'chief': dict(display='Chief', shelf_allow=['findings/', 'notes/', 'reports/'], skills_allow=None,
                  chat=dict(provider_uuid='prov-local', model='model-main', effort=None),
                  work=dict(provider_uuid=None, model=None, effort=None)),
    'clerk': dict(display='Clerk', shelf_allow=['notes/'], skills_allow=[],
                  chat=dict(provider_uuid=None, model=None, effort=None),
                  work=dict(provider_uuid='prov-local', model='model-fast', effort=None)),
    'scout': dict(display='Scout', shelf_allow=['findings/', 'notes/'], skills_allow=['code-review'],
                  chat=dict(provider_uuid='prov-local', model='model-fast', effort=None),
                  work=dict(provider_uuid='prov-local', model='model-fast', effort='low')),
    'analyst': dict(display='Analyst', shelf_allow=['findings/', 'reports/'], skills_allow=None,
                    chat=dict(provider_uuid='prov-local', model='model-main', effort=None),
                    work=dict(provider_uuid='prov-local', model='model-deep', effort='high')),
}
BASE_LAYER = dict(avatars={
    'chief': dict(role='chief', display='Chief', alias='CH'), 'clerk': dict(role='clerk', display='Clerk', alias='CK'),
    'scout': dict(role='scout', display='Scout', alias='SC'),
    'analyst': dict(role='analyst', display='Analyst', alias='AN')})
CHOICES = [
    dict(id='model-main', name='Main model', model_id='fake/main', provider_name='local', provider_uuid='prov-local'),
    dict(id='model-fast', name='Fast model', model_id='fake/fast', provider_name='local', provider_uuid='prov-local'),
    dict(id='model-deep', name='Deep model', model_id='fake/deep', provider_name='local', provider_uuid='prov-local'),
    dict(id='model-bare', name='Bare model', model_id='bare/model', provider_name=None, provider_uuid=None),
]
template_layer = dict(avatars={'recon': dict(role='scout', display='Recon', alias='RC')}) if WITH_WORLD else dict(avatars={})
LAYER_ORDER = ('base', 'template', 'session')

def layer_revision(layer):
    if not layer['avatars']:
        return 'absent'
    return hashlib.sha256(json.dumps(layer, sort_keys=True, separators=(',', ':')).encode()).hexdigest()[:16]

def layers_for(target):
    if target == 'room':
        return [('base', BASE_LAYER), ('template', template_layer), ('session', bound_room['layer'])]
    return [('base', BASE_LAYER), ('template', template_layer)]

def target_layer(target):
    return template_layer if target == 'template' else bound_room['layer']

def avatar_lanes(role):
    return ['chat'] if role == 'chief' else ['work'] if role == 'clerk' else ['chat', 'work']

def effective_avatars(layers):
    """{id: {role, defined_in}} in first-definition order."""
    found = {}
    for name, layer in layers:
        for aid, entry in layer['avatars'].items():
            if aid not in found and entry.get('role'):
                found[aid] = dict(role=entry['role'], defined_in=name)
    return found

def field_of(layers, aid, key):
    value, origin = None, 'default'
    for name, layer in layers:
        entry = layer['avatars'].get(aid)
        if entry and entry.get(key) is not None:
            value, origin = entry[key], name
    return dict(value=value, origin=origin)

def lane_view(layers, aid, role, lane):
    state = dict(provider_uuid=BASE_ROLES[role][lane]['provider_uuid'], model=BASE_ROLES[role][lane]['model'],
                 effort=BASE_ROLES[role][lane]['effort'])
    origin = {k: 'role' if state[k] is not None else 'default' for k in state}
    for name, layer in layers:
        entry = layer['avatars'].get(aid, {}).get('lanes', {}).get(lane)
        if not entry:
            continue
        if entry.get('model') is not None:
            for key in state:
                state[key], origin[key] = entry.get('provider' if key == 'provider_uuid' else key), name
        elif entry.get('effort') is not None:
            state['effort'], origin['effort'] = entry['effort'], name
    choice = next((c for c in CHOICES if state['model'] in (c['id'], c['model_id'])), None)
    fields = {k: dict(value=state[k], origin=origin[k]) for k in state}
    return dict(lane=lane, model_label=choice['name'] if choice else None,
                resolves=state['model'] is None or choice is not None, **fields)

def avatar_view(layers, target, aid, info):
    display, alias = field_of(layers, aid, 'display'), field_of(layers, aid, 'alias')
    entry = layers[-1][1]['avatars'].get(aid)
    return dict(id=aid, role=info['role'], defined_in=info['defined_in'],
                removable=entry is not None,
                display=display, display_effective=display['value'] or aid.capitalize(), alias=alias,
                alias_effective=alias['value'] or aid[:2].upper(),
                lanes=[lane_view(layers, aid, info['role'], lane) for lane in avatar_lanes(info['role'])])

def team_view(target):
    layers = layers_for(target)
    found = effective_avatars(layers)
    frozen = bound_room['frozen_template'] if target == 'room' and bound_room else None
    roles = [dict(id=rid, display=r['display'], shelf_allow=r['shelf_allow'], skills_allow=r['skills_allow'],
                  chat=r['chat'], work=r['work'], defined_in='base') for rid, r in BASE_ROLES.items()]
    return dict(target=target, revision=layer_revision(target_layer(target)),
                layer_present=bool(target_layer(target)['avatars']), template_revision=layer_revision(template_layer),
                template_pending_restart=frozen is not None and frozen != layer_revision(template_layer),
                avatars=[avatar_view(layers, target, aid, info) for aid, info in found.items()],
                avatars_truncated=False, roles=roles, roles_truncated=False, choices=[dict(c) for c in CHOICES],
                choices_truncated=False, choices_source='room_daemon' if binding == 'room_live' and target == 'room'
                else 'config', warnings=[], warnings_truncated=False)

def roster():
    return list(effective_avatars(layers_for('room')))

def display_of(author):
    found = effective_avatars(layers_for('room')) if bound_room is not None else {}
    if author not in found:
        return None, None
    layers = layers_for('room')
    display, alias = field_of(layers, author, 'display'), field_of(layers, author, 'alias')
    return display['value'] or author.capitalize(), alias['value'] or author[:2].upper()

# One state per room the adapter has bound; the line ring and epoch survive into ROOM(closed).
bound_room = None
epoch_counter = 0
scripted_done = False

def seed_lines(modern):
    lines = [
        ('user', 'you', 'Map the login flow of the target', None, None, {}),
        ('chief', 'chief', 'Plan: recon first, then analysis.', None, 'team', {}),
        ('visible', 'room', 'Scout started on web-recon', None, None, {}),
        ('lane_chat', 'scout', 'I found /login and /admin.', 'chat', 'user', {}),
        ('reasoning', 'scout', 'Check the login form for injection.', 'work', 'team', {}),
        ('tool_call', 'scout', None, 'work', 'team', dict(call_id='call-1', tool='http_get',
                                                           args='{"url":"https://target.test/login"}')),
        ('tool_result', 'scout', None, 'work', 'team', dict(call_id='call-1', ok=True, result='200 OK, 4 forms')),
        ('shell', 'shell', '!ls /srv', None, None, {}),
        ('visible', 'analyst', 'Finding f-001 recorded.', None, None, {}),
        ('chief', 'chief', 'One finding is verified.', None, 'user', {}),
    ]
    return lines if modern else lines[:3]

def make_line(st, kind, author, text=None, lane=None, audience=None, replay=False, origin='room', **extra):
    seq_no = st['next_seq']
    st['next_seq'] += 1
    display, alias = display_of(author)
    line = dict(line_seq=seq_no, kind=kind, author=author, display=display, alias=alias, lane=lane,
                audience=audience, text=text, call_id=None, tool=None, args=None, ok=None, result=None,
                replay=replay, origin=origin, received_at_ms=NOW_MS - 60000 + seq_no * 100, truncated=False)
    line.update(extra)
    st['ring'].append(line)
    if len(st['ring']) > 8000:
        st['ring'].pop(0)
    return line

def lane_rows(st):
    layers = layers_for('room')
    rows = []
    for aid, info in effective_avatars(layers).items():
        display, alias = field_of(layers, aid, 'display'), field_of(layers, aid, 'alias')
        for lane in avatar_lanes(info['role']):
            running = (aid, lane) in (('scout', 'work'), ('probe', 'work'))
            phase = 'paused' if aid in st['paused'] else 'running' if running else 'idle'
            rows.append(dict(avatar=aid, display=display['value'] or aid.capitalize(),
                             alias=alias['value'] or aid[:2].upper(), role=info['role'], lane=lane, phase=phase,
                             task='scan /login' if running else '', started_at_ms=NOW_MS - 120000 if running else 0,
                             last_event_at_ms=NOW_MS - 2000, steps=4 if running else 0,
                             last_tool='http_get' if running else '', error=''))
    return rows

def shelf_counts():
    findings = [i for k, i in SHELF.items() if k.startswith('findings/')]
    return dict(open=sum(i['status'] == 'open' for i in findings),
                verified=sum(i['status'] == 'verified' for i in findings),
                returned=sum(i['status'] == 'returned' for i in findings),
                reports=sum(k.startswith('reports/') for k in SHELF))

def status_of(st, live, fresh):
    return dict(session_id=st['row']['id'], live=live, fresh=fresh and live, age_ms=(0 if fresh else 90000) if live else None,
                now_at_ms=NOW_MS, counts=shelf_counts(), counts_basis='first_400_items' if live else 'disk_scan',
                paused=sorted(st['paused']), paused_truncated=False, lanes=lane_rows(st) if live else [],
                lanes_truncated=False,
                events=[dict(at_ms=NOW_MS - 60000, avatar='scout', marker='start', label='scout started')] if live else [],
                digest='3 findings, 1 verified', timeline='scout started', trajectory_outline='web-recon: 3 nodes',
                shelf_fingerprint=shelf_fingerprint(), truncated=False)

def room_event(name, **body):
    global seq
    seq += 1
    return dict(type='event', event=name, seq=seq, epoch=bound_room['epoch'], **body)

def status_event():
    return room_event('room.status', status=status_of(bound_room, binding == 'room_live', bound_room['fresh']))

def attach_room(row, started, tail, wait_ms):
    """Bind `row` as a live room under a new epoch; the events of the scenario follow the response."""
    global binding, bound_room, epoch_counter, scripted_done
    modern = row.get('modern', True)
    epoch_counter += 1
    st = dict(row=row, modern=modern, features=['sdk_user', 'sdk_reload'] if modern else [], ring=[], next_seq=1,
              epoch=str(uuid.uuid5(uuid.NAMESPACE_OID, 'epoch-%d' % epoch_counter)), paused=set(),
              fresh=wait_ms > 0, layer=dict(avatars={}), frozen_template=layer_revision(template_layer) if modern else None)
    if row['id'] == room_session_id('sess-coding-a'):
        st['layer'] = dict(avatars={'probe': dict(role='analyst', display='Probe', alias='PR',
                                                 lanes=dict(chat=dict(provider='prov-local', model='model-fast')))})
    bound_room = st
    binding = 'room_live'
    row.update(live=True, locked=True)
    for kind, author, text, lane, audience, extra in seed_lines(modern):
        make_line(st, kind, author, text, lane, audience, replay=True, **extra)
    last = st['ring'][-1]['line_seq']
    result = dict(room=room_info(row), started=started,
                  daemon=dict(version='1.0.0-fake' if modern else '0.9.0-fake', features=list(st['features']),
                              template_revision=st['frozen_template']),
                  epoch=st['epoch'], history=dict(first_line_seq=st['ring'][0]['line_seq'], last_line_seq=last,
                                                  count=len(st['ring']), replayed=len(st['ring'])),
                  tail=copy.deepcopy(st['ring'][max(len(st['ring']) - tail, 0):] if tail else []),
                  status=status_of(st, True, st['fresh']), roster=roster(), roster_truncated=False)
    follow = []
    if a.scenario in ('room', 'room_gap', 'room_takeover') and (a.scenario == 'room' or not scripted_done):
        follow = scripted_events(st)
        scripted_done = True
    return result, follow

def scripted_events(st):
    """The events a scenario emits right after an attach response."""
    global binding
    frames = [status_event()]
    if a.scenario == 'room':
        for piece in ('Looking at ', 'the admin panel'):
            frames.append(room_event('room.delta', author='scout', lane='chat', kind='text', text=piece))
        line = make_line(st, 'lane_chat', 'scout', 'Looking at the admin panel', 'chat', 'user')
        frames.append(room_event('room.line', line=copy.deepcopy(line)))
        frames.append(room_event('room.notice', code='stalled', text='no status for 10 seconds'))
        frames.append(room_event('room.notice', code='recovered', text='status resumed'))
    elif a.scenario == 'room_gap':
        line = make_line(st, 'lane_chat', 'scout', 'Found /admin.', 'chat', 'user')
        frames.append(room_event('room.line', line=copy.deepcopy(line)))
        for text in ('Checking /admin.', 'Admin needs a login.', 'Trying default credentials.'):
            make_line(st, 'lane_chat', 'scout', text, 'chat', 'user')
        frames.append(room_event('room.gap', after_line_seq=line['line_seq'], dropped_lines=3, dropped_deltas=2))
    else:
        frames.append(room_event('room.closed', session_id=st['row']['id'], reason='taken_over'))
        binding = 'room_closed'
    return frames

def room_list(rid, params):
    if 'live' in params and not isinstance(params['live'], bool):
        return invalid(rid, 'out_of_range', 'live must be a boolean')
    if 'parent_session_id' in params and not is_id(params['parent_session_id']):
        return invalid(rid, 'out_of_range', 'parent_session_id must be a string of 1 to 64 characters')
    rows = [r for r in registry if r['kind'] == 'room' and params.get('live', r['live']) == r['live']
            and params.get('parent_session_id', r['parent']) == r['parent']]
    page, cursor, error = paged(rid, rows, session_sort, params, 100, 500)
    return error or response(rid, dict(rooms=[room_info(r) for r in page], next_cursor=cursor))

def check_attach_params(rid, params, wait_max, wait_default):
    tail, wait = params.get('tail', 50), params.get('wait_ms', wait_default)
    if not is_int(tail) or not 0 <= tail <= 200:
        return None, None, invalid(rid, 'out_of_range', 'tail must be an integer from 0 to 200')
    if not is_int(wait) or not 0 <= wait <= wait_max:
        return None, None, invalid(rid, 'out_of_range', 'wait_ms must be an integer from 0 to %d' % wait_max)
    return tail, wait, None

def rebind_response(rid, result, follow):
    after.extend(follow)
    return response(rid, result)

def room_open(rid, params):
    parent = params.get('parent_session_id')
    if not is_id(parent):
        return invalid(rid, 'out_of_range', 'parent_session_id must be a string of 1 to 64 characters')
    tail, wait, error = check_attach_params(rid, params, 30000, 12000)
    if error:
        return error
    prow = find_row(parent)
    if prow is None or not prow['registered']:
        return failure(rid, 'not_found', 'not_found', 'no session ' + parent)
    if prow['kind'] == 'room':
        return invalid(rid, 'parent_is_room', 'the parent session is itself a room')
    room_id = room_session_id(parent)
    row = find_row(room_id)
    if row is not None and row['kind'] == 'coding':
        return failure(rid, 'invalid_state', 'room_id_in_use', 'the room id belongs to a coding session')
    started = False
    if row is None or not row['live']:
        if prow.get('main_ok') is False:
            return failure(rid, 'invalid_state', 'no_main_route', 'the parent session has no usable Main model')
        if prow.get('no_start'):
            return failure(rid, 'unavailable', 'room_start_failed', 'the room did not come up in time',
                           delivery='unknown', detail=dict(log_tail='fake room: no main route could be resolved\n'))
        if row is None:
            row = registry_row(room_id, prow['name'] and prow['name'] + ' room', prow['workdir'], kind='room',
                               parent=parent, modern=True)
            row['created'] = NOW
            registry.append(row)
        started = True
    row['updated'] = NOW
    result, follow = attach_room(row, started, tail, wait)
    return rebind_response(rid, result, follow)

def room_attach(rid, params):
    sid = params.get('session_id')
    if not is_id(sid):
        return invalid(rid, 'out_of_range', 'session_id must be a string of 1 to 64 characters')
    tail, wait, error = check_attach_params(rid, params, 15000, 2000)
    if error:
        return error
    row = find_row(sid)
    if row is None or not row['live']:
        return failure(rid, 'invalid_state', 'not_running', 'the room is not running; use room.open')
    if row['kind'] != 'room':
        return failure(rid, 'invalid_state', 'not_a_room', 'the socket did not answer as a room')
    result, follow = attach_room(row, False, tail, wait)
    return rebind_response(rid, result, follow)

def room_close(rid, params):
    global binding, bound_room
    if 'stop' in params and not isinstance(params['stop'], bool):
        return invalid(rid, 'out_of_range', 'stop must be a boolean')
    row = bound_room['row']
    stopped = bool(params.get('stop', False))
    if stopped:
        row.update(live=False, locked=False)
    elif binding == 'room_live':
        row['locked'] = False
    binding, bound_room = 'unbound', None
    return response(rid, dict(closed=True, stopped=stopped))

def check_text(rid, params):
    text = params.get('text')
    if not isinstance(text, str) or not text.strip() or len(text.encode()) > 2097152:
        return invalid(rid, 'out_of_range', 'text must be nonblank and at most 2 MiB')
    return None

def check_attachments(rid, value):
    if not isinstance(value, list) or len(value) > 16:
        return invalid(rid, 'out_of_range', 'attachments must be a list of at most 16 items')
    for item in value:
        if not isinstance(item, dict) or item.get('kind') not in ('image_file', 'pasted_text'):
            return invalid(rid, 'out_of_range', 'attachment kind must be image_file or pasted_text')
        if item['kind'] == 'image_file' and not is_abs(item.get('path')):
            return invalid(rid, 'out_of_range', 'image_file needs an absolute path')
        if item['kind'] == 'pasted_text' and not isinstance(item.get('text'), str):
            return invalid(rid, 'out_of_range', 'pasted_text needs text')
    if sum(len(i.get('text', '').encode()) for i in value) > 2097152:
        return invalid(rid, 'out_of_range', 'pasted text is larger than 2 MiB')
    return None

def avatar_check(rid, name):
    if name == 'clerk':
        return invalid(rid, 'clerk_not_addressable', 'the clerk cannot be addressed')
    if name not in roster():
        return failure(rid, 'not_found', 'unknown_avatar', 'no avatar ' + str(name))
    return None

def stream_error(rid, message):
    """A socket write with an unknown outcome closes the room binding and keeps the process."""
    global binding
    binding = 'room_closed'
    after.append(room_event('room.closed', session_id=bound_room['row']['id'], reason='stream_error'))
    return failure(rid, 'unavailable', 'stream_error', message, delivery='unknown')

def room_send(rid, params):
    error = check_text(rid, params)
    if error:
        return error
    shell, to = params.get('shell', False), params.get('to')
    if not isinstance(shell, bool):
        return invalid(rid, 'out_of_range', 'shell must be a boolean')
    attachments = params.get('attachments')
    if attachments is not None:
        error = check_attachments(rid, attachments)
        if error:
            return error
    if shell and (to is not None or attachments):
        return invalid(rid, 'conflicting_params', 'shell excludes to and attachments')
    if to is not None:
        if not isinstance(to, str):
            return invalid(rid, 'out_of_range', 'to must be an avatar id')
        error = avatar_check(rid, to)
        if error:
            return error
    modern = 'sdk_user' in bound_room['features']
    if attachments and not modern:
        return failure(rid, 'invalid_state', 'peer_unsupported', 'the room does not support attachments')
    if params['text'] == 'fake:stream_error':
        return stream_error(rid, 'the room socket failed during the write')
    if any(i['kind'] == 'image_file' and 'refused' in i['path'] for i in attachments or []):
        return invalid(rid, 'attachment_refused', 'the room refused the attachment')
    wire = '!' + params['text'] if shell else '@%s %s' % (to, params['text']) if to is not None else params['text']
    inserts = ['[Image #%d]' % n if i['kind'] == 'image_file' else '[Pasted text #%d]' % n
               for n, i in enumerate(attachments or [], 1)] if modern else []
    line = make_line(bound_room, 'shell' if shell else 'user', 'shell' if shell else 'you', wire,
                     origin='adapter_echo')
    emit(room_event('room.line', line=copy.deepcopy(line)))
    return response(rid, dict(sent=True, acked=modern, inserts=inserts, echo_line_seq=line['line_seq']))

def room_interrupt(rid, params):
    return response(rid, dict(sent=True))

def room_control(rid, params):
    action, avatar, note = params.get('action'), params.get('avatar'), params.get('note')
    if action not in ('pause', 'resume', 'stop', 'redirect'):
        return invalid(rid, 'out_of_range', 'action must be pause, resume, stop or redirect')
    if not isinstance(avatar, str) or not avatar:
        return invalid(rid, 'out_of_range', 'avatar must be an avatar id')
    if action == 'redirect' and note is None:
        return invalid(rid, 'note_required', 'redirect needs a note')
    if action != 'redirect' and note is not None:
        return invalid(rid, 'conflicting_params', 'note is only allowed with redirect')
    if note is not None and (not isinstance(note, str) or not 1 <= len(note) <= 8192):
        return invalid(rid, 'out_of_range', 'note must be 1 to 8192 characters')
    if avatar == 'chief':
        return invalid(rid, 'no_work_lane', 'the chief has no work lane')
    if avatar not in roster():
        return failure(rid, 'not_found', 'unknown_avatar', 'no avatar ' + avatar)
    if action == 'pause':
        bound_room['paused'].add(avatar)
    elif action == 'resume':
        bound_room['paused'].discard(avatar)
    after.append(status_event())
    return response(rid, dict(sent=True))

def room_status(rid, params):
    live = binding == 'room_live'
    return response(rid, status_of(bound_room, live, bound_room['fresh']))

def room_history(rid, params):
    epoch = params.get('epoch')
    if not isinstance(epoch, str):
        return invalid(rid, 'missing_params', 'epoch is required')
    limit = params.get('limit', 100)
    if not is_int(limit) or not 1 <= limit <= 200:
        return invalid(rid, 'out_of_range', 'limit must be an integer from 1 to 200')
    for key in ('after_line_seq', 'before_line_seq'):
        if key in params and not is_int(params[key]):
            return invalid(rid, 'out_of_range', key + ' must be an integer')
    if 'after_line_seq' in params and 'before_line_seq' in params:
        return invalid(rid, 'conflicting_params', 'give at most one of after_line_seq and before_line_seq')
    kinds = params.get('kinds')
    if kinds is not None and (not isinstance(kinds, list) or not all(isinstance(k, str) for k in kinds)):
        return invalid(rid, 'out_of_range', 'kinds must be a list of strings')
    if 'author' in params and not isinstance(params['author'], str):
        return invalid(rid, 'out_of_range', 'author must be a string')
    if epoch != bound_room['epoch']:
        return failure(rid, 'invalid_state', 'epoch_changed', 'the epoch is not the current binding\'s')
    ring = bound_room['ring']
    pool = [l for l in ring if (kinds is None or l['kind'] in kinds) and params.get('author', l['author']) == l['author']]
    if 'after_line_seq' in params:
        later = [l for l in pool if l['line_seq'] > params['after_line_seq']]
        page, before, after_more = later[:limit], len(later) < len(pool), len(later) > limit
    elif 'before_line_seq' in params:
        earlier = [l for l in pool if l['line_seq'] < params['before_line_seq']]
        page = earlier[max(len(earlier) - limit, 0):]
        before, after_more = len(earlier) > limit, len(earlier) < len(pool)
    else:
        page = pool[max(len(pool) - limit, 0):]
        before, after_more = len(pool) > limit, False
    return response(rid, dict(epoch=epoch, first_line_seq=ring[0]['line_seq'] if ring else None,
                              last_line_seq=ring[-1]['line_seq'] if ring else None, lines=copy.deepcopy(page),
                              has_more_before=before, has_more_after=after_more))

def room_shelf_list(rid, params):
    prefix = params.get('prefix', '')
    if not isinstance(prefix, str) or len(prefix) > 200:
        return invalid(rid, 'out_of_range', 'prefix must be at most 200 characters')
    rows = [i for k, i in SHELF.items() if k.startswith(prefix)]
    page, cursor, error = paged(rid, rows, lambda i: (i['key'],), params, 200, 500)
    return error or response(rid, dict(rows=[shelf_row(i) for i in page], total=len(rows), next_cursor=cursor,
                                       fingerprint=shelf_fingerprint()))

def room_item(rid, params):
    key = params.get('key')
    if not isinstance(key, str) or not 1 <= len(key) <= 512:
        return invalid(rid, 'out_of_range', 'key must be 1 to 512 characters')
    if key.startswith('/') or key.endswith('/') or '..' in key.split('/') or re.search(r'\s', key):
        return invalid(rid, 'invalid_name', 'the key breaks the key grammar')
    item = SHELF.get(key)
    if item is None:
        return failure(rid, 'not_found', 'not_found', 'no shelf item ' + key)
    return response(rid, dict(shelf_row(item), content=item['content'], json=copy.deepcopy(item['json']),
                              truncated=False))

def goal_name(rid, params):
    goal = params.get('goal')
    if goal is not None and not isinstance(goal, str):
        return None, invalid(rid, 'out_of_range', 'goal must be a string')
    return (goal[len('trajectory/'):] if goal and goal.startswith('trajectory/') else goal), None

def room_trajectory(rid, params):
    goal, error = goal_name(rid, params)
    if error:
        return error
    goals = [copy.deepcopy(GOAL)] if goal in (None, 'web-recon') else []
    return response(rid, dict(goals=goals, goals_truncated=False))

def room_reports(rid, params):
    if 'include_markdown' in params and not isinstance(params['include_markdown'], bool):
        return invalid(rid, 'out_of_range', 'include_markdown must be a boolean')
    rows = [i for k, i in SHELF.items() if k.startswith('reports/')]
    page, cursor, error = paged(rid, rows, lambda i: (-i['updated_at'], i['key']), params, 50, 100)
    if error:
        return error
    reports = []
    for item in page:
        record = item['json']
        reports.append(dict(key=item['key'], revision=item['revision'], updated_at=item['updated_at'],
                            title=record['title'], audience=record['audience'],
                            severity_summary=record['severity_summary'], go_no_go=record['go_no_go'],
                            finding_keys=list(record['finding_keys']), finding_keys_truncated=False,
                            markdown=item['content'] if params.get('include_markdown') else None))
    return response(rid, dict(reports=reports, next_cursor=cursor))

def room_spend(rid, params):
    goal, error = goal_name(rid, params)
    if error:
        return error
    rows = [r for r in ledger if r[1] == bound_room['row']['id']]
    by_avatar = {}
    for r in rows:
        by_avatar.setdefault('chief' if r[3] == 'main' else r[3][len('sub:'):], []).append(r)
    ledger_part = dict(ledger='ok' if WITH_LEDGER else 'missing', totals=totals_of(rows),
                       by_avatar=[dict(avatar=k, totals=totals_of(v)) for k, v in sorted(by_avatar.items())],
                       by_avatar_truncated=False)
    goals = [copy.deepcopy(GOAL_SPEND)] if goal in (None, 'web-recon') else []
    return response(rid, dict(goals=goals, goals_truncated=False, ledger=ledger_part))

def team_target(rid, method, params, write):
    """(target, error frame): the validated target, or the binding error for a room target."""
    target = params.get('target')
    if target is None and write:
        return None, invalid(rid, 'target_required', 'target is required')
    if target is None:
        target = 'room' if binding in ROOM_BINDINGS else 'template'
    if target not in ('template', 'room'):
        return None, invalid(rid, 'out_of_range', 'target must be template or room')
    params['target'] = target
    return target, binding_denial(rid, method, params)

def reject_layer(rid, target, aid, candidate):
    """layer_rejected when `candidate` makes two avatars share an alias that was not shared before."""
    before = layers_for(target)
    now = [(n, candidate if l is target_layer(target) else l) for n, l in before]
    def clashes(layers):
        seen, bad = {}, set()
        for avatar in effective_avatars(layers):
            alias = field_of(layers, avatar, 'alias')['value']
            if alias is None:
                continue
            alias = alias.upper()
            if alias in seen:
                bad.update((avatar, seen[alias]))
            seen[alias] = avatar
        return bad
    new = clashes(now) - clashes(before)
    if aid in new:
        warning = dict(layer='template' if target == 'template' else 'session', path='avatars.toml',
                       message='alias of %s is already used by another avatar' % aid)
        return failure(rid, 'invalid_request', 'layer_rejected', 'the candidate layer is rejected',
                       detail=dict(warnings=[warning]))
    return None

def commit_team(rid, target, candidate, extra=None):
    layer = target_layer(target)
    layer['avatars'] = candidate['avatars']
    if target == 'template':
        applies, room = 'next_start', 'not_live'
    elif binding == 'room_live':
        applies, room = ('live', 'reloaded') if 'sdk_reload' in bound_room['features'] else ('next_reload', 'not_supported')
    else:
        applies, room = 'next_reload', 'not_live'
    result = dict(team=team_view(target), applies=applies, propagation=dict(room=room))
    result.update(extra or {})
    return response(rid, result)

def stale_team(rid, target, params):
    if 'if_revision' in params:
        if not isinstance(params['if_revision'], str):
            return invalid(rid, 'out_of_range', 'if_revision must be a string')
        current = layer_revision(target_layer(target))
        if params['if_revision'] != current:
            return failure(rid, 'invalid_state', 'stale', 'the layer changed; its revision is ' + current)
    return None

def tidy(layer):
    avatars = {}
    for aid, entry in layer['avatars'].items():
        entry = {k: v for k, v in entry.items() if k != 'lanes' and v is not None}
        lanes = {l: {k: v for k, v in u.items() if v is not None} for l, u in layer['avatars'][aid].get('lanes', {}).items()}
        lanes = {l: u for l, u in lanes.items() if u}
        if lanes:
            entry['lanes'] = lanes
        if entry.get('role') or lanes:
            avatars[aid] = entry
    return dict(avatars=avatars)

def models_set(rid, params):
    target, error = team_target(rid, 'room.models.set', params, True)
    if error:
        return error
    avatar, lane = params.get('avatar'), params.get('lane')
    if not isinstance(avatar, str) or lane not in ('chat', 'work'):
        return invalid(rid, 'out_of_range', 'avatar must be a string and lane chat or work')
    model = params.get('model')
    if 'model' in params and model is not None and (not isinstance(model, str) or not model):
        return invalid(rid, 'out_of_range', 'model must be a choice id or null')
    if 'model' not in params and 'effort' not in params:
        return invalid(rid, 'missing_params', 'give model or effort')
    effort = params.get('effort')
    if effort is not None and 'effort' in params and (not isinstance(effort, str) or not 1 <= len(effort) <= 64
                                                      or re.search(r'\s', effort) or CONTROL.search(effort)):
        return invalid(rid, 'out_of_range', 'effort must be 1 to 64 characters without whitespace')
    error = stale_team(rid, target, params)
    if error:
        return error
    found = effective_avatars(layers_for(target))
    if avatar not in found:
        return failure(rid, 'not_found', 'unknown_avatar', 'no avatar ' + avatar)
    if lane not in avatar_lanes(found[avatar]['role']):
        return invalid(rid, 'no_such_lane', '%s has no %s lane' % (avatar, lane))
    model = params.get('model')
    choice = None
    if model is not None:
        choice = next((c for c in CHOICES if c['id'] == model), None)
        if choice is None:
            return failure(rid, 'not_found', 'unknown_model', 'no model choice ' + str(model))
    if effort == 'room-rejects' and target == 'room' and binding == 'room_live' and 'sdk_reload' in bound_room['features']:
        warning = dict(layer='session', path='avatars.toml', message='the room rejects effort room-rejects')
        return failure(rid, 'invalid_state', 'rejected_by_room', 'the room refused the change',
                       detail=dict(warnings=[warning]))
    candidate = copy.deepcopy(target_layer(target))
    entry = candidate['avatars'].setdefault(avatar, {})
    unit = entry.setdefault('lanes', {}).setdefault(lane, {})
    if 'model' in params:
        unit.clear()
        if choice is not None:
            unit['model'] = choice['id'] if choice['provider_uuid'] else choice['model_id']
            if choice['provider_uuid']:
                unit['provider'] = choice['provider_uuid']
    if 'effort' in params:
        unit.pop('effort', None)
        if effort is not None:
            unit['effort'] = effort
    return commit_team(rid, target, tidy(candidate))

def avatars_upsert(rid, params):
    target, error = team_target(rid, 'room.avatars.upsert', params, True)
    if error:
        return error
    aid = params.get('id')
    if not isinstance(aid, str) or not re.match(r'^[a-z0-9][a-z0-9_-]{0,47}$', aid):
        return invalid(rid, 'invalid_name', 'id must match [a-z0-9][a-z0-9_-]{0,47}')
    display, alias, role = params.get('display'), params.get('alias'), params.get('role')
    if display is not None and (not isinstance(display, str) or not 1 <= len(display) <= 64):
        return invalid(rid, 'out_of_range', 'display must be 1 to 64 characters')
    if alias is not None and (not isinstance(alias, str) or not 1 <= len(alias) <= 3 or not alias.isascii()
                              or not alias.isalnum()):
        return invalid(rid, 'out_of_range', 'alias must be 1 to 3 ASCII letters or digits')
    error = stale_team(rid, target, params)
    if error:
        return error
    found = effective_avatars(layers_for(target))
    if aid not in found:
        if role is None:
            return invalid(rid, 'missing_params', 'a new avatar needs a role')
        if role in ('chief', 'clerk') or aid in ('chief', 'clerk'):
            return invalid(rid, 'reserved_id', 'chief and clerk are reserved')
        if role not in BASE_ROLES:
            return failure(rid, 'not_found', 'unknown_role', 'no role ' + str(role))
        if len(found) >= 64:
            return invalid(rid, 'out_of_range', 'the team is capped at 64 avatars')
    else:
        if role is not None and role != found[aid]['role']:
            return invalid(rid, 'role_immutable', 'the role of an avatar cannot change')
        if all(k not in params for k in ('role', 'display', 'alias')):
            return invalid(rid, 'missing_params', 'give role, display or alias')
    candidate = copy.deepcopy(target_layer(target))
    entry = candidate['avatars'].setdefault(aid, {})
    if aid not in found:
        entry['role'] = role
    for key, value in (('display', display), ('alias', alias.upper() if isinstance(alias, str) else None)):
        if key in params:
            entry[key] = value
    if not entry.get('role') and aid in found:
        entry['role'] = found[aid]['role']
    error = reject_layer(rid, target, aid, tidy(candidate))
    return error or commit_team(rid, target, tidy(candidate))

def avatars_remove(rid, params):
    target, error = team_target(rid, 'room.avatars.remove', params, True)
    if error:
        return error
    aid = params.get('id')
    if not isinstance(aid, str) or not aid:
        return invalid(rid, 'out_of_range', 'id must be a string')
    error = stale_team(rid, target, params)
    if error:
        return error
    layer = target_layer(target)
    if aid not in layer['avatars']:
        return failure(rid, 'not_found', 'unknown_avatar', 'the layer has no avatar ' + aid)
    if binding == 'room_live' and target == 'room' and any(
            l['avatar'] == aid and l['phase'] == 'running' for l in lane_rows(bound_room)):
        return failure(rid, 'invalid_state', 'lane_running', 'the avatar has a running lane')
    candidate = copy.deepcopy(layer)
    del candidate['avatars'][aid]
    lower = [n for n, l in layers_for(target) if l is not layer and aid in l['avatars']]
    remains = aid in effective_avatars([(n, candidate if l is layer else l) for n, l in layers_for(target)])
    return commit_team(rid, target, candidate, dict(reverted_to=lower[-1] if lower and remains else None))

def team_get(rid, params):
    target, error = team_target(rid, 'room.team.get', params, False)
    return error or response(rid, team_view(target))

ROOM_HANDLERS = {'room.list': room_list, 'room.open': room_open, 'room.attach': room_attach,
                 'room.close': room_close, 'room.send': room_send, 'room.interrupt': room_interrupt,
                 'room.control': room_control, 'room.status': room_status, 'room.history': room_history,
                 'room.shelf.list': room_shelf_list, 'room.item': room_item, 'room.trajectory': room_trajectory,
                 'room.reports': room_reports, 'room.spend': room_spend}
TEAM_HANDLERS = {'room.team.get': team_get, 'room.models.set': models_set, 'room.avatars.upsert': avatars_upsert,
                 'room.avatars.remove': avatars_remove}

def bind_session(rid, method, params):
    """Bind a coding session for create or resume; the registry gains a row for it."""
    global session, binding
    name, warnings = None, []
    if method == 'create' and 'name' in params and not LEGACY:
        name = checked_name(rid, params['name'])
        if isinstance(name, dict):
            return name
    sid = params.get('session_id', 'session-1')
    row = find_row(sid)
    cwd = params.get('cwd', row['workdir'] if row and row['workdir'] else '/fake/workspace')
    if name == 'fail-rename':
        name = sid
        warnings.append(dict(code='rename_failed', message='the session could not be renamed'))
    elif name is None:
        name = row['name'] if method == 'resume' and row and row['name'] else 'fake session'
    session = dict(state=dict(session_id=sid, cwd=cwd, model_id=params.get('model_id', 'fake/model'),
        policy=params.get('policy', dict(plan='forbid', approval='reject')),
        revision=1, external_approvals=params.get('external_approvals', False), pending_approval=None, gate=None,
        turn=None, working=False, awaiting_approval=False, approval_reason=None,
        pending_count=0, messages=[], streaming=None),
        name=name, effort='default', main_pin=None,
        limits=dict(max_output_tokens=0, context_window_limit=0, context_model_alias='', effective_window=128000))
    if a.scenario == 'config' and find_model('model-main') is not None:
        session['main_pin'] = 'model-main'
    binding = 'coding'
    register_session(sid, name, cwd)
    touch()
    result = state_result()
    if warnings:
        result['warnings'] = warnings
    return response(rid, result)

def new_call(rid, method, params):
    """Answer a sessions, usage, skills, room or team call."""
    if method in TEAM_HANDLERS:
        return TEAM_HANDLERS[method](rid, params)
    denied = binding_denial(rid, method, params)
    if denied is not None:
        return denied
    if method.startswith('sessions.'):
        return sessions_call(rid, method, params)
    if method == 'usage.query':
        return usage_query(rid, params)
    if method in SKILL_HANDLERS:
        return SKILL_HANDLERS[method](rid, params)
    return ROOM_HANDLERS[method](rid, params)

def outcome(disposition, turn_id=None, code=None, replacement=False, interrupted=None):
    value = dict(disposition=disposition, turn_id=turn_id,
                 reason=None if code is None else dict(code=code, message='fake ' + code))
    if replacement:
        value['interrupted_turn_id'] = interrupted
    return value

if a.binding == 'room_closed':
    # The adapter starts holding a room whose connection already ended: the ring and epoch are kept.
    attach_room(find_row(room_session_id('sess-coding-a')), False, 0, 0)
    binding = 'room_closed'

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
    elif method in NEW_CALLS:
        frame = new_call(rid, method, params)
    elif binding_denial(rid, method, params) is not None:
        frame = binding_denial(rid, method, params)
        if method == 'detach':
            emit(frame)
            break
    elif method in ('create', 'resume'):
        frame = bind_session(rid, method, params)
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
    for queued in after:
        emit(queued)
    after.clear()
