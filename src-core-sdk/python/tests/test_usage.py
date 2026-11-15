from pathlib import Path
import sys
import unittest

from koma_sdk import Client, LedgerState, RemoteError, UsageReport, UsageRoleKind, SessionKind
from koma_sdk.types import UsageGroup

FAKE = Path(__file__).resolve().parents[2] / 'conformance' / 'fake_adapter.py'


class UsageConformance(unittest.IsolatedAsyncioTestCase):
    def client(self, scenario='usage'):
        c = Client(sys.executable, adapter_args=(str(FAKE), '--scenario', scenario), close_timeout=.1)
        self.addAsyncCleanup(c.close)
        return c

    async def test_totals_groups_and_buckets(self):
        async with self.client().host() as host:
            plain = await host.usage_query()
            self.assertIsInstance(plain, UsageReport)
            self.assertIs(plain.ledger, LedgerState.OK)
            self.assertIsNone(plain.groups)
            self.assertIsNone(plain.buckets)
            self.assertGreater(plain.totals.calls, 0)
            self.assertIsInstance(plain.totals.cost, float)
            wide = await host.usage_query(from_at=plain.from_at - 20 * 86400)
            self.assertGreater(wide.totals.calls, plain.totals.calls)

            by_model = await host.usage_query(group_by='model', limit=1)
            self.assertEqual(len(by_model.groups), 1)
            self.assertTrue(by_model.groups_truncated)
            by_role = await host.usage_query(group_by='role')
            main = next(g for g in by_role.groups if g.key == 'main')
            self.assertIs(main.role_kind, UsageRoleKind.MAIN)
            sub = next(g for g in by_role.groups if g.key.startswith('sub:'))
            self.assertEqual(sub.agent, sub.key[len('sub:'):])
            self.assertEqual(sorted(by_role.groups, key=lambda g: -g.cost), list(by_role.groups))
            by_session = await host.usage_query(group_by='session')
            gone = next(g for g in by_session.groups if g.key == 'sess-deleted')
            self.assertTrue(gone.deleted)
            self.assertIsNone(gone.label)
            self.assertIsInstance(gone, UsageGroup)
            self.assertIn(SessionKind.ROOM, {g.kind for g in by_session.groups})

            days = await host.usage_query(bucket='day', tz_offset_minutes=420)
            self.assertTrue(days.buckets)
            starts = [b.start_at for b in days.buckets]
            self.assertEqual(starts, sorted(starts))
            self.assertAlmostEqual(sum(b.cost for b in days.buckets), days.totals.cost, places=6)

    async def test_missing_ledger_is_not_an_error(self):
        async with self.client('normal').host() as host:
            result = await host.usage_query()
            self.assertIs(result.ledger, LedgerState.MISSING)
            self.assertEqual(result.totals.calls, 0)

    async def test_refusals_carry_reasons(self):
        async with self.client().host() as host:
            with self.assertRaises(RemoteError) as ctx:
                await host.usage_query(from_at=1, to_at=2_000_000_000, bucket='hour')
            self.assertEqual((ctx.exception.code, ctx.exception.reason), ('invalid_request', 'range_too_large'))
            with self.assertRaises(RemoteError) as slow:
                await host.usage_query(session_id='sess-slow')
            self.assertEqual((slow.exception.code, slow.exception.reason), ('unavailable', 'query_timeout'))

    async def test_arguments_are_checked_before_sending(self):
        async with self.client().host() as host:
            for kwargs in (dict(from_at=10, to_at=10), dict(from_at=-1), dict(group_by='day'), dict(bucket='month'),
                           dict(tz_offset_minutes=900), dict(tz_offset_minutes=-721), dict(limit=0),
                           dict(workdir='relative/path'), dict(session_id='')):
                with self.assertRaises(ValueError, msg=kwargs):
                    await host.usage_query(**kwargs)
            self.assertIsNone(host.hello)

    async def test_unknown_values_do_not_fail(self):
        report = UsageReport.from_wire(dict(from_at=1, to_at=2, ledger='sharded', totals=dict(
            calls=0, tokens_in=0, tokens_cached=0, tokens_out=0, cost=0), groups=[dict(
                calls=1, tokens_in=1, tokens_cached=0, tokens_out=1, cost=0.5, key='k', label=None,
                role_kind='coordinator', agent=None)], groups_truncated=False))
        self.assertIs(report.ledger, LedgerState.OTHER)
        self.assertIs(report.groups[0].role_kind, UsageRoleKind.OTHER)
        with self.assertRaises(ValueError):
            UsageReport.from_wire(dict(from_at=1, to_at=2, ledger='ok', totals=dict(calls=-1)))
