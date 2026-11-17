import asyncio
import os
from pathlib import Path
import sys
import tempfile
import unittest

from koma_sdk import (Client, ConfigFile, ConfigView, DeliveryUnknown, EffortState, IncompatibleAdapter,
                      KeyState, MainFallback, PropagationStatus, ProviderKind, PublicRole, RemoteError,
                      RouteOrigin, RouteReason, SessionInfo)
from koma_sdk.client import _validate_result

FAKE = Path(__file__).resolve().parents[2] / 'conformance' / 'fake_adapter.py'
STATE = dict(session_id='s', revision=1, external_approvals=False, pending_approval=None, gate=None, cwd='/w',
             model_id='m', policy=dict(plan='forbid', approval='reject'), turn=None, working=False,
             awaiting_approval=False, approval_reason=None, pending_count=0, messages=[], streaming=None)
SECRETS = ('sk-fake-planted-0000000000', 'fake-pass', 'fake-query', 'fake-oauth-token-0000')


class ConfigConformance(unittest.IsolatedAsyncioTestCase):
    def client(self, scenario='config', **kwargs):
        c = Client(sys.executable, adapter_args=(str(FAKE), '--scenario', scenario), close_timeout=.1, **kwargs)
        self.addAsyncCleanup(c.close)
        return c

    def logged(self):
        log = tempfile.NamedTemporaryFile(prefix='koma-config-', delete=False)
        log.close()
        os.environ['KOMA_FAKE_LOG'] = log.name
        def cleanup():
            os.environ.pop('KOMA_FAKE_LOG', None)
            Path(log.name).unlink(missing_ok=True)
        self.addCleanup(cleanup)
        return Path(log.name)

    async def test_hello_advertises_methods_features_events_limits(self):
        hello = await self.client().probe()
        self.assertIn('config.get', hello.methods)
        self.assertIn('sessions.info', hello.methods)
        self.assertIn('errors.reason', hello.features)
        self.assertEqual(hello.events[:3], ('state', 'output_append', 'turn'))
        self.assertIn('room.gap', hello.events)
        self.assertEqual(hello.limits['request_ids'], 65536)
        self.assertEqual(hello.request_ids, 65536)
        self.assertTrue(hello.supports('config.limits.set'))
        self.assertTrue(hello.supports('room.send'))
        self.assertTrue(hello.has_feature('create.name'))
        self.assertEqual(len(hello.capabilities), 10)

    async def test_host_config_view_is_typed_and_redacted(self):
        async with self.client().host() as host:
            view = await host.config_get()
        self.assertIsInstance(view, ConfigView)
        self.assertIs(view.file, ConfigFile.OK)
        self.assertEqual(len(view.revision), 16)
        local = next(p for p in view.providers if p.provider_uuid == 'prov-local')
        self.assertEqual(local.endpoint, 'https://llm.example.test/v1')
        self.assertTrue(local.endpoint_redacted)
        self.assertIs(local.key_state, KeyState.SET)
        self.assertTrue(next(p for p in view.providers if p.provider_uuid == 'prov-ext').managed)
        models = {m.model_uuid: m for m in view.models}
        self.assertEqual(models['model-planner'].roles, (PublicRole.PLANNER,))
        self.assertTrue(models['model-planner'].other_roles)
        self.assertIs(models['model-orphan'].provider_kind, ProviderKind.MISSING)
        self.assertIs(models['model-oauth'].provider_kind, ProviderKind.OAUTH)
        self.assertIs(view.route('main').origin, RouteOrigin.GLOBAL)
        self.assertIs(view.route(PublicRole.COMPACTOR).origin, RouteOrigin.INHERIT_MAIN)
        self.assertIsNone(view.main_fallback)
        for secret in SECRETS:
            self.assertNotIn(secret, repr(view))

    async def test_host_dry_run_then_guarded_delete(self):
        log = self.logged()
        async with self.client().host() as host:
            before = await host.config_get()
            dry = await host.config_providers_delete('prov-local', if_revision=before.revision, dry_run=True)
            self.assertTrue(dry.dry_run)
            self.assertEqual(set(dry.removed.models_removed), {'model-main', 'model-planner'})
            self.assertTrue(dry.removed.main_reset)
            self.assertEqual(dry.config.revision, before.revision)
            self.assertEqual(dry.propagation.bound, None)
            self.assertEqual((await host.config_get()).revision, before.revision)
            done = await host.config_providers_delete('prov-local', if_revision=before.revision)
            self.assertFalse(done.dry_run)
            self.assertNotEqual(done.config.revision, before.revision)
            self.assertIs(done.propagation.bound, PropagationStatus.NOT_BOUND)
            self.assertIs(done.propagation.rebind, PropagationStatus.DONE)
            self.assertIs(done.config.main_fallback, MainFallback.UNCONFIGURED)
            self.assertIsNone(done.session_info)
            with self.assertRaises(RemoteError) as ctx:
                await host.config_providers_delete('prov-local')
            self.assertEqual((ctx.exception.code, ctx.exception.reason), ('not_found', 'not_found'))
        self.assertIn('"dry_run": true', log.read_text())

    async def test_host_refusals_carry_reason(self):
        async with self.client().host() as host:
            with self.assertRaises(RemoteError) as ctx:
                await host.config_providers_delete('prov-local', if_revision='0000000000000000')
            self.assertEqual((ctx.exception.code, ctx.exception.reason), ('invalid_state', 'stale'))
            self.assertFalse(ctx.exception.written)
            with self.assertRaises(RemoteError) as ctx:
                await host.config_providers_delete('prov-ext')
            self.assertEqual(ctx.exception.reason, 'managed')
            with self.assertRaises(RemoteError) as ctx:
                await host.config_models_delete('model-missing')
            self.assertEqual(ctx.exception.reason, 'not_found')
            released = await host.config_models_delete('model-planner')
            self.assertEqual(released.removed.roles_released, (PublicRole.PLANNER,))
            self.assertIs(released.config.route('planner').origin, RouteOrigin.INHERIT_MAIN)
        with self.assertRaises(ValueError):
            await self.client().host().config_roles_unassign('cyber-red')
        with self.assertRaises(ValueError):
            await self.client().host().config_models_delete('')

    async def test_unverified_delete_is_delivery_unknown(self):
        async with self.client().host() as host:
            with self.assertRaises(DeliveryUnknown) as ctx:
                await host.config_models_delete('model-unverified')
            cause = ctx.exception.cause
            self.assertIsInstance(cause, RemoteError)
            self.assertEqual(cause.reason, 'unverified')
            self.assertTrue(cause.written)
            self.assertEqual(cause.detail, {'diff': ['model-unverified']})
            # Unbound, the adapter stays up; the deleted entry is gone.
            self.assertNotIn('model-unverified', [m.model_uuid for m in (await host.config_get()).models])

    async def test_host_role_unassign_global(self):
        async with self.client().host() as host:
            first = await host.config_roles_unassign('planner')
            self.assertTrue(first.changed)
            self.assertEqual(first.previous_model_uuid, 'model-planner')
            again = await host.config_roles_unassign(PublicRole.PLANNER, if_revision=first.config.revision)
            self.assertFalse(again.changed)
            self.assertEqual(again.config.revision, first.config.revision)
            main = await host.config_roles_unassign('main')
            self.assertIs(main.config.main_fallback, MainFallback.UNCONFIGURED)
            self.assertIs(main.config.route('main').reason, RouteReason.UNASSIGNED)

    async def test_host_subagent_risky_set(self):
        async with self.client().host() as host:
            before = (await host.config_get()).revision
            first = await host.config_subagent_risky_set('parent')
            self.assertTrue(first.changed)
            self.assertEqual(first.mode, 'parent')
            self.assertEqual(first.config.view.value, 'global')
            self.assertNotEqual(first.config.revision, before)
            again = await host.config_subagent_risky_set('parent', if_revision=first.config.revision)
            self.assertFalse(again.changed)
            with self.assertRaises(RemoteError) as ctx:
                await host.config_subagent_risky_set('deny', if_revision='0000000000000000')
            self.assertEqual(ctx.exception.reason, 'stale')
            with self.assertRaises(RemoteError) as ctx:
                await host.config_subagent_risky_set('allow')
            self.assertEqual(ctx.exception.reason, 'out_of_range')
            reset = await host.config_subagent_risky_set('classifier')
            self.assertEqual((reset.changed, reset.mode), (True, 'classifier'))
            with self.assertRaises(TypeError):
                await host.config_subagent_risky_set(None)

    async def test_session_subagent_risky_set_keeps_session_info_fresh(self):
        s = await self.client().create('/fake/work')
        before = s.session_info
        result = await s.config_subagent_risky_set('deny')
        self.assertTrue(result.changed)
        self.assertEqual(result.propagation.bound.value, 'reloaded')
        self.assertGreater(s.session_info.observed_at_ms, before.observed_at_ms)

    async def test_host_configure_feeds_the_view(self):
        async with self.client('normal').host() as host:
            self.assertIs((await host.config_get()).file, ConfigFile.MISSING)
            await host.configure(providers=[dict(name='p', endpoint='http://127.0.0.1:9/v1', api_type='openai',
                                                 api_key='sk-host-0000000000')],
                                 models=dict(main=dict(provider='p', model_id='m')))
            view = await host.config_get()
            self.assertIs(view.providers[0].key_state, KeyState.SET)
            self.assertEqual(view.route('main').model_uuid, 'model-main')
            self.assertNotIn('sk-host-0000000000', repr(view))

    async def test_host_recycles_adapter_before_limit(self):
        host = self.client().host(recycle_after=2)
        self.addAsyncCleanup(host.close)
        self.assertIsNone(host.requests_remaining)
        await host.config_get()
        first = host._transport
        self.assertEqual(host.requests_remaining, 1)
        await host.config_get()
        self.assertIs(host._transport, first)
        self.assertEqual(host.requests_remaining, 0)
        await host.config_get()
        self.assertIsNot(host._transport, first)
        self.assertTrue(first._closed)
        self.assertEqual(host.requests_remaining, 1)
        # Concurrent calls straddling a recycle all complete; the old adapter closes once idle.
        views = await asyncio.gather(*(host.config_get() for _ in range(5)))
        self.assertEqual(len({v.revision for v in views}), 1)
        with self.assertRaises(ValueError):
            self.client().host(recycle_after=0)

    async def test_host_replaces_failed_adapter_without_retrying(self):
        host = self.client().host()
        self.addAsyncCleanup(host.close)
        await host.config_get()
        first = host._transport
        await first.close()
        view = await host.config_get()
        self.assertIsNot(host._transport, first)
        self.assertIs(view.file, ConfigFile.OK)

    async def test_session_info_and_coding_scoped_calls(self):
        s = await self.client().create('/fake/work')
        info = s.session_info
        self.assertIsInstance(info, SessionInfo)
        self.assertEqual(info.effort, 'default')
        self.assertIs(info.route('main').origin, RouteOrigin.SESSION)
        self.assertEqual(info.daemon.version, '0.0.0-fake')
        self.assertIsNone(info.usage)
        remaining = s.requests_remaining
        self.assertEqual(remaining, 65536 - 1)

        loading = await s.effort_get(wait_ms=0)
        self.assertIs(loading.state, EffortState.LOADING)
        ready = await s.effort_get()
        self.assertIs(ready.state, EffortState.READY)
        self.assertEqual(ready.options[0], 'default')
        self.assertEqual(ready.main_model_uuid, 'model-main')
        result = await s.effort_set('high', if_main_model=ready.main_model_uuid)
        self.assertEqual((result.effort, result.superseded), ('high', False))
        self.assertEqual(s.session_info.effort, 'high')
        with self.assertRaises(RemoteError) as ctx:
            await s.effort_set('turbo')
        self.assertEqual((ctx.exception.code, ctx.exception.reason), ('invalid_request', 'not_offered'))
        with self.assertRaises(RemoteError) as ctx:
            await s.effort_set('low', if_main_model='model-other')
        self.assertEqual(ctx.exception.reason, 'main_changed')
        with self.assertRaises(RemoteError) as ctx:
            await s.effort_set('low', if_main_model=None)
        self.assertEqual(ctx.exception.reason, 'main_changed')
        with self.assertRaises(ValueError):
            await s.effort_set('two words')

        revision = (await s.get_state()).revision
        limits = await s.limits_set(max_output_tokens=4096)
        self.assertEqual(limits.max_output_tokens, 4096)
        self.assertEqual(limits.effective_window, 128000)
        self.assertEqual(s.session_info.limits.max_output_tokens, 4096)
        state = await s.get_state()
        self.assertEqual(state.revision, revision)
        self.assertEqual((await s.info()).limits.max_output_tokens, 4096)
        self.assertEqual((await s.limits_get()).max_output_tokens, 4096)
        with self.assertRaises(ValueError):
            await s.limits_set()
        with self.assertRaises(RemoteError) as ctx:
            await s.limits_set(max_output_tokens=10)
        self.assertEqual(ctx.exception.reason, 'out_of_range')

        session_view = await s.config_get()
        self.assertEqual(session_view.view.value, 'session')
        self.assertIs(session_view.route('main').origin, RouteOrigin.SESSION)
        self.assertEqual((await s.config_get(view='global')).view.value, 'global')
        cleared = await s.clear_main_pin()
        self.assertTrue(cleared.changed)
        self.assertEqual(cleared.previous_model_uuid, 'model-main')
        self.assertIs(cleared.config.route('main').origin, RouteOrigin.GLOBAL)
        self.assertEqual(s.session_info.effort, 'default')
        self.assertIs(s.session_info.route('main').origin, RouteOrigin.GLOBAL)
        self.assertFalse((await s.clear_main_pin()).changed)
        self.assertLess(s.requests_remaining, remaining)

    async def test_session_global_writes(self):
        s = await self.client().create('/fake/work')
        dry = await s.config_providers_delete('prov-local', dry_run=True)
        self.assertTrue(dry.dry_run)
        self.assertIn('model-main', dry.removed.models_removed)
        before = s.session_info
        released = await s.config_roles_unassign('planner')
        self.assertTrue(released.changed)
        self.assertEqual(released.config.view.value, 'global')
        self.assertGreater(s.session_info.observed_at_ms, before.observed_at_ms)
        deleted = await s.config_models_delete('model-orphan')
        self.assertEqual(deleted.propagation.bound.value, 'applied_by_daemon')
        with self.assertRaises(RemoteError) as ctx:
            await s.config_providers_delete('prov-ext')
        self.assertEqual(ctx.exception.reason, 'managed')

    async def test_coding_unknown_outcome_ends_adapter(self):
        log = self.logged()
        s = await self.client().create('/fake/work')
        with self.assertRaises(DeliveryUnknown) as ctx:
            await s.config_models_delete('model-unverified')
        self.assertTrue(ctx.exception.cause.written)
        self.assertEqual(ctx.exception.session_id, s.session_id)
        with self.assertRaises(Exception):
            await s.info()
        self.assertEqual(log.read_text().count('config.models.delete'), 1)

    async def test_session_info_ordered_by_observation_not_revision(self):
        from dataclasses import replace
        s = await self.client().create('/fake/work')
        # A state readback at an unchanged revision still carries fresher info.
        await s.limits_set(context_model_alias='fake/alias')
        s._info = replace(s._info, limits=replace(s._info.limits, context_model_alias=''),
                          observed_at_ms=s._info.observed_at_ms - 1)
        state_before = s._state
        await s.get_state()
        self.assertIs(s._state, state_before)
        self.assertEqual(s.session_info.limits.context_model_alias, 'fake/alias')
        current = s.session_info
        older = replace(current, effort='low', observed_at_ms=current.observed_at_ms - 1)
        self.assertIs(s._update_info(older), current)
        same = replace(current, effort='low')
        self.assertIs(s._update_info(same), current)
        newer = replace(current, effort='high', observed_at_ms=current.observed_at_ms + 1)
        self.assertIs(s._update_info(newer), newer)

    async def test_mutation_flags(self):
        s = await self.client().create('/fake/work')
        flags = {}
        original = s._transport.request
        async def record(method, params, *, mutation, **kwargs):
            flags[method] = mutation
            return await original(method, params, mutation=mutation, **kwargs)
        s._transport.request = record
        await s.info()
        await s.config_get()
        await s.effort_get()
        await s.effort_set('low')
        await s.limits_get()
        await s.limits_set(context_window_limit=0)
        await s.clear_main_pin()
        self.assertEqual(flags, {'sessions.info': False, 'config.get': False, 'config.effort.get': False,
                                 'config.effort.set': True, 'config.limits.get': False,
                                 'config.limits.set': True, 'config.roles.unassign': True})
        host = self.client().host()
        self.addAsyncCleanup(host.close)
        await host.config_get()
        host_flags = {}
        original_host = host._transport.request
        async def record_host(method, params, *, mutation, **kwargs):
            host_flags[(method, params.get('dry_run', False))] = mutation
            return await original_host(method, params, mutation=mutation, **kwargs)
        host._transport.request = record_host
        await host.config_get()
        await host.config_providers_delete('prov-local', dry_run=True)
        await host.config_models_delete('model-orphan', dry_run=True)
        await host.config_models_delete('model-orphan')
        await host.config_roles_unassign('awareness')
        await host.config_subagent_risky_set('classifier')
        self.assertEqual(host_flags, {('config.get', False): False, ('config.providers.delete', True): False,
                                      ('config.models.delete', True): False,
                                      ('config.models.delete', False): True,
                                      ('config.roles.unassign', False): True,
                                      ('config.subagent_risky.set', False): True})


class OpenEnums(unittest.TestCase):
    def route(self, **overrides):
        route = dict(role='main', origin='global', model_uuid=None, source_uuid=None, configured_model=None,
                     provider_uuid=None, provider_name=None, effective_model=None, reason=None)
        route.update(overrides)
        return route

    def view(self, **overrides):
        view = dict(revision='absent', file='missing', view='global', providers=[], providers_truncated=False,
                    oauth=[], oauth_truncated=False, models=[], models_truncated=False,
                    roles=[self.route(role=r) for r in ('main', 'awareness', 'safeguard', 'compactor', 'planner')],
                    main_fallback='unconfigured')
        view.update(overrides)
        return view

    def test_unknown_new_values_map_to_other(self):
        roles = [self.route(role='main', origin='future_origin', reason='future_reason')] + \
            [self.route(role=r) for r in ('awareness', 'safeguard', 'compactor', 'planner')]
        view = ConfigView.from_wire(self.view(file='future', view='future', main_fallback='future', roles=roles))
        self.assertIs(view.file, ConfigFile.OTHER)
        self.assertEqual(view.view.value, 'other')
        self.assertIs(view.main_fallback, MainFallback.OTHER)
        self.assertIs(view.route('main').origin, RouteOrigin.OTHER)
        self.assertIs(view.route('main').reason, RouteReason.OTHER)
        self.assertIs(KeyState('rotated'), KeyState.OTHER)
        self.assertIs(PropagationStatus('queued'), PropagationStatus.OTHER)
        _validate_result('config.providers.delete', dict(
            dry_run=False, removed=dict(provider_uuid='p', models_removed=[], main_reset=False),
            config=self.view(), propagation=dict(bound='future', room='reloaded')))

    def test_closed_enums_and_shapes_stay_strict(self):
        with self.assertRaises(ValueError):
            ConfigView.from_wire(self.view(roles=[self.route(role='cyber-red')] * 5))
        with self.assertRaises(ValueError):
            ConfigView.from_wire(self.view(roles=[]))
        with self.assertRaises(ValueError):
            ConfigView.from_wire(self.view(file=1))
        with self.assertRaises(ValueError):
            _validate_result('config.effort.get', dict(state='ready', options='default', selected='default',
                                                       note=None, main_model_uuid=None))
        with self.assertRaises(KeyError):
            _validate_result('sessions.info', {})
        with self.assertRaises(ValueError):
            _validate_result('state', dict(state=STATE, session_info=None))
        _validate_result('state', dict(state=STATE))


class LegacyHello(unittest.IsolatedAsyncioTestCase):
    def client(self):
        c = Client(sys.executable, adapter_args=(str(FAKE), '--scenario', 'legacy_hello'), close_timeout=.1)
        self.addAsyncCleanup(c.close)
        return c

    async def test_older_adapter_gates_on_capabilities(self):
        hello = await self.client().probe()
        self.assertEqual((hello.methods, hello.features, hello.events), ((), (), ()))
        self.assertEqual(dict(hello.limits), {})
        self.assertEqual(hello.request_ids, 65536)
        self.assertTrue(hello.supports('detach'))
        self.assertFalse(hello.supports('config.get'))

    async def test_new_calls_refused_before_sending(self):
        log = tempfile.NamedTemporaryFile(prefix='koma-legacy-', delete=False)
        log.close()
        os.environ['KOMA_FAKE_LOG'] = log.name
        try:
            c = self.client()
            host = c.host()
            with self.assertRaises(IncompatibleAdapter):
                await host.config_get()
            s = await c.create('/fake/work')
            self.assertIsNone(s.session_info)
            for call in (s.info(), s.effort_get(), s.limits_get(), s.config_get(), s.clear_main_pin()):
                with self.assertRaises(IncompatibleAdapter):
                    await call
            await s.get_state()
            self.assertIsNone(s.session_info)
            with self.assertRaises(RemoteError) as ctx:
                await s._request('create', {'cwd': '/x'}, mutation=True)
            self.assertIsNone(ctx.exception.reason)
            self.assertEqual(ctx.exception.allowed, ())
            self.assertIsNone(ctx.exception.binding)
            await host.close()
            await s.detach()
            sent = [line for line in Path(log.name).read_text().splitlines() if line]
            self.assertFalse(any('config.' in line or 'sessions.' in line for line in sent))
        finally:
            os.environ.pop('KOMA_FAKE_LOG', None)
            Path(log.name).unlink(missing_ok=True)


if __name__ == '__main__':
    unittest.main()
