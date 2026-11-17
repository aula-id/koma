"""Stdlib subprocess smoke tests; optional JSON Schema fixture validation."""
import base64
import io
import json
from pathlib import Path
import subprocess
import sys
import unittest
import uuid
import zipfile

HERE = Path(__file__).resolve().parent
PROTOCOL = HERE.parent / 'protocol'
V1 = ['create', 'resume', 'state', 'submit', 'enqueue', 'interrupt', 'interrupt_and_submit', 'approve',
      'configure', 'detach']
CONFIG = ['config.get', 'config.providers.delete', 'config.models.delete', 'config.roles.unassign',
          'config.subagent_risky.set', 'config.effort.get', 'config.effort.set', 'config.limits.get', 'config.limits.set']
SESSIONS = ['sessions.info', 'sessions.list', 'sessions.rename', 'sessions.delete', 'sessions.stop']
SKILLS = ['skills.list', 'skills.get', 'skills.read_file', 'skills.install', 'skills.remove', 'skills.create',
          'skills.update']
ROOM = ['room.list', 'room.open', 'room.attach', 'room.close', 'room.send', 'room.interrupt', 'room.control',
        'room.status', 'room.history', 'room.shelf.list', 'room.item', 'room.trajectory', 'room.reports',
        'room.spend', 'room.team.get', 'room.models.set', 'room.avatars.upsert', 'room.avatars.remove']
FEATURES = ['bindings.room', 'errors.reason', 'state.session_info', 'create.name', 'config.lock', 'events.room',
            'room.template']
EVENTS = ['state', 'output_append', 'turn', 'room.line', 'room.delta', 'room.status', 'room.notice', 'room.closed',
          'room.gap']
NOW = 1767225600
DAY = 86400

def room_id(parent):
    return uuid.uuid5(uuid.NAMESPACE_OID, 'secure-room:' + parent).hex

def epoch(n):
    return str(uuid.uuid5(uuid.NAMESPACE_OID, 'epoch-%d' % n))

ROOM_A, ROOM_B = room_id('sess-coding-a'), room_id('sess-coding-b')
SECRETS = ('sk-fake-planted-0000000000', 'fake-oauth-token-0000', 'fake-pass', 'fake-query', 'sk-unbound-0000')

def validator():
    try:
        import jsonschema
    except ImportError:
        return None
    return jsonschema.Draft202012Validator(json.loads((PROTOCOL / 'schema.json').read_text()))

def call(rid, method, **params):
    return dict(id=str(rid), method=method, params=params)

class Conformance(unittest.TestCase):
    def run_peer(self, scenario, requests, *extra):
        result = subprocess.run([sys.executable, str(HERE / 'fake_adapter.py'),
            '--scenario', scenario, *extra], input=''.join(json.dumps(r)+'\n' for r in requests),
            text=True, capture_output=True, timeout=5)
        return result, [json.loads(line) for line in result.stdout.splitlines()]

    def replies(self, scenario, requests, *extra):
        result, frames = self.run_peer(scenario, requests, *extra)
        for secret in SECRETS:
            self.assertNotIn(secret, result.stdout)
        return result, {f['id']: f for f in frames if f.get('type') == 'response'}

    def assertBinding(self, frame, reason, binding, allowed, message=None):
        error = frame['error']
        self.assertEqual((error['code'], error['delivery'], error['reason'], error['binding'], error['allowed']),
                         ('invalid_state', 'not_sent', reason, binding, allowed))
        if message is not None:
            self.assertEqual(error['message'], message)

    def assertReason(self, frame, code, reason):
        self.assertEqual((frame['error']['code'], frame['error']['reason']), (code, reason))

    def test_partial(self):
        _, frames = self.run_peer('partial', [
            dict(id='1', method='create', params=dict(cwd='/tmp')),
            dict(id='2', method='submit', params=dict(text='hello')),
            dict(id='3', method='interrupt_and_submit', params=dict(turn_id='turn-1',text='replace')),
            dict(id='4', method='detach', params={})])
        self.assertEqual(frames[0]['type'], 'hello')
        reply = next(f for f in frames if f.get('id') == '3')['result']
        self.assertEqual(reply['disposition'], 'rejected')
        self.assertEqual(reply['interrupted_turn_id'], 'turn-1')
        self.assertEqual(frames[-1]['result'], dict(detached=True))

    def test_reordered(self):
        _, frames = self.run_peer('out_of_order', [
            dict(id='1', method='create', params=dict(cwd='/tmp')),
            dict(id='2', method='state', params={})])
        self.assertEqual([f['id'] for f in frames[1:]], ['2','1'])

    def test_unknown(self):
        _, frames = self.run_peer('unknown', [
            dict(id='1', method='create', params=dict(cwd='/tmp')),
            dict(id='2', method='submit', params=dict(text='hello'))])
        self.assertEqual(frames[-1]['error']['delivery'], 'unknown')

    def test_hello_advertises_methods_features_events_limits(self):
        _, frames = self.run_peer('normal', [])
        hello = frames[0]
        self.assertEqual(hello['capabilities'], V1)
        self.assertEqual(hello['methods'], V1 + CONFIG + SESSIONS + ['usage.query'] + SKILLS + ROOM)
        self.assertEqual(hello['features'], FEATURES)
        self.assertEqual(hello['events'], EVENTS)
        self.assertEqual(hello['limits']['request_ids'], 65536)
        self.assertEqual(hello['limits']['config'], dict(lock_wait_ms=3000))
        self.assertEqual(hello['limits']['list']['history_lines'], 200)
        self.assertEqual(hello['limits']['room']['ring_lines'], 8000)
        self.assertEqual(hello['limits']['skills']['inline_zip_bytes'], 29360128)
        self.assertEqual(hello['limits']['time']['session_stop_ms'], 8000)
        schema = validator()
        if schema is not None:
            schema.validate(hello)

    def test_missing_cap_drops_detach_by_name(self):
        _, frames = self.run_peer('missing_cap', [])
        hello = frames[0]
        self.assertEqual(hello['capabilities'], [m for m in V1 if m != 'detach'])
        self.assertNotIn('detach', hello['methods'])
        self.assertIn('config.get', hello['methods'])

    def test_legacy_hello_is_an_older_adapter(self):
        _, replies = self.replies('legacy_hello', [
            call(1, 'state'), call(2, 'config.get'), call(3, 'create', cwd='/tmp'), call(4, 'create', cwd='/tmp'),
            call(5, 'sessions.info'), call(6, 'state')])
        _, frames = self.run_peer('legacy_hello', [])
        self.assertEqual(set(frames[0]), {'type', 'protocol', 'binary', 'diagnostics', 'capabilities'})
        self.assertEqual(frames[0]['capabilities'], V1)
        self.assertEqual(replies['1']['error'], dict(code='invalid_state', message='fake invalid_state',
                                                     delivery='not_sent'))
        self.assertEqual(replies['2']['error']['code'], 'unsupported_method')
        self.assertEqual(set(replies['3']['result']), {'state'})
        self.assertNotIn('reason', replies['4']['error'])
        self.assertEqual(replies['5']['error']['code'], 'unsupported_method')
        self.assertEqual(set(replies['6']['result']), {'state'})

    def test_wrong_binding_unbound(self):
        result, replies = self.replies('wrong_binding', [
            call(1, 'state'), call(2, 'sessions.info'), call(3, 'config.effort.get'),
            call(4, 'config.get', view='session'), call(5, 'config.roles.unassign', role='main', scope='session'),
            call(6, 'room.status'), call(7, 'config.roles.unassign', role='planner', scope='session'),
            call(8, 'config.get'), call(9, 'room.open'), call(10, 'detach'), call(11, 'state')])
        self.assertBinding(replies['1'], 'wrong_binding', 'unbound', ['coding'], 'no session bound')
        self.assertBinding(replies['2'], 'wrong_binding', 'unbound', ['coding'],
                           'sessions.info requires binding coding; this adapter is unbound')
        self.assertBinding(replies['3'], 'wrong_binding', 'unbound', ['coding'],
                           'config.effort.get requires binding coding; this adapter is unbound')
        self.assertBinding(replies['4'], 'wrong_binding', 'unbound', ['coding'])
        self.assertBinding(replies['5'], 'wrong_binding', 'unbound', ['coding'])
        self.assertBinding(replies['6'], 'wrong_binding', 'unbound', ['room_live', 'room_closed'],
                           'room.status requires binding room_live or room_closed; this adapter is unbound')
        # The role is checked before the binding.
        self.assertReason(replies['7'], 'invalid_request', 'out_of_range')
        self.assertEqual(replies['8']['result']['view'], 'global')
        self.assertReason(replies['9'], 'invalid_request', 'out_of_range')
        # Unbound detach keeps its v1 error, with no added keys, and ends the adapter.
        self.assertEqual(replies['10']['error'], dict(code='invalid_state', message='no session bound',
                                                      delivery='not_sent'))
        self.assertNotIn('11', replies)
        self.assertEqual(result.returncode, 0)

    def test_wrong_binding_coding(self):
        _, replies = self.replies('wrong_binding', [
            call(1, 'create', cwd='/tmp'), call(2, 'resume', session_id='other'), call(3, 'room.send', text='hi'),
            call(4, 'config.roles.unassign', role='planner', scope='session', if_revision='x'),
            call(5, 'sessions.info'), call(6, 'state'), call(7, 'room.history')])
        self.assertIn('session_info', replies['1']['result'])
        self.assertBinding(replies['2'], 'already_bound', 'coding', ['unbound'], 'session already bound')
        self.assertBinding(replies['3'], 'wrong_binding', 'coding', ['room_live'],
                           'room.send requires binding room_live; this adapter is coding')
        self.assertReason(replies['4'], 'invalid_request', 'out_of_range')
        self.assertEqual(set(replies['5']['result']), {'session_info'})
        self.assertEqual(replies['6']['result']['state']['revision'], 1)
        self.assertBinding(replies['7'], 'wrong_binding', 'coding', ['room_live', 'room_closed'])

    def test_wrong_binding_room_closed(self):
        result, replies = self.replies('wrong_binding', [
            call(1, 'room.send', text='hi'), call(2, 'room.control', action='pause'), call(3, 'state'),
            call(4, 'create', cwd='/tmp'), call(5, 'config.limits.get'), call(6, 'config.get'),
            call(7, 'room.status'), call(8, 'detach'), call(9, 'state')], '--binding', 'room_closed')
        self.assertBinding(replies['1'], 'room_closed', 'room_closed', ['room_live'],
                           'the room connection is closed')
        self.assertBinding(replies['2'], 'room_closed', 'room_closed', ['room_live'])
        self.assertBinding(replies['3'], 'wrong_binding', 'room_closed', ['coding'],
                           'state requires binding coding; this adapter is room_closed')
        self.assertBinding(replies['4'], 'already_bound', 'room_closed', ['unbound'], 'session already bound')
        self.assertBinding(replies['5'], 'wrong_binding', 'room_closed', ['coding'])
        self.assertEqual(replies['6']['result']['view'], 'global')
        self.assertEqual((replies['7']['result']['live'], replies['7']['result']['counts_basis']),
                         (False, 'disk_scan'))
        self.assertEqual(replies['8']['result'], dict(detached=True))
        self.assertNotIn('9', replies)
        self.assertEqual(result.returncode, 0)

    def test_wrong_binding_new_methods(self):
        _, replies = self.replies('wrong_binding', [
            call(1, 'room.team.get', target='room'), call(2, 'room.models.set', target='room', avatar='scout',
                                                           lane='chat', model='model-fast'),
            call(3, 'room.models.set', avatar='scout', lane='chat', model='model-fast'),
            call(4, 'room.close'), call(5, 'room.attach', session_id='x'), call(6, 'sessions.list'),
            call(7, 'usage.query'), call(8, 'skills.list'), call(9, 'room.list'), call(10, 'room.team.get'),
            call(11, 'create', cwd='/tmp'), call(12, 'room.attach', session_id='x')])
        self.assertBinding(replies['1'], 'wrong_binding', 'unbound', ['room_live', 'room_closed'])
        self.assertBinding(replies['2'], 'wrong_binding', 'unbound', ['room_live', 'room_closed'])
        self.assertReason(replies['3'], 'invalid_request', 'target_required')
        self.assertBinding(replies['4'], 'wrong_binding', 'unbound', ['room_live', 'room_closed'])
        self.assertReason(replies['5'], 'invalid_state', 'not_running')
        for rid in ('6', '7', '8', '9'):
            self.assertIn('result', replies[rid])
        self.assertEqual(replies['10']['result']['target'], 'template')
        self.assertBinding(replies['12'], 'already_bound', 'coding', ['unbound', 'room_closed'], 'session already bound')

    def test_binding_errors_validate_against_schema(self):
        schema = validator()
        if schema is None:
            self.skipTest('jsonschema is not installed')
        _, frames = self.run_peer('wrong_binding', [
            call(1, 'state'), call(2, 'create', cwd='/tmp'), call(3, 'create', cwd='/tmp'), call(4, 'state'),
            call(5, 'room.send', text='hi')])
        for frame in frames:
            schema.validate(frame)

    def test_subagent_risky_set(self):
        _, replies = self.replies('config', [
            call(1, 'config.get'),
            call(2, 'config.subagent_risky.set'),
            call(3, 'config.subagent_risky.set', mode=1),
            call(4, 'config.subagent_risky.set', mode='allow'),
            call(5, 'config.subagent_risky.set', mode='Parent'),
            call(6, 'config.subagent_risky.set', mode='deny', if_revision='0000000000000000'),
            call(7, 'config.subagent_risky.set', mode='parent'),
            call(8, 'config.subagent_risky.set', mode='parent'),
            call(9, 'config.get'),
            call(10, 'config.subagent_risky.set', mode='classifier'),
            call(11, 'config.subagent_risky.set', mode='classifier')])
        self.assertReason(replies['2'], 'invalid_request', 'missing_params')
        for number in '345':
            self.assertReason(replies[number], 'invalid_request', 'out_of_range')
        self.assertReason(replies['6'], 'invalid_state', 'stale')
        first = replies['7']['result']
        self.assertEqual((first['changed'], first['mode'], first['config']['view']), (True, 'parent', 'global'))
        self.assertEqual(first['propagation']['bound'], 'not_bound')
        self.assertEqual((replies['8']['result']['changed'], replies['8']['result']['propagation']), (False, {}))
        self.assertNotEqual(replies['1']['result']['revision'], replies['9']['result']['revision'])
        self.assertEqual((replies['10']['result']['changed'], replies['10']['result']['mode']), (True, 'classifier'))
        self.assertEqual(replies['11']['result']['changed'], False)
        schema = validator()
        if schema is not None:
            for number in '78':
                schema.validate(replies[number])

    def test_config_unbound(self):
        result, replies = self.replies('config', [
            call(1, 'config.get'),
            call(2, 'config.providers.delete', provider_uuid='nope'),
            call(3, 'config.providers.delete', provider_uuid='prov-ext'),
            call(4, 'config.providers.delete', provider_uuid='prov-local', if_revision='0000000000000000'),
            call(5, 'config.providers.delete', provider_uuid='prov-local', dry_run=True),
            call(6, 'config.models.delete', model_uuid='model-ext', scope='session'),
            call(7, 'config.models.delete', model_uuid='model-unverified'),
            call(8, 'config.roles.unassign', role='planner'),
            call(9, 'config.roles.unassign', role='planner'),
            call(10, 'config.roles.unassign', role='cyber-red'),
            call(11, 'config.models.delete', model_uuid='model-orphan', dry_run=True),
            call(12, 'config.providers.delete', provider_uuid='prov-local'),
            call(13, 'config.get', view='global')])
        view = replies['1']['result']
        self.assertEqual((view['view'], view['file'], view['main_fallback']), ('global', 'ok', None))
        self.assertEqual(len(view['revision']), 16)
        local = next(p for p in view['providers'] if p['provider_uuid'] == 'prov-local')
        self.assertEqual((local['endpoint'], local['endpoint_redacted'], local['key_state'], local['model_count']),
                         ('https://llm.example.test/v1', True, 'set', 2))
        ext = next(p for p in view['providers'] if p['provider_uuid'] == 'prov-ext')
        self.assertEqual((ext['managed'], ext['has_key'], ext['key_state']), (True, False, 'missing'))
        self.assertEqual(view['oauth'][0]['has_token'], True)
        models = {m['model_uuid']: m for m in view['models']}
        self.assertEqual((models['model-planner']['roles'], models['model-planner']['other_roles']),
                         (['planner'], True))
        self.assertEqual(models['model-oauth']['provider_kind'], 'oauth')
        self.assertEqual(models['model-orphan']['provider_kind'], 'missing')
        self.assertEqual([r['role'] for r in view['roles']], ['main', 'awareness', 'safeguard', 'compactor', 'planner'])
        main, safeguard, compactor = view['roles'][0], view['roles'][2], view['roles'][3]
        # Safeguard never inherits Main; the other unassigned roles use Main's route.
        self.assertEqual((safeguard['origin'], safeguard['model_uuid'], safeguard['reason']),
                         ('default', None, 'unassigned'))
        self.assertEqual((compactor['origin'], compactor['model_uuid'], compactor['configured_model'],
                          compactor['provider_uuid'], compactor['reason']),
                         ('inherit_main', None, main['configured_model'], main['provider_uuid'], None))
        self.assertReason(replies['2'], 'not_found', 'not_found')
        self.assertReason(replies['3'], 'invalid_state', 'managed')
        self.assertReason(replies['4'], 'invalid_state', 'stale')
        dry = replies['5']['result']
        self.assertEqual(dry['dry_run'], True)
        self.assertEqual(dry['removed'], dict(provider_uuid='prov-local', models_removed=['model-main', 'model-planner'],
                                              main_reset=True))
        self.assertEqual(dry['config']['revision'], view['revision'])
        self.assertReason(replies['6'], 'invalid_request', 'out_of_range')
        unverified = replies['7']['error']
        self.assertEqual((unverified['code'], unverified['reason'], unverified['delivery'], unverified['written']),
                         ('unavailable', 'unverified', 'unknown', True))
        changed = replies['8']['result']
        self.assertEqual((changed['changed'], changed['previous_model_uuid']), (True, 'model-planner'))
        self.assertEqual(changed['propagation']['bound'], 'not_bound')
        self.assertNotIn('session_info', changed)
        self.assertEqual((replies['9']['result']['changed'], replies['9']['result']['propagation']), (False, {}))
        self.assertReason(replies['10'], 'invalid_request', 'out_of_range')
        self.assertEqual(replies['11']['result']['removed']['roles_released'], [])
        deleted = replies['12']['result']
        self.assertEqual((deleted['dry_run'], deleted['config']['main_fallback']), (False, 'unconfigured'))
        self.assertEqual(deleted['propagation'], dict(bound='not_bound', broadcast='sent', rebind='done',
                                                      rooms='next_start'))
        self.assertNotEqual(replies['13']['result']['revision'], view['revision'])
        self.assertEqual(result.returncode, 0)

    def test_config_coding(self):
        result, replies = self.replies('config', [
            call(1, 'create', cwd='/tmp'),
            call(2, 'config.get'),
            call(3, 'config.roles.unassign', role='main', scope='session', if_revision='x'),
            call(4, 'config.roles.unassign', role='planner', scope='session'),
            call(5, 'config.effort.get', wait_ms=0),
            call(6, 'config.effort.get'),
            call(7, 'config.effort.set', effort='turbo'),
            call(8, 'config.effort.set', effort='high', if_main_model='model-other'),
            call(9, 'config.effort.set', effort='high', if_main_model='model-main'),
            call(10, 'config.limits.set'),
            call(11, 'config.limits.set', max_output_tokens=100),
            call(12, 'config.limits.set', max_output_tokens=4096),
            call(13, 'state'),
            call(14, 'sessions.info'),
            call(15, 'config.roles.unassign', role='main', scope='session'),
            call(16, 'config.roles.unassign', role='main', scope='session'),
            call(17, 'config.limits.get'),
            call(18, 'config.models.delete', model_uuid='model-ext', dry_run=True),
            call(19, 'config.models.delete', model_uuid='model-unverified'),
            call(20, 'state')])
        created = replies['1']['result']
        info = created['session_info']
        self.assertEqual((info['effort'], info['usage'], info['routes'][0]['origin']), ('default', None, 'session'))
        self.assertEqual(info['limits'], dict(max_output_tokens=0, context_window_limit=0, context_model_alias='',
                                              effective_window=128000))
        self.assertEqual(replies['2']['result']['view'], 'session')
        self.assertReason(replies['3'], 'invalid_request', 'conflicting_params')
        self.assertReason(replies['4'], 'invalid_request', 'out_of_range')
        self.assertEqual(replies['5']['result']['state'], 'loading')
        ready = replies['6']['result']
        self.assertEqual((ready['state'], ready['options'][0], ready['main_model_uuid']), ('ready', 'default', 'model-main'))
        self.assertReason(replies['7'], 'invalid_request', 'not_offered')
        self.assertReason(replies['8'], 'invalid_state', 'main_changed')
        self.assertEqual(replies['9']['result']['effort'], 'high')
        self.assertEqual(replies['9']['result']['session_info']['effort'], 'high')
        self.assertReason(replies['10'], 'invalid_request', 'missing_params')
        self.assertReason(replies['11'], 'invalid_request', 'out_of_range')
        self.assertEqual(replies['12']['result']['max_output_tokens'], 4096)
        state = replies['13']['result']
        self.assertEqual(state['state']['revision'], created['state']['revision'])
        self.assertEqual(state['session_info']['limits']['max_output_tokens'], 4096)
        self.assertGreater(state['session_info']['observed_at_ms'], info['observed_at_ms'])
        self.assertEqual(replies['14']['result']['session_info'], state['session_info'])
        cleared = replies['15']['result']
        self.assertEqual((cleared['changed'], cleared['previous_model_uuid'], cleared['config']['view']),
                         (True, 'model-main', 'session'))
        self.assertEqual(cleared['session_info']['effort'], 'default')
        self.assertEqual(cleared['config']['roles'][0]['origin'], 'global')
        self.assertEqual(replies['16']['result']['changed'], False)
        self.assertEqual(replies['17']['result']['max_output_tokens'], 4096)
        self.assertEqual(replies['18']['result']['propagation'], {})
        self.assertIn('session_info', replies['18']['result'])
        self.assertEqual(replies['19']['error']['delivery'], 'unknown')
        self.assertNotIn('20', replies)
        self.assertEqual(result.returncode, 0)

    def test_configure_feeds_config_view(self):
        _, replies = self.replies('normal', [
            call(1, 'config.get'),
            call(2, 'configure', providers=[dict(name='p', endpoint='http://127.0.0.1:9/v1', api_type='openai',
                                                 api_key='sk-unbound-0000')],
                 models=dict(main=dict(provider='p', model_id='m'))),
            call(3, 'config.get'),
            call(4, 'config.models.delete', model_uuid='model-main'),
            call(5, 'config.get')])
        missing = replies['1']['result']
        self.assertEqual((missing['file'], missing['revision'], missing['main_fallback']),
                         ('missing', 'absent', 'unconfigured'))
        view = replies['3']['result']
        self.assertEqual((view['providers'][0]['key_state'], view['providers'][0]['endpoint_redacted']), ('set', False))
        self.assertEqual(view['roles'][0]['model_uuid'], 'model-main')
        self.assertEqual(replies['4']['result']['removed']['roles_released'], ['main'])
        self.assertEqual(replies['5']['result']['main_fallback'], 'unconfigured')

    def frames_by_id(self, scenario, requests, *extra):
        """(all frames, replies by id, events) of one run."""
        result, frames = self.run_peer(scenario, requests, *extra)
        replies = {f['id']: f for f in frames if f.get('type') == 'response'}
        return frames, replies, [f for f in frames if f.get('type') == 'event']

    def test_create_name(self):
        _, replies = self.replies('sessions', [
            call(1, 'create', cwd='/tmp', name='  Renamed  '), call(2, 'sessions.info'),
            call(3, 'sessions.list', kind='coding', limit=1)])
        self.assertEqual(replies['1']['result']['session_info']['name'], 'Renamed')
        self.assertNotIn('warnings', replies['1']['result'])
        self.assertEqual(replies['2']['result']['session_info']['name'], 'Renamed')
        _, replies = self.replies('sessions', [call(1, 'create', name='x' * 201), call(2, 'create', name='a\x01'),
                                               call(3, 'create', name='fail-rename')])
        self.assertReason(replies['1'], 'invalid_request', 'invalid_name')
        self.assertReason(replies['2'], 'invalid_request', 'invalid_name')
        created = replies['3']['result']
        self.assertEqual(created['warnings'][0]['code'], 'rename_failed')
        self.assertEqual(created['session_info']['name'], created['state']['session_id'])
        _, replies = self.replies('normal', [call(1, 'create', cwd='/tmp')])
        self.assertEqual(set(replies['1']['result']), {'state', 'session_info'})

    def test_sessions(self):
        order = [ROOM_A, 'sess-coding-a', 'sess-stuck', ROOM_B, 'sess-coding-b', 'sess-coding-c', 'sess-locked',
                 'sess-partial', 'sess-busy', 'sess-nostart', 'sess-clash', room_id('sess-clash'), 'sess-live-nr']
        _, frames = self.run_peer('sessions', [])
        self.assertEqual(frames[0]['type'], 'hello')
        _, replies = self.replies('sessions', [
            call(1, 'sessions.list', limit=5), call(2, 'sessions.list', limit=500),
            call(3, 'sessions.list', kind='room'), call(4, 'sessions.list', live=True),
            call(5, 'sessions.list', limit=0), call(6, 'sessions.list', cursor='nonsense'),
            call(7, 'sessions.list', kind='other'), call(8, 'sessions.list', live='yes')])
        first = replies['1']['result']
        self.assertEqual([r['session_id'] for r in first['sessions']], order[:5])
        self.assertIsInstance(first['next_cursor'], str)
        _, more = self.replies('sessions', [call(1, 'sessions.list', limit=5, cursor=first['next_cursor']),
                                            call(2, 'sessions.list', limit=50, cursor=first['next_cursor'])])
        self.assertEqual([r['session_id'] for r in more['1']['result']['sessions']], order[5:10])
        self.assertEqual([r['session_id'] for r in more['2']['result']['sessions']], order[5:])
        self.assertIsNone(more['2']['result']['next_cursor'])
        everything = replies['2']['result']['sessions']
        self.assertEqual([r['session_id'] for r in everything], order)
        by_id = {r['session_id']: r for r in everything}
        alpha = by_id['sess-coding-a']
        self.assertEqual((alpha['name'], alpha['kind'], alpha['live'], alpha['working'], alpha['locked'], alpha['bound']),
                         ('Alpha', 'coding', True, True, True, False))
        self.assertEqual((alpha['room_session_id'], alpha['room_live'], alpha['parent_session_id']),
                         (ROOM_A, True, None))
        self.assertEqual((by_id['sess-coding-b']['room_session_id'], by_id['sess-coding-b']['room_live']),
                         (ROOM_B, False))
        room = by_id[ROOM_A]
        self.assertEqual((room['kind'], room['working'], room['parent_session_id'], room['room_session_id']),
                         ('room', None, 'sess-coding-a', None))
        orphan = by_id['sess-live-nr']
        self.assertEqual((orphan['name'], orphan['workdir'], orphan['live'], orphan['created_at']),
                         (None, None, True, None))
        self.assertEqual(alpha['updated_at'], NOW - 60)
        self.assertEqual([r['kind'] for r in replies['3']['result']['sessions']], ['room', 'room'])
        self.assertTrue(all(r['live'] for r in replies['4']['result']['sessions']))
        self.assertReason(replies['5'], 'invalid_request', 'out_of_range')
        self.assertReason(replies['6'], 'invalid_request', 'bad_cursor')
        self.assertReason(replies['7'], 'invalid_request', 'out_of_range')
        self.assertReason(replies['8'], 'invalid_request', 'out_of_range')

    def test_sessions_rename_delete_stop(self):
        result, replies = self.replies('sessions', [
            call(1, 'sessions.rename', session_id='sess-coding-b', name='  Beta two '),
            call(2, 'sessions.rename', session_id='sess-coding-a', name='x'),
            call(3, 'sessions.rename', session_id=ROOM_A, name='Room new'),
            call(4, 'sessions.rename', session_id='missing', name='x'),
            call(5, 'sessions.rename', session_id='sess-coding-b', name=''),
            call(6, 'sessions.rename', session_id='sess-coding-b', name='a\x07'),
            call(7, 'sessions.rename', session_id='sess-busy', name='x'),
            call(8, 'sessions.delete', session_id='sess-coding-a'),
            call(9, 'sessions.delete', session_id='sess-locked'),
            call(10, 'sessions.delete', session_id='sess-partial'),
            call(11, 'sessions.delete', session_id='sess-partial'),
            call(12, 'sessions.delete', session_id='sess-partial'),
            call(13, 'sessions.delete', session_id='sess-coding-b'),
            call(14, 'sessions.stop', session_id='sess-unknown'),
            call(15, 'sessions.stop', session_id='sess-stuck'),
            call(16, 'sessions.stop', session_id='sess-coding-a'),
            call(17, 'sessions.stop', session_id='sess-coding-a'),
            call(18, 'sessions.list', limit=500)])
        self.assertEqual((replies['1']['result']['session']['name'], replies['1']['result']['session']['live']),
                         ('Beta two', False))
        self.assertEqual(replies['2']['error']['message'], 'rename it from an adapter bound to it, or stop it first')
        self.assertReason(replies['2'], 'invalid_state', 'live')
        self.assertEqual(replies['3']['result']['session']['name'], 'Room new')
        self.assertReason(replies['4'], 'not_found', 'not_found')
        self.assertReason(replies['5'], 'invalid_request', 'invalid_name')
        self.assertReason(replies['6'], 'invalid_request', 'invalid_name')
        self.assertReason(replies['7'], 'unavailable', 'sqlite_busy')
        self.assertReason(replies['8'], 'invalid_state', 'live')
        self.assertReason(replies['9'], 'invalid_state', 'session_locked')
        partial = replies['10']['error']
        self.assertEqual((partial['code'], partial['reason'], partial['delivery'], partial['written']),
                         ('unavailable', 'partial_delete', 'unknown', True))
        self.assertEqual(replies['11']['result'], dict(deleted=True, kind='coding', orphaned_room_session_id=None))
        self.assertReason(replies['12'], 'not_found', 'not_found')
        self.assertEqual(replies['13']['result']['orphaned_room_session_id'], ROOM_B)
        self.assertEqual(replies['14']['result'], dict(stopped=True, was_live=False))
        stuck = replies['15']['error']
        self.assertEqual((stuck['code'], stuck['reason'], stuck['delivery']), ('unavailable', 'still_running', 'unknown'))
        self.assertEqual(replies['16']['result'], dict(stopped=True, was_live=True))
        self.assertEqual(replies['17']['result'], dict(stopped=True, was_live=False))
        ids = [r['session_id'] for r in replies['18']['result']['sessions']]
        self.assertNotIn('sess-partial', ids)
        self.assertNotIn('sess-coding-b', ids)
        self.assertFalse(next(r for r in replies['18']['result']['sessions'] if r['session_id'] == 'sess-coding-a')['live'])
        self.assertEqual(result.returncode, 0)

    def test_sessions_bound_self(self):
        _, replies = self.replies('sessions', [
            call(1, 'create', cwd='/tmp', name='Mine'), call(2, 'sessions.delete', session_id='session-1'),
            call(3, 'sessions.stop', session_id='session-1'),
            call(4, 'sessions.rename', session_id='session-1', name='Mine two'), call(5, 'sessions.info'),
            call(6, 'sessions.list', limit=500)])
        self.assertReason(replies['2'], 'invalid_state', 'bound_self')
        self.assertReason(replies['3'], 'invalid_state', 'bound_self')
        self.assertEqual(replies['4']['result']['session']['name'], 'Mine two')
        self.assertEqual(replies['5']['result']['session_info']['name'], 'Mine two')
        mine = next(r for r in replies['6']['result']['sessions'] if r['session_id'] == 'session-1')
        self.assertEqual((mine['bound'], mine['live'], mine['name']), (True, True, 'Mine two'))

    def test_usage(self):
        _, replies = self.replies('usage', [
            call(1, 'usage.query'),
            call(2, 'usage.query', from_at=NOW - 4 * DAY, to_at=NOW, group_by='model'),
            call(3, 'usage.query', from_at=NOW - 4 * DAY, to_at=NOW, group_by='role', limit=2),
            call(4, 'usage.query', from_at=NOW - 4 * DAY, to_at=NOW, group_by='session'),
            call(5, 'usage.query', from_at=NOW - 4 * DAY, to_at=NOW, bucket='day'),
            call(6, 'usage.query', from_at=NOW - 4 * DAY, to_at=NOW, bucket='day', tz_offset_minutes=60,
                 session_id='sess-coding-a'),
            call(7, 'usage.query', from_at=NOW - 3 * DAY, to_at=NOW - 3 * DAY + 3 * 3600, bucket='hour'),
            call(8, 'usage.query', from_at=NOW - 14 * DAY, to_at=NOW, bucket='week'),
            call(9, 'usage.query', workdir='/fake/other'),
            call(10, 'usage.query', from_at=NOW, to_at=NOW), call(11, 'usage.query', from_at=0, to_at=401 * DAY),
            call(12, 'usage.query', from_at=NOW - 399 * DAY, to_at=NOW, bucket='hour'),
            call(13, 'usage.query', group_by='nope'), call(14, 'usage.query', tz_offset_minutes=900),
            call(15, 'usage.query', limit=501), call(16, 'usage.query', workdir='relative'),
            call(17, 'usage.query', session_id='sess-slow'), call(18, 'usage.query', session_id='nobody'),
            call(19, 'usage.query', from_at=NOW - 50 * DAY, to_at=NOW)])
        whole = replies['1']['result']
        self.assertEqual((whole['from_at'], whole['to_at'], whole['ledger']), (NOW - 30 * DAY, NOW, 'ok'))
        self.assertEqual(whole['totals'], dict(calls=11, tokens_in=19200, tokens_cached=2000, tokens_out=4200,
                                               cost=0.196))
        self.assertNotIn('groups', whole)
        self.assertNotIn('buckets', whole)
        models = replies['2']['result']['groups']
        self.assertEqual([g['key'] for g in models], ['fake/main', 'fake/planner', 'fake/fast'])
        self.assertEqual((models[0]['label'], models[0]['calls']), (None, 7))
        self.assertEqual([g['cost'] for g in models], sorted((g['cost'] for g in models), reverse=True))
        roles = replies['3']['result']
        self.assertEqual(len(roles['groups']), 2)
        self.assertTrue(roles['groups_truncated'])
        self.assertEqual((roles['groups'][0]['key'], roles['groups'][0]['role_kind'], roles['groups'][0]['agent']),
                         ('main', 'main', None))
        sessions = {g['key']: g for g in replies['4']['result']['groups']}
        self.assertEqual((sessions['sess-coding-a']['label'], sessions['sess-coding-a']['kind']), ('Alpha', 'coding'))
        self.assertEqual((sessions['sess-deleted']['label'], sessions['sess-deleted']['kind'],
                          sessions['sess-deleted']['deleted']), (None, None, True))
        self.assertEqual(sessions[ROOM_A]['kind'], 'room')
        days = replies['5']['result']['buckets']
        self.assertEqual([b['start_at'] for b in days], [NOW - 4 * DAY + n * DAY for n in range(4)])
        self.assertEqual([b['calls'] for b in days], [0, 2, 2, 7])
        shifted = replies['6']['result']['buckets']
        self.assertEqual(shifted[0]['start_at'], NOW - 4 * DAY - 3600)
        self.assertEqual(sum(b['calls'] for b in shifted), 4)
        hours = replies['7']['result']['buckets']
        self.assertEqual(len(hours), 3)
        self.assertEqual([b['calls'] for b in hours], [0, 1, 1])
        weeks = replies['8']['result']['buckets']
        self.assertEqual([b['start_at'] % (7 * DAY) for b in weeks], [4 * DAY] * len(weeks))
        self.assertEqual(replies['9']['result']['totals']['calls'], 2)
        self.assertReason(replies['10'], 'invalid_request', 'out_of_range')
        self.assertReason(replies['11'], 'invalid_request', 'range_too_large')
        self.assertReason(replies['12'], 'invalid_request', 'range_too_large')
        for rid in ('13', '14', '15', '16'):
            self.assertReason(replies[rid], 'invalid_request', 'out_of_range')
        self.assertReason(replies['17'], 'unavailable', 'query_timeout')
        self.assertEqual(replies['18']['result']['totals'], dict(calls=0, tokens_in=0, tokens_cached=0, tokens_out=0,
                                                                 cost=0))
        self.assertEqual(replies['19']['result']['totals']['calls'], whole['totals']['calls'] + 1)
        _, missing = self.replies('normal', [call(1, 'usage.query', group_by='model', bucket='day',
                                                  from_at=NOW - 2 * DAY, to_at=NOW)])
        result = missing['1']['result']
        self.assertEqual((result['ledger'], result['totals']['calls'], result['groups']), ('missing', 0, []))
        self.assertEqual([b['calls'] for b in result['buckets']], [0, 0])

    def test_skills_list_and_get(self):
        _, replies = self.replies('skills', [
            call(1, 'skills.list'), call(2, 'skills.list', workdir='/fake/workspace', include_diagnostics=True),
            call(3, 'skills.list', workdir='/fake/workspace', scope='project'),
            call(4, 'skills.list', workdir='/fake/workspace', tier='extra-root:0'),
            call(5, 'skills.list', scope='project'), call(6, 'skills.list', tier='claude'),
            call(7, 'skills.list', workdir='/fake/workspace', limit=3),
            call(8, 'skills.list', workdir='relative'), call(9, 'skills.list', workdir='/fake/nowhere'),
            call(10, 'skills.list', tier='bogus'), call(11, 'skills.list', limit=0),
            call(12, 'skills.list', scope='computer', workdir='/fake/workspace')])
        listed = replies['1']['result']
        self.assertEqual((listed['project_workdir'], [s['name'] for s in listed['skills']], listed['diagnostics']),
                         (None, ['code-review', 'deploy-notes', 'odd-yaml'], []))
        row = listed['skills'][0]
        self.assertEqual((row['scope'], row['tier'], row['editable'], row['effective'], row['shadowed_by']),
                         ('computer', 'global', True, True, None))
        full = replies['2']['result']
        self.assertEqual(full['project_workdir'], '/fake/workspace')
        self.assertEqual([(s['tier'], s['name']) for s in full['skills']], [
            ('global', 'code-review'), ('global', 'deploy-notes'), ('global', 'odd-yaml'),
            ('claude', 'claude-helper'), ('project-agent', 'legacy-skill'), ('project-agents', 'code-review'),
            ('project-agents', 'proj-helper'), ('extra-root:0', 'extra-skill')])
        shadowed = full['skills'][0]
        self.assertEqual((shadowed['effective'], shadowed['shadowed_by']), (False, 'project-agents'))
        by_tier = {s['tier']: s for s in full['skills']}
        self.assertEqual((by_tier['claude']['scope'], by_tier['claude']['editable']), ('external', False))
        self.assertEqual((by_tier['project-agent']['scope'], by_tier['project-agent']['editable']), ('project', True))
        self.assertEqual(full['diagnostics'], [dict(path='/fake/home/.koma/skills/broken/SKILL.md',
                                                    error='missing description')])
        self.assertEqual([s['tier'] for s in replies['3']['result']['skills']],
                         ['project-agent', 'project-agents', 'project-agents'])
        self.assertEqual([s['name'] for s in replies['4']['result']['skills']], ['extra-skill'])
        self.assertReason(replies['5'], 'invalid_request', 'workdir_required')
        self.assertReason(replies['6'], 'invalid_request', 'workdir_required')
        page = replies['7']['result']
        self.assertEqual(len(page['skills']), 3)
        _, more = self.replies('skills', [call(1, 'skills.list', workdir='/fake/workspace', limit=50,
                                               cursor=page['next_cursor'])])
        self.assertEqual(len(more['1']['result']['skills']), 5)
        self.assertReason(replies['8'], 'invalid_request', 'out_of_range')
        self.assertReason(replies['9'], 'invalid_request', 'out_of_range')
        self.assertReason(replies['10'], 'invalid_request', 'out_of_range')
        self.assertReason(replies['11'], 'invalid_request', 'out_of_range')
        self.assertEqual([s['name'] for s in replies['12']['result']['skills']],
                         ['code-review', 'deploy-notes', 'odd-yaml'])
        _, replies = self.replies('skills', [
            call(1, 'skills.get', name='code-review', include_raw=True),
            call(2, 'skills.get', name='code-review', workdir='/fake/workspace'),
            call(3, 'skills.get', name='code-review', tier='global', workdir='/fake/workspace'),
            call(4, 'skills.get', name='nope'), call(5, 'skills.get', name='odd-yaml'),
            call(6, 'skills.get', name='claude-helper', workdir='/fake/workspace'), call(7, 'skills.get'),
            call(8, 'skills.read_file', name='code-review', path='notes/checklist.md'),
            call(9, 'skills.read_file', name='code-review', path='bin/tool.bin'),
            call(10, 'skills.read_file', name='code-review', path='missing.md'),
            call(11, 'skills.read_file', name='code-review', path='../x'),
            call(12, 'skills.read_file', name='code-review', path='notes/checklist.md', generation='0' * 16)])
        detail = replies['1']['result']
        self.assertEqual((detail['instruction'], detail['declared_tools'], detail['companion_files']),
                         ('Read the diff and list defects.', ['read_file'], ['bin/tool.bin', 'notes/checklist.md']))
        self.assertTrue(detail['raw'].startswith('---\nname: code-review\n'))
        self.assertTrue(detail['structured_save_supported'])
        self.assertIsNone(replies['2']['result']['raw'])
        self.assertEqual((replies['2']['result']['tier'], replies['3']['result']['tier']), ('project-agents', 'global'))
        self.assertReason(replies['4'], 'not_found', 'not_found')
        self.assertFalse(replies['5']['result']['structured_save_supported'])
        self.assertEqual(replies['6']['result']['tier'], 'claude')
        self.assertReason(replies['7'], 'invalid_request', 'invalid_name')
        self.assertEqual(replies['8']['result'], dict(path='notes/checklist.md', text='check names\ncheck tests\n',
                                                      bytes=24))
        self.assertReason(replies['9'], 'invalid_request', 'binary')
        self.assertReason(replies['10'], 'not_found', 'not_found')
        self.assertReason(replies['11'], 'invalid_request', 'out_of_range')
        self.assertReason(replies['12'], 'invalid_state', 'stale')

    def test_validation_matches_the_adapter(self):
        # The reasons and the order of checks the real adapter gives.
        _, replies = self.replies('skills', [
            call(1, 'sessions.rename', session_id='sess-coding-b', name='x' * 201),
            call(2, 'sessions.rename', session_id='sess-coding-b', name=7),
            call(3, 'sessions.rename', session_id='sess-nope', name='a\n'),
            call(4, 'usage.query', from_at=-1),
            call(5, 'usage.query', to_at=999999999999),
            call(6, 'skills.update', name='x', patch={'description': 'd'}),
            call(7, 'skills.remove', name='x'),
            call(8, 'skills.read_file', name='code-review'),
            call(9, 'skills.update', tier='claude', name='claude-helper', patch={'description': 'd'}),
            call(10, 'skills.remove', tier='claude', name='claude-helper'),
            call(11, 'skills.read_file', name='no-such-skill', path='../x'),
            call(12, 'skills.update', tier='global', name='no-such-skill', patch={'name': 'Bad'}),
            call(13, 'skills.install', scope='computer'),
            call(14, 'skills.create', scope='computer', name='fine', instruction='b')])
        expected = {
            '1': 'invalid_name', '2': 'invalid_name', '3': 'invalid_name', '4': 'out_of_range',
            '5': 'out_of_range', '6': 'out_of_range', '7': 'out_of_range', '8': 'out_of_range',
            '11': 'out_of_range', '12': 'invalid_name', '13': 'conflicting_params', '14': 'out_of_range'}
        for rid, reason in expected.items():
            self.assertReason(replies[rid], 'invalid_request', reason)
        self.assertReason(replies['9'], 'invalid_state', 'read_only')
        self.assertReason(replies['10'], 'invalid_state', 'read_only')

    def test_skills_writes(self):
        package = io.BytesIO()
        with zipfile.ZipFile(package, 'w') as archive:
            archive.writestr('zipped/SKILL.md', '---\nname: zipped\ndescription: From a zip\n---\nZip body\n')
            archive.writestr('zipped/notes.md', 'extra')
        data = base64.b64encode(package.getvalue()).decode()
        macos = io.BytesIO()
        with zipfile.ZipFile(macos, 'w') as archive:
            archive.writestr('__MACOSX/x', 'junk')
            archive.writestr('zipped/SKILL.md', '---\ndescription: d\n---\nbody')
        _, replies = self.replies('skills', [
            call(1, 'skills.create', scope='computer', name='fresh', description='A fresh skill', instruction='Body',
                 triggers=['go'], allowed_tools=['read_file']),
            call(2, 'skills.create', scope='computer', name='fresh', description='d', instruction='b'),
            call(3, 'skills.create', scope='computer', name='Bad_Name', description='d', instruction='b'),
            call(4, 'skills.create', scope='computer', name='readme', description='d', instruction='b'),
            call(5, 'skills.create', scope='project', name='proj-new', description='d', instruction='b'),
            call(6, 'skills.create', scope='project', name='proj-new', description='d', instruction='b',
                 workdir='/fake/other'),
            call(7, 'skills.create', scope='computer', name='nodesc', instruction='b'),
            call(8, 'skills.create', scope='nowhere', name='x', description='d', instruction='b'),
            call(9, 'skills.get', name='fresh'),
            call(10, 'skills.update', name='fresh', patch={'description': 'Changed'}),
            call(11, 'skills.update', tier='global', name='fresh', patch={}),
            call(12, 'skills.update', tier='global', name='fresh', generation='0' * 16, patch={'description': 'x'}),
            call(13, 'skills.update', tier='global', name='fresh', patch={'name': 'fresher', 'instruction': 'New'}),
            call(14, 'skills.update', tier='global', name='fresher', patch={'name': 'deploy-notes'}),
            call(15, 'skills.update', tier='global', name='odd-yaml', patch={'description': 'x'}),
            call(16, 'skills.update', tier='claude', name='claude-helper', workdir='/fake/workspace',
                 patch={'description': 'x'}),
            call(17, 'skills.install', scope='computer', data_b64=data),
            call(18, 'skills.install', scope='computer', data_b64=data),
            call(19, 'skills.install', scope='computer'),
            call(20, 'skills.install', scope='computer', data_b64=data, path='/x'),
            call(21, 'skills.install', scope='computer', data_b64='!!not base64!!'),
            call(22, 'skills.install', scope='computer', data_b64=base64.b64encode(b'plain').decode()),
            call(23, 'skills.install', scope='computer', data_b64=base64.b64encode(macos.getvalue()).decode()),
            call(24, 'skills.install', scope='computer', data_b64='A' * 39146841),
            call(25, 'skills.install', scope='computer', data_b64=data, name='renamed-zip'),
            call(26, 'skills.remove', tier='global', name='fresher', generation='0' * 16),
            call(27, 'skills.remove', tier='global', name='fresher'),
            call(28, 'skills.remove', tier='global', name='fresher'),
            call(29, 'skills.remove', tier='project-agents', name='code-review', workdir='/fake/workspace'),
            call(30, 'skills.remove', tier='extra-root:0', name='extra-skill', workdir='/fake/workspace'),
            call(31, 'skills.get', name='zipped'), call(32, 'skills.read_file', name='zipped', path='notes.md')])
        created = replies['1']['result']
        self.assertEqual((created['skill']['tier'], created['skill']['scope'], created['skill']['triggers']),
                         ('global', 'computer', ['go']))
        self.assertEqual((created['skill']['raw'], created['propagation']), (None, dict(skills='next_rebuild')))
        self.assertReason(replies['2'], 'invalid_state', 'exists')
        self.assertReason(replies['3'], 'invalid_request', 'invalid_name')
        self.assertReason(replies['4'], 'invalid_request', 'invalid_name')
        self.assertReason(replies['5'], 'invalid_request', 'workdir_required')
        self.assertEqual(replies['6']['result']['skill']['tier'], 'project-agents')
        self.assertReason(replies['7'], 'invalid_request', 'out_of_range')
        self.assertReason(replies['8'], 'invalid_request', 'out_of_range')
        generation = replies['9']['result']['generation']
        self.assertReason(replies['10'], 'invalid_request', 'out_of_range')
        self.assertReason(replies['11'], 'invalid_request', 'missing_params')
        self.assertReason(replies['12'], 'invalid_state', 'stale')
        renamed = replies['13']['result']
        self.assertEqual((renamed['renamed'], renamed['skill']['name'], renamed['skill']['instruction']),
                         (True, 'fresher', 'New'))
        self.assertNotEqual(renamed['skill']['generation'], generation)
        self.assertReason(replies['14'], 'invalid_state', 'exists')
        self.assertReason(replies['15'], 'invalid_state', 'unsafe_yaml')
        self.assertReason(replies['16'], 'invalid_state', 'read_only')
        installed = replies['17']['result']['skill']
        self.assertEqual((installed['name'], installed['tier'], installed['companion_files']),
                         ('zipped', 'global', ['notes.md']))
        self.assertReason(replies['18'], 'invalid_state', 'exists')
        self.assertReason(replies['19'], 'invalid_request', 'conflicting_params')
        self.assertReason(replies['20'], 'invalid_request', 'conflicting_params')
        self.assertReason(replies['21'], 'invalid_request', 'invalid_package')
        self.assertReason(replies['22'], 'invalid_request', 'invalid_package')
        self.assertIn('__MACOSX', replies['23']['error']['message'])
        self.assertReason(replies['24'], 'invalid_request', 'too_large')
        self.assertEqual(replies['25']['result']['skill']['name'], 'renamed-zip')
        self.assertReason(replies['26'], 'invalid_state', 'stale')
        self.assertEqual(replies['27']['result'], dict(removed=True, now_effective=None,
                                                       propagation=dict(skills='next_rebuild')))
        self.assertReason(replies['28'], 'not_found', 'not_found')
        now = replies['29']['result']['now_effective']
        self.assertEqual((now['tier'], now['effective'], now['shadowed_by']), ('global', True, None))
        self.assertReason(replies['30'], 'invalid_state', 'read_only')
        self.assertEqual(replies['32']['result']['text'], 'extra')

    def test_skills_install_path_and_room_propagation(self):
        import tempfile
        package = io.BytesIO()
        with zipfile.ZipFile(package, 'w') as archive:
            archive.writestr('from-file/SKILL.md', '---\ndescription: From a file\n---\nbody')
        with tempfile.TemporaryDirectory() as tmp:
            path = str(Path(tmp) / 'skill.zip')
            Path(path).write_bytes(package.getvalue())
            _, replies = self.replies('skills', [
                call(1, 'skills.install', scope='computer', path=path),
                call(2, 'skills.install', scope='computer', path=str(Path(tmp) / 'absent.zip')),
                call(3, 'skills.install', scope='computer', path='relative.zip')])
        self.assertEqual(replies['1']['result']['skill']['name'], 'from-file')
        self.assertReason(replies['2'], 'not_found', 'not_found')
        self.assertReason(replies['3'], 'invalid_request', 'out_of_range')
        _, replies = self.replies('skills', [
            call(1, 'create', cwd='/fake/workspace'), call(2, 'skills.list'),
            call(3, 'skills.create', scope='project', name='coding-new', description='d', instruction='b')])
        self.assertEqual(replies['2']['result']['project_workdir'], '/fake/workspace')
        self.assertEqual(len(replies['2']['result']['skills']), 8)
        self.assertEqual(replies['3']['result']['propagation'], dict(skills='next_rebuild'))
        _, replies = self.replies('room', [
            call(1, 'room.open', parent_session_id='sess-coding-a'),
            call(2, 'skills.create', scope='project', name='room-new', description='d', instruction='b')])
        self.assertEqual(replies['2']['result']['propagation'], dict(skills='room_restart'))
        self.assertEqual(replies['2']['result']['skill']['tier'], 'project-agents')

    def test_room_open_events_and_reads(self):
        frames, replies, events = self.frames_by_id('room', [
            call(1, 'room.open', parent_session_id='sess-coding-a', tail=3),
            call(2, 'room.status'), call(3, 'room.history', epoch=epoch(1)),
            call(4, 'room.history', epoch=epoch(1), limit=4),
            call(5, 'room.history', epoch=epoch(1), before_line_seq=5, limit=2),
            call(6, 'room.history', epoch=epoch(1), after_line_seq=8, limit=1),
            call(7, 'room.history', epoch=epoch(1), after_line_seq=8, before_line_seq=3),
            call(8, 'room.history', epoch='stale'),
            call(9, 'room.history', epoch=epoch(1), kinds=['tool_call', 'tool_result']),
            call(10, 'room.history', epoch=epoch(1), author='scout', limit=1),
            call(11, 'room.history', epoch=epoch(1), limit=201)])
        opened = replies['1']['result']
        self.assertEqual((opened['started'], opened['epoch']), (False, epoch(1)))
        self.assertEqual(opened['room'], dict(session_id=ROOM_A, parent_session_id='sess-coding-a',
            parent_name='Alpha', name='Alpha room', workdir='/fake/workspace', live=True, locked=True, bound=True,
            created_at=NOW - 30 * DAY, updated_at=NOW))
        self.assertEqual(opened['daemon']['features'], ['sdk_user', 'sdk_reload'])
        self.assertEqual(opened['history'], dict(first_line_seq=1, last_line_seq=10, count=10, replayed=10))
        self.assertEqual([l['line_seq'] for l in opened['tail']], [8, 9, 10])
        self.assertEqual(opened['roster'], ['chief', 'clerk', 'scout', 'analyst', 'recon', 'probe'])
        self.assertTrue(opened['status']['fresh'])
        replay = opened['tail'][-1]
        self.assertEqual((replay['kind'], replay['author'], replay['replay'], replay['origin'], replay['display']),
                         ('chief', 'chief', True, 'room', 'Chief'))
        self.assertEqual(set(replay), {'line_seq', 'kind', 'author', 'display', 'alias', 'lane', 'audience', 'text',
                                       'call_id', 'tool', 'args', 'ok', 'result', 'replay', 'origin',
                                       'received_at_ms', 'truncated'})
        self.assertLess(frames.index(replies['1']), frames.index(events[0]))
        self.assertEqual([e['event'] for e in events], ['room.status', 'room.delta', 'room.delta', 'room.line',
                                                        'room.notice', 'room.notice'])
        self.assertTrue(all(e['epoch'] == epoch(1) for e in events))
        self.assertTrue(all('state' not in e and 'revision' not in e for e in events))
        seqs = [e['seq'] for e in events]
        self.assertEqual(seqs, sorted(seqs))
        self.assertEqual(events[0]['status'], opened['status'])
        self.assertEqual([(e['author'], e['lane'], e['kind'], e['text']) for e in events[1:3]],
                         [('scout', 'chat', 'text', 'Looking at '), ('scout', 'chat', 'text', 'the admin panel')])
        live_line = events[3]['line']
        self.assertEqual((live_line['line_seq'], live_line['replay'], live_line['text']),
                         (11, False, 'Looking at the admin panel'))
        self.assertEqual([e['code'] for e in events[4:]], ['stalled', 'recovered'])
        status = replies['2']['result']
        self.assertEqual((status['live'], status['fresh'], status['counts_basis']), (True, True, 'first_400_items'))
        self.assertEqual(status['counts'], dict(open=1, verified=1, returned=1, reports=2))
        scout = next(l for l in status['lanes'] if l['avatar'] == 'scout' and l['lane'] == 'work')
        self.assertEqual(scout['phase'], 'running')
        page = replies['3']['result']
        self.assertEqual((page['epoch'], page['first_line_seq'], page['last_line_seq']), (epoch(1), 1, 11))
        self.assertEqual((len(page['lines']), page['has_more_before'], page['has_more_after']), (11, False, False))
        self.assertEqual([l['line_seq'] for l in replies['4']['result']['lines']], [8, 9, 10, 11])
        self.assertTrue(replies['4']['result']['has_more_before'])
        older = replies['5']['result']
        self.assertEqual(([l['line_seq'] for l in older['lines']], older['has_more_before'], older['has_more_after']),
                         ([3, 4], True, True))
        newer = replies['6']['result']
        self.assertEqual(([l['line_seq'] for l in newer['lines']], newer['has_more_before'], newer['has_more_after']),
                         ([9], True, True))
        self.assertReason(replies['7'], 'invalid_request', 'conflicting_params')
        self.assertReason(replies['8'], 'invalid_state', 'epoch_changed')
        pair = replies['9']['result']['lines']
        self.assertEqual([(l['kind'], l['call_id']) for l in pair], [('tool_call', 'call-1'), ('tool_result', 'call-1')])
        self.assertEqual((pair[0]['tool'], pair[1]['ok'], pair[1]['result']), ('http_get', True, '200 OK, 4 forms'))
        self.assertEqual([l['line_seq'] for l in replies['10']['result']['lines']], [11])
        self.assertReason(replies['11'], 'invalid_request', 'out_of_range')

    def test_room_send_control_interrupt_close(self):
        frames, replies, events = self.frames_by_id('room', [
            call(1, 'room.open', parent_session_id='sess-coding-a'),
            call(2, 'room.send', text='hello team'),
            call(3, 'room.send', text='check it', to='scout'),
            call(4, 'room.send', text='ls', shell=True),
            call(5, 'room.send', text='x', shell=True, to='scout'),
            call(6, 'room.send', text='x', to='clerk'), call(7, 'room.send', text='x', to='nobody'),
            call(8, 'room.send', text='   '), call(9, 'room.send', text='x' * 2097153),
            call(10, 'room.send', text='see', attachments=[dict(kind='pasted_text', text='abc'),
                                                           dict(kind='image_file', path='/tmp/a.png')]),
            call(11, 'room.send', text='see', attachments=[dict(kind='image_file', path='/tmp/refused.png')]),
            call(12, 'room.send', text='see', attachments=[dict(kind='pasted_text', text='a')] * 17),
            call(13, 'room.interrupt'),
            call(14, 'room.control', action='pause', avatar='scout'),
            call(15, 'room.status'),
            call(16, 'room.control', action='resume', avatar='scout'),
            call(17, 'room.control', action='redirect', avatar='scout', note='look at /admin'),
            call(18, 'room.control', action='redirect', avatar='scout'),
            call(19, 'room.control', action='stop', avatar='scout', note='x'),
            call(20, 'room.control', action='stop', avatar='chief'),
            call(21, 'room.control', action='stop', avatar='nobody'),
            call(22, 'room.control', action='explode', avatar='scout'),
            call(23, 'room.close', stop=True), call(24, 'room.close'), call(25, 'room.list', live=True)])
        echoes = [e for e in events if e['event'] == 'room.line' and e['line']['origin'] == 'adapter_echo']
        self.assertEqual([(e['line']['kind'], e['line']['author'], e['line']['text']) for e in echoes],
                         [('user', 'you', 'hello team'), ('user', 'you', '@scout check it'),
                          ('shell', 'shell', '!ls'), ('user', 'you', 'see')])
        first = replies['2']['result']
        self.assertEqual((first['sent'], first['acked'], first['inserts']), (True, True, []))
        self.assertEqual(first['echo_line_seq'], echoes[0]['line']['line_seq'])
        self.assertLess(frames.index(echoes[0]), frames.index(replies['2']))
        self.assertLess(frames.index(replies['2']), frames.index(echoes[1]))
        self.assertReason(replies['5'], 'invalid_request', 'conflicting_params')
        self.assertReason(replies['6'], 'invalid_request', 'clerk_not_addressable')
        self.assertReason(replies['7'], 'not_found', 'unknown_avatar')
        self.assertReason(replies['8'], 'invalid_request', 'out_of_range')
        self.assertReason(replies['9'], 'invalid_request', 'out_of_range')
        self.assertEqual(replies['10']['result']['inserts'], ['[Pasted text #1]', '[Image #2]'])
        self.assertReason(replies['11'], 'invalid_request', 'attachment_refused')
        self.assertReason(replies['12'], 'invalid_request', 'out_of_range')
        self.assertEqual(replies['13']['result'], dict(sent=True))
        self.assertEqual(replies['14']['result'], dict(sent=True))
        self.assertEqual(replies['15']['result']['paused'], ['scout'])
        self.assertEqual(next(l for l in replies['15']['result']['lanes'] if l['avatar'] == 'scout')['phase'], 'paused')
        self.assertTrue(any(e['event'] == 'room.status' and e['status']['paused'] == ['scout'] for e in events))
        self.assertEqual(replies['16']['result'], dict(sent=True))
        self.assertEqual(replies['17']['result'], dict(sent=True))
        self.assertReason(replies['18'], 'invalid_request', 'note_required')
        self.assertReason(replies['19'], 'invalid_request', 'conflicting_params')
        self.assertReason(replies['20'], 'invalid_request', 'no_work_lane')
        self.assertReason(replies['21'], 'not_found', 'unknown_avatar')
        self.assertReason(replies['22'], 'invalid_request', 'out_of_range')
        self.assertEqual(replies['23']['result'], dict(closed=True, stopped=True))
        self.assertBinding(replies['24'], 'wrong_binding', 'unbound', ['room_live', 'room_closed'])
        self.assertEqual(replies['25']['result']['rooms'], [])

    def test_room_stream_error_and_old_room(self):
        frames, replies, events = self.frames_by_id('room', [
            call(1, 'room.open', parent_session_id='sess-coding-b', tail=0),
            call(2, 'room.send', text='plain'),
            call(3, 'room.send', text='see', attachments=[dict(kind='pasted_text', text='a')]),
            call(4, 'room.send', text='fake:stream_error'),
            call(5, 'room.send', text='again'), call(6, 'room.status'), call(7, 'room.interrupt')])
        opened = replies['1']['result']
        self.assertEqual((opened['started'], opened['tail'], opened['daemon']['features'],
                          opened['daemon']['template_revision']), (True, [], [], None))
        self.assertEqual(opened['history']['count'], 3)
        self.assertEqual(opened['roster'], ['chief', 'clerk', 'scout', 'analyst', 'recon'])
        self.assertEqual(replies['2']['result'], dict(sent=True, acked=False, inserts=[],
                                                      echo_line_seq=replies['2']['result']['echo_line_seq']))
        self.assertReason(replies['3'], 'invalid_state', 'peer_unsupported')
        error = replies['4']['error']
        self.assertEqual((error['code'], error['reason'], error['delivery']), ('unavailable', 'stream_error', 'unknown'))
        closed = events[-1]
        self.assertEqual((closed['event'], closed['reason'], closed['session_id']), ('room.closed', 'stream_error', ROOM_B))
        self.assertLess(frames.index(replies['4']), frames.index(closed))
        self.assertBinding(replies['5'], 'room_closed', 'room_closed', ['room_live'])
        self.assertEqual(replies['6']['result']['live'], False)
        self.assertBinding(replies['7'], 'room_closed', 'room_closed', ['room_live'])

    def test_room_open_errors(self):
        _, replies = self.replies('room', [
            call(1, 'room.open', parent_session_id='missing'), call(2, 'room.open', parent_session_id=ROOM_A),
            call(3, 'room.open', parent_session_id='sess-clash'),
            call(4, 'room.open', parent_session_id='sess-coding-c'),
            call(5, 'room.open', parent_session_id='sess-nostart'),
            call(6, 'room.open', parent_session_id='sess-coding-a', tail=201),
            call(7, 'room.open', parent_session_id='sess-coding-a', wait_ms=30001),
            call(8, 'room.attach', session_id=ROOM_B), call(9, 'room.attach', session_id='sess-coding-a'),
            call(10, 'room.attach', session_id='nobody'), call(11, 'room.attach', session_id=ROOM_A, wait_ms=15001),
            call(12, 'room.open'), call(13, 'room.list'), call(14, 'room.list', parent_session_id='sess-coding-b'),
            call(15, 'room.list', live=True), call(16, 'room.list', limit=1),
            call(17, 'room.open', parent_session_id='sess-locked'), call(18, 'room.list', live=True)])
        self.assertReason(replies['1'], 'not_found', 'not_found')
        self.assertReason(replies['2'], 'invalid_request', 'parent_is_room')
        self.assertReason(replies['3'], 'invalid_state', 'room_id_in_use')
        self.assertReason(replies['4'], 'invalid_state', 'no_main_route')
        error = replies['5']['error']
        self.assertEqual((error['code'], error['reason'], error['delivery']), ('unavailable', 'room_start_failed', 'unknown'))
        self.assertIn('log_tail', error['detail'])
        for rid in ('6', '7', '11', '12'):
            self.assertReason(replies[rid], 'invalid_request', 'out_of_range')
        self.assertReason(replies['8'], 'invalid_state', 'not_running')
        self.assertReason(replies['9'], 'invalid_state', 'not_a_room')
        self.assertReason(replies['10'], 'invalid_state', 'not_running')
        rooms = replies['13']['result']['rooms']
        self.assertEqual([r['session_id'] for r in rooms], [ROOM_A, ROOM_B])
        self.assertEqual([r['session_id'] for r in replies['14']['result']['rooms']], [ROOM_B])
        self.assertEqual([r['session_id'] for r in replies['15']['result']['rooms']], [ROOM_A])
        self.assertIsNotNone(replies['16']['result']['next_cursor'])
        self.assertEqual(replies['17']['result']['started'], True)
        self.assertEqual(len(replies['18']['result']['rooms']), 2)

    def test_room_disk_reads(self):
        _, replies = self.replies('room', [
            call(1, 'room.open', parent_session_id='sess-coding-a'),
            call(2, 'room.shelf.list'), call(3, 'room.shelf.list', prefix='findings/', limit=2),
            call(4, 'room.item', key='findings/f-001'), call(5, 'room.item', key='findings/none'),
            call(6, 'room.item', key='../x'), call(7, 'room.item'),
            call(8, 'room.trajectory'), call(9, 'room.trajectory', goal='trajectory/web-recon'),
            call(10, 'room.trajectory', goal='other'),
            call(11, 'room.reports'), call(12, 'room.reports', include_markdown=True, limit=1),
            call(13, 'room.spend'), call(14, 'room.spend', goal='other'), call(15, 'room.shelf.list', limit=501)])
        listed = replies['2']['result']
        self.assertEqual((listed['total'], len(listed['rows']), len(listed['fingerprint'])), (8, 8, 16))
        self.assertEqual(set(listed['rows'][0]), {'key', 'revision', 'updated_at', 'kind', 'title', 'status'})
        paged = replies['3']['result']
        self.assertEqual(([r['key'] for r in paged['rows']], paged['total']),
                         (['findings/f-001', 'findings/f-002'], 3))
        self.assertIsNotNone(paged['next_cursor'])
        item = replies['4']['result']
        self.assertEqual((item['kind'], item['status'], item['truncated'], item['json']), ('finding', 'verified', False, None))
        self.assertEqual(item['content'], "Payload: a' OR 1=1 --")
        self.assertReason(replies['5'], 'not_found', 'not_found')
        self.assertReason(replies['6'], 'invalid_request', 'invalid_name')
        self.assertReason(replies['7'], 'invalid_request', 'out_of_range')
        goal = replies['8']['result']['goals'][0]
        self.assertEqual((goal['goal_key'], len(goal['nodes']), len(goal['edges']), goal['nodes_truncated']),
                         ('trajectory/web-recon', 3, 2, False))
        self.assertEqual(replies['9']['result'], replies['8']['result'])
        self.assertEqual(replies['10']['result'], dict(goals=[], goals_truncated=False))
        reports = replies['11']['result']['reports']
        self.assertEqual([r['key'] for r in reports], ['reports/r-001', 'reports/r-002'])
        self.assertEqual((reports[0]['markdown'], reports[0]['finding_keys']), (None, ['findings/f-001', 'findings/f-003']))
        self.assertEqual(replies['12']['result']['reports'][0]['markdown'], '# Web recon\n\nOne high finding.\n')
        self.assertIsNotNone(replies['12']['result']['next_cursor'])
        spend = replies['13']['result']
        self.assertEqual(spend['goals'][0]['goal_key'], 'trajectory/web-recon')
        self.assertEqual(spend['ledger']['ledger'], 'ok')
        self.assertEqual([a['avatar'] for a in spend['ledger']['by_avatar']], ['analyst', 'chief', 'scout'])
        self.assertEqual(spend['ledger']['totals']['calls'], 4)
        self.assertEqual(replies['14']['result']['goals'], [])
        self.assertReason(replies['15'], 'invalid_request', 'out_of_range')

    def test_room_team(self):
        _, replies = self.replies('room', [
            call(1, 'room.open', parent_session_id='sess-coding-a'),
            call(2, 'room.team.get'), call(3, 'room.team.get', target='template'),
            call(4, 'room.models.set', target='room', avatar='scout', lane='chat', model='model-deep', effort='high'),
            call(5, 'room.models.set', target='room', avatar='scout', lane='chat', model='nope'),
            call(6, 'room.models.set', target='room', avatar='chief', lane='work', model='model-main'),
            call(7, 'room.models.set', target='room', avatar='clerk', lane='chat', effort='low'),
            call(8, 'room.models.set', target='room', avatar='nobody', lane='chat', effort='low'),
            call(9, 'room.models.set', target='room', avatar='scout', lane='chat'),
            call(10, 'room.models.set', target='room', avatar='scout', lane='chat', effort='a b'),
            call(11, 'room.models.set', target='room', avatar='scout', lane='chat', model=None),
            call(12, 'room.models.set', target='room', avatar='scout', lane='chat', effort='low', if_revision='0' * 16),
            call(13, 'room.models.set', target='room', avatar='scout', lane='work', effort='room-rejects'),
            call(14, 'room.avatars.upsert', target='room', id='spy', role='analyst', display='Spy', alias='sp'),
            call(15, 'room.avatars.upsert', target='room', id='spy2', role='analyst', alias='SP'),
            call(16, 'room.avatars.upsert', target='room', id='spy', role='scout'),
            call(17, 'room.avatars.upsert', target='room', id='chief2', role='chief'),
            call(18, 'room.avatars.upsert', target='room', id='ghost', role='nobody'),
            call(19, 'room.avatars.upsert', target='room', id='Bad Id', role='scout'),
            call(20, 'room.avatars.upsert', target='room', id='norole'),
            call(21, 'room.avatars.upsert', target='room', id='spy', display=None),
            call(22, 'room.team.get'),
            call(23, 'room.avatars.remove', target='room', id='probe'),
            call(24, 'room.avatars.remove', target='room', id='spy'),
            call(25, 'room.avatars.remove', target='room', id='spy'),
            call(26, 'room.models.set', target='template', avatar='recon', lane='work', model='model-bare'),
            call(27, 'room.avatars.remove', target='template', id='recon'),
            call(28, 'room.team.get', target='room'), call(29, 'room.avatars.upsert', id='x', role='scout'),
            call(30, 'room.team.get', target='elsewhere'),
            call(31, 'room.models.set', target='room', avatar='scout', lane='chat', model='')])
        team = replies['2']['result']
        self.assertEqual((team['target'], team['choices_source'], team['template_pending_restart']),
                         ('room', 'room_daemon', False))
        avatars = {a['id']: a for a in team['avatars']}
        self.assertEqual(list(avatars), ['chief', 'clerk', 'scout', 'analyst', 'recon', 'probe'])
        self.assertEqual([l['lane'] for l in avatars['chief']['lanes']], ['chat'])
        self.assertEqual([l['lane'] for l in avatars['clerk']['lanes']], ['work'])
        self.assertEqual((avatars['recon']['defined_in'], avatars['probe']['defined_in']), ('template', 'session'))
        self.assertEqual((avatars['probe']['removable'], avatars['recon']['removable'], avatars['scout']['removable']),
                         (True, False, False))
        chat = avatars['scout']['lanes'][0]
        self.assertEqual((chat['model'], chat['provider_uuid'], chat['model_label'], chat['resolves']),
                         (dict(value='model-fast', origin='role'), dict(value='prov-local', origin='role'),
                          'Fast model', True))
        self.assertEqual(avatars['probe']['lanes'][1]['model']['origin'], 'role')
        self.assertEqual(avatars['probe']['lanes'][0]['model'], dict(value='model-fast', origin='session'))
        self.assertEqual([r['id'] for r in team['roles']], ['chief', 'clerk', 'scout', 'analyst'])
        self.assertEqual(replies['3']['result']['target'], 'template')
        self.assertEqual([a['id'] for a in replies['3']['result']['avatars']], ['chief', 'clerk', 'scout', 'analyst', 'recon'])
        self.assertEqual(replies['3']['result']['choices_source'], 'config')
        changed = replies['4']['result']
        self.assertEqual((changed['applies'], changed['propagation']), ('live', dict(room='reloaded')))
        lane = next(a for a in changed['team']['avatars'] if a['id'] == 'scout')['lanes'][0]
        self.assertEqual((lane['model'], lane['provider_uuid'], lane['effort']), (
            dict(value='model-deep', origin='session'), dict(value='prov-local', origin='session'),
            dict(value='high', origin='session')))
        self.assertNotEqual(changed['team']['revision'], team['revision'])
        self.assertReason(replies['5'], 'not_found', 'unknown_model')
        self.assertReason(replies['6'], 'invalid_request', 'no_such_lane')
        self.assertReason(replies['7'], 'invalid_request', 'no_such_lane')
        self.assertReason(replies['8'], 'not_found', 'unknown_avatar')
        self.assertReason(replies['9'], 'invalid_request', 'missing_params')
        self.assertReason(replies['10'], 'invalid_request', 'out_of_range')
        self.assertReason(replies['31'], 'invalid_request', 'out_of_range')
        cleared = next(a for a in replies['11']['result']['team']['avatars'] if a['id'] == 'scout')['lanes'][0]
        self.assertEqual((cleared['model']['origin'], cleared['effort']), ('role', dict(value=None, origin='default')))
        self.assertReason(replies['12'], 'invalid_state', 'stale')
        rejected = replies['13']['error']
        self.assertEqual((rejected['reason'], rejected['delivery'], rejected['detail']['warnings'][0]['layer']),
                         ('rejected_by_room', 'not_sent', 'session'))
        added = next(a for a in replies['14']['result']['team']['avatars'] if a['id'] == 'spy')
        self.assertEqual((added['alias']['value'], added['display']['value'], added['role'], added['removable']),
                         ('SP', 'Spy', 'analyst', True))
        self.assertReason(replies['15'], 'invalid_request', 'layer_rejected')
        self.assertEqual(replies['15']['error']['detail']['warnings'][0]['path'], 'avatars.toml')
        self.assertReason(replies['16'], 'invalid_request', 'role_immutable')
        self.assertReason(replies['17'], 'invalid_request', 'reserved_id')
        self.assertReason(replies['18'], 'not_found', 'unknown_role')
        self.assertReason(replies['19'], 'invalid_request', 'invalid_name')
        self.assertReason(replies['20'], 'invalid_request', 'missing_params')
        self.assertIsNone(next(a for a in replies['21']['result']['team']['avatars'] if a['id'] == 'spy')['display']['value'])
        self.assertIn('spy', [a['id'] for a in replies['22']['result']['avatars']])
        self.assertReason(replies['23'], 'invalid_state', 'lane_running')
        removed = replies['24']['result']
        self.assertEqual((removed['reverted_to'], 'spy' in [a['id'] for a in removed['team']['avatars']]), (None, False))
        self.assertReason(replies['25'], 'not_found', 'unknown_avatar')
        edited = replies['26']['result']
        self.assertEqual((edited['applies'], edited['propagation']), ('next_start', dict(room='not_live')))
        recon_lane = next(a for a in edited['team']['avatars'] if a['id'] == 'recon')['lanes'][1]
        self.assertEqual((recon_lane['model'], recon_lane['provider_uuid']['value']), (dict(value='bare/model', origin='template'), None))
        self.assertTrue(edited['team']['avatars'][0]['id'] == 'chief')
        self.assertEqual(replies['27']['result']['reverted_to'], None)
        self.assertEqual(replies['28']['result']['template_pending_restart'], True)
        self.assertEqual([a['id'] for a in replies['28']['result']['avatars']], ['chief', 'clerk', 'scout', 'analyst', 'probe'])
        self.assertReason(replies['29'], 'invalid_request', 'target_required')
        self.assertReason(replies['30'], 'invalid_request', 'out_of_range')

    def test_room_team_without_room(self):
        _, replies = self.replies('room', [
            call(1, 'room.models.set', target='template', avatar='scout', lane='chat', model='model-deep'),
            call(2, 'room.team.get'), call(3, 'room.avatars.upsert', target='template', id='recon', display='Recon two'),
            call(4, 'room.avatars.remove', target='template', id='recon', if_revision='absent'),
            call(5, 'room.avatars.remove', target='template', id='recon'),
            call(6, 'room.avatars.remove', target='template', id='scout'),
            call(7, 'room.open', parent_session_id='sess-coding-a'), call(8, 'room.team.get')])
        written = replies['1']['result']
        self.assertEqual((written['applies'], written['team']['target'], written['team']['layer_present']),
                         ('next_start', 'template', True))
        scout = next(a for a in written['team']['avatars'] if a['id'] == 'scout')
        self.assertEqual((scout['lanes'][0]['model']['origin'], scout['removable']), ('template', True))
        self.assertEqual(replies['2']['result']['target'], 'template')
        self.assertEqual(next(a for a in replies['3']['result']['team']['avatars'] if a['id'] == 'recon')['display']['value'],
                         'Recon two')
        self.assertReason(replies['4'], 'invalid_state', 'stale')
        self.assertEqual(replies['5']['result']['reverted_to'], None)
        self.assertEqual(replies['6']['result']['reverted_to'], 'base')
        self.assertEqual(replies['6']['result']['team']['layer_present'], False)
        self.assertEqual(replies['6']['result']['team']['revision'], 'absent')
        self.assertEqual(replies['8']['result']['template_pending_restart'], False)

    def test_room_old_room_team_writes(self):
        _, replies = self.replies('room', [
            call(1, 'room.open', parent_session_id='sess-coding-b'),
            call(2, 'room.models.set', target='room', avatar='scout', lane='chat', model='model-deep'),
            call(3, 'room.team.get')])
        self.assertEqual((replies['2']['result']['applies'], replies['2']['result']['propagation']),
                         ('next_reload', dict(room='not_supported')))
        self.assertFalse(replies['3']['result']['template_pending_restart'])

    def test_room_gap(self):
        frames, replies, events = self.frames_by_id('room_gap', [
            call(1, 'room.open', parent_session_id='sess-coding-a', tail=0), call(2, 'room.status'),
            call(3, 'room.history', epoch=epoch(1), after_line_seq=11, limit=2),
            call(4, 'room.history', epoch=epoch(1), after_line_seq=11),
            call(5, 'room.attach', session_id=ROOM_A)])
        self.assertEqual((replies['1']['result']['tail'], replies['1']['result']['history']['last_line_seq']), ([], 10))
        self.assertEqual([e['event'] for e in events[:3]], ['room.status', 'room.line', 'room.gap'])
        self.assertEqual(events[1]['line']['line_seq'], 11)
        gap = events[2]
        self.assertEqual((gap['epoch'], gap['after_line_seq'], gap['dropped_lines'], gap['dropped_deltas']),
                         (epoch(1), 11, 3, 2))
        self.assertEqual(replies['2']['result']['live'], True)
        first = replies['3']['result']
        self.assertEqual(([l['line_seq'] for l in first['lines']], first['has_more_after']), ([12, 13], True))
        rest = replies['4']['result']
        self.assertEqual(([l['line_seq'] for l in rest['lines']], rest['has_more_after'], rest['last_line_seq']),
                         ([12, 13, 14], False, 14))
        self.assertReason(replies['5'], 'invalid_state', 'already_bound')
        self.assertEqual(len(events), 3)

    def test_room_takeover(self):
        frames, replies, events = self.frames_by_id('room_takeover', [
            call(1, 'room.open', parent_session_id='sess-coding-a'), call(2, 'room.send', text='hi'),
            call(3, 'room.interrupt'), call(4, 'room.control', action='pause', avatar='scout'),
            call(5, 'room.status'), call(6, 'room.history', epoch=epoch(1), limit=2), call(7, 'room.shelf.list'),
            call(8, 'room.team.get'),
            call(9, 'room.models.set', target='room', avatar='scout', lane='chat', effort='low'),
            call(10, 'room.attach', session_id=ROOM_A), call(11, 'room.send', text='hi again'),
            call(12, 'room.history', epoch=epoch(1)), call(13, 'room.history', epoch=epoch(2), limit=1)])
        self.assertEqual([e['event'] for e in events], ['room.status', 'room.closed', 'room.line'])
        closed = events[1]
        self.assertEqual((closed['epoch'], closed['session_id'], closed['reason']), (epoch(1), ROOM_A, 'taken_over'))
        self.assertLess(frames.index(replies['1']), frames.index(closed))
        for rid in ('2', '3', '4'):
            self.assertBinding(replies[rid], 'room_closed', 'room_closed', ['room_live'], 'the room connection is closed')
        status = replies['5']['result']
        self.assertEqual((status['live'], status['fresh'], status['lanes'], status['events'], status['counts_basis']),
                         (False, False, [], [], 'disk_scan'))
        self.assertEqual(len(replies['6']['result']['lines']), 2)
        self.assertEqual(replies['7']['result']['total'], 8)
        self.assertEqual(replies['8']['result']['choices_source'], 'config')
        written = replies['9']['result']
        self.assertEqual((written['applies'], written['propagation']), ('next_reload', dict(room='not_live')))
        self.assertEqual(replies['10']['result']['epoch'], epoch(2))
        self.assertEqual(replies['11']['result']['sent'], True)
        self.assertReason(replies['12'], 'invalid_state', 'epoch_changed')
        self.assertEqual(replies['13']['result']['epoch'], epoch(2))
        self.assertEqual((events[2]['epoch'], events[2]['line']['text']), (epoch(2), 'hi again'))

    def test_room_close_then_rebind(self):
        _, replies, events = self.frames_by_id('room', [
            call(1, 'room.open', parent_session_id='sess-coding-a'), call(2, 'room.close'),
            call(3, 'room.send', text='x'), call(4, 'create', cwd='/tmp'),
            call(5, 'room.attach', session_id=ROOM_A), call(6, 'detach')])
        self.assertEqual(replies['2']['result'], dict(closed=True, stopped=False))
        self.assertBinding(replies['3'], 'wrong_binding', 'unbound', ['room_live'])
        self.assertIn('session_info', replies['4']['result'])
        self.assertBinding(replies['5'], 'already_bound', 'coding', ['unbound', 'room_closed'])
        self.assertEqual(replies['6']['result'], dict(detached=True))

    def test_fixtures(self):
        schema = validator()
        if schema is None:
            self.skipTest('jsonschema is not installed')
        paths = sorted((PROTOCOL / 'fixtures').glob('*.jsonl'))
        self.assertIn('representative.jsonl', [p.name for p in paths])
        for path in paths:
            with self.subTest(fixture=path.name):
                for number, line in enumerate(path.read_text().splitlines(), 1):
                    if not line.strip():
                        continue
                    frame = json.loads(line)
                    self.assertIsInstance(frame, dict, '%s:%d' % (path.name, number))
                    errors = sorted(schema.iter_errors(frame), key=lambda e: list(e.path))
                    self.assertFalse(errors, '%s:%d: %s' % (path.name, number, errors[0].message if errors else ''))

if __name__ == '__main__':
    unittest.main()
