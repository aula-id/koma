from pathlib import Path
import sys
import unittest

from koma_sdk import Client, DeliveryUnknown, IncompatibleAdapter, RemoteError, SessionKind, SessionPage

FAKE = Path(__file__).resolve().parents[2] / 'conformance' / 'fake_adapter.py'


class SessionsConformance(unittest.IsolatedAsyncioTestCase):
    def client(self, scenario='sessions'):
        c = Client(sys.executable, adapter_args=(str(FAKE), '--scenario', scenario), close_timeout=.1)
        self.addAsyncCleanup(c.close)
        return c

    async def test_list_is_typed_and_pages_by_cursor(self):
        async with self.client().host() as host:
            first = await host.sessions_list(limit=3)
            self.assertIsInstance(first, SessionPage)
            self.assertEqual(len(first.sessions), 3)
            self.assertIsNotNone(first.next_cursor)
            seen = [r.session_id for r in first.sessions]
            page = first
            while page.next_cursor is not None:
                page = await host.sessions_list(limit=3, cursor=page.next_cursor)
                seen.extend(r.session_id for r in page.sessions)
            self.assertEqual(len(seen), len(set(seen)))
            self.assertIn('sess-coding-a', seen)
            rooms = await host.sessions_list(kind='room')
            self.assertTrue(rooms.sessions and all(r.kind is SessionKind.ROOM for r in rooms.sessions))
            self.assertTrue(all(r.working is None for r in rooms.sessions))
            parent = next(r for r in (await host.sessions_list(kind='coding')).sessions if r.session_id == 'sess-coding-a')
            self.assertTrue(parent.live and parent.working and parent.room_live)
            self.assertIsNotNone(parent.room_session_id)
            idle = await host.sessions_list(kind='coding', live=False)
            self.assertTrue(idle.sessions and not any(r.live for r in idle.sessions))
            orphan = next(r for r in (await host.sessions_list()).sessions if r.session_id == 'sess-live-nr')
            self.assertEqual((orphan.name, orphan.workdir, orphan.created_at), (None, None, None))

    async def test_list_errors_carry_a_reason(self):
        async with self.client().host() as host:
            with self.assertRaises(RemoteError) as ctx:
                await host.sessions_list(cursor='not a cursor')
            self.assertEqual((ctx.exception.code, ctx.exception.reason), ('invalid_request', 'bad_cursor'))

    async def test_arguments_are_checked_before_sending(self):
        async with self.client().host() as host:
            for call in (host.sessions_list(kind='other'), host.sessions_list(limit=0), host.sessions_list(limit=501),
                         host.sessions_list(live='yes'), host.sessions_rename('s', '  '),
                         host.sessions_rename('s', 'a\x07b'), host.sessions_rename('s', 'x' * 201),
                         host.sessions_delete(''), host.sessions_stop('x' * 65)):
                with self.assertRaises(ValueError):
                    await call
            self.assertIsNone(host.hello)

    async def test_rename_paths(self):
        async with self.client().host() as host:
            renamed = await host.sessions_rename('sess-coding-b', '  Beta renamed ')
            self.assertEqual(renamed.session.name, 'Beta renamed')
            self.assertFalse(renamed.session.live)
            with self.assertRaises(RemoteError) as live:
                await host.sessions_rename('sess-coding-a', 'x')
            self.assertEqual((live.exception.code, live.exception.reason), ('invalid_state', 'live'))
            self.assertEqual(live.exception.detail, None)
            with self.assertRaises(RemoteError) as busy:
                await host.sessions_rename('sess-busy', 'x')
            self.assertEqual(busy.exception.reason, 'sqlite_busy')
            with self.assertRaises(RemoteError) as missing:
                await host.sessions_rename('nope', 'x')
            self.assertEqual(missing.exception.code, 'not_found')

    async def test_delete_guards_and_unknown_outcome(self):
        async with self.client().host() as host:
            deleted = await host.sessions_delete('sess-coding-b')
            self.assertTrue(deleted.deleted)
            self.assertIs(deleted.kind, SessionKind.CODING)
            self.assertIsNotNone(deleted.orphaned_room_session_id)
            for session_id, reason in (('sess-coding-a', 'live'), ('sess-locked', 'session_locked')):
                with self.assertRaises(RemoteError) as ctx:
                    await host.sessions_delete(session_id)
                self.assertEqual(ctx.exception.reason, reason)
            with self.assertRaises(DeliveryUnknown) as unknown:
                await host.sessions_delete('sess-partial')
            cause = unknown.exception.cause
            self.assertIsInstance(cause, RemoteError)
            self.assertEqual((cause.reason, cause.written), ('partial_delete', True))
            self.assertTrue((await host.sessions_delete('sess-partial')).deleted)

    async def test_stop(self):
        async with self.client().host() as host:
            self.assertEqual((await host.sessions_stop('sess-coding-b')).was_live, False)
            self.assertTrue((await host.sessions_stop('sess-coding-a')).was_live)
            with self.assertRaises(DeliveryUnknown) as ctx:
                await host.sessions_stop('sess-stuck')
            self.assertEqual(ctx.exception.cause.reason, 'still_running')

    async def test_create_name(self):
        c = self.client()
        with self.assertRaises(ValueError):
            await c.create('/fake/work', name=' ')
        async with await c.create('/fake/work', name='  nightly build ') as session:
            self.assertEqual(session.session_info.name, 'nightly build')
            self.assertEqual(session.warnings, ())
        async with await c.create('/fake/work', name='fail-rename') as session:
            self.assertEqual([w.code for w in session.warnings], ['rename_failed'])
            self.assertEqual(session.session_info.name, session.session_id)

    async def test_create_name_needs_the_feature(self):
        c = self.client('legacy_hello')
        async with await c.create('/fake/work') as session:
            self.assertEqual(session.warnings, ())
        with self.assertRaises(IncompatibleAdapter):
            await c.create('/fake/work', name='x')

    async def test_unknown_kind_is_other(self):
        from koma_sdk.types import SessionRow
        row = SessionRow.from_wire(dict(session_id='s', name=None, workdir=None, kind='archive', created_at=None,
                                        updated_at=None, live=False, working=None, locked=False, bound=False,
                                        parent_session_id=None, room_session_id=None, room_live=False))
        self.assertIs(row.kind, SessionKind.OTHER)
        with self.assertRaises((ValueError, KeyError)):
            SessionRow.from_wire(dict(session_id='s'))
