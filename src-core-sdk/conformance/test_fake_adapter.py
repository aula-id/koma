"""Stdlib subprocess smoke tests; optional JSON Schema fixture validation."""
import json
from pathlib import Path
import subprocess
import sys
import unittest

HERE = Path(__file__).resolve().parent
PROTOCOL = HERE.parent / 'protocol'
V1 = ['create', 'resume', 'state', 'submit', 'enqueue', 'interrupt', 'interrupt_and_submit', 'approve',
      'configure', 'detach']
CONFIG = ['config.get', 'config.providers.delete', 'config.models.delete', 'config.roles.unassign',
          'config.effort.get', 'config.effort.set', 'config.limits.get', 'config.limits.set']
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
        self.assertEqual(hello['methods'], V1 + CONFIG + ['sessions.info'])
        self.assertEqual(hello['features'], ['errors.reason', 'state.session_info', 'config.lock'])
        self.assertEqual(hello['events'], ['state', 'output_append', 'turn'])
        self.assertEqual(hello['limits']['request_ids'], 65536)
        self.assertEqual(hello['limits']['config'], dict(lock_wait_ms=3000))
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
        # Room methods are not implemented, so their names are unknown.
        self.assertEqual(replies['6']['error']['code'], 'unsupported_method')
        # The role is checked before the binding.
        self.assertReason(replies['7'], 'invalid_request', 'out_of_range')
        self.assertEqual(replies['8']['result']['view'], 'global')
        self.assertEqual(replies['9']['error']['code'], 'unsupported_method')
        # Unbound detach keeps its v1 error, with no added keys, and ends the adapter.
        self.assertEqual(replies['10']['error'], dict(code='invalid_state', message='no session bound',
                                                      delivery='not_sent'))
        self.assertNotIn('11', replies)
        self.assertEqual(result.returncode, 0)

    def test_wrong_binding_coding(self):
        _, replies = self.replies('wrong_binding', [
            call(1, 'create', cwd='/tmp'), call(2, 'resume', session_id='other'), call(3, 'room.send', text='hi'),
            call(4, 'config.roles.unassign', role='planner', scope='session', if_revision='x'),
            call(5, 'sessions.info'), call(6, 'state')])
        self.assertIn('session_info', replies['1']['result'])
        self.assertBinding(replies['2'], 'already_bound', 'coding', ['unbound'], 'session already bound')
        self.assertEqual(replies['3']['error']['code'], 'unsupported_method')
        self.assertReason(replies['4'], 'invalid_request', 'out_of_range')
        self.assertEqual(set(replies['5']['result']), {'session_info'})
        self.assertEqual(replies['6']['result']['state']['revision'], 1)

    def test_binding_errors_validate_against_schema(self):
        schema = validator()
        if schema is None:
            self.skipTest('jsonschema is not installed')
        _, frames = self.run_peer('wrong_binding', [
            call(1, 'state'), call(2, 'create', cwd='/tmp'), call(3, 'create', cwd='/tmp'), call(4, 'state'),
            call(5, 'room.send', text='hi')])
        for frame in frames:
            schema.validate(frame)

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
