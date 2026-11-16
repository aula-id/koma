import asyncio
from pathlib import Path
import sys
import unittest
import uuid

from koma_sdk import (Applies, Client, CloseReason, DeliveryUnknown, ImageFile, IncompatibleAdapter, LineKind,
                      LineOrigin, NoticeCode, PastedText, PropagationStatus, RemoteError, Room, RoomClosedEvent,
                      RoomDeltaEvent, RoomGapEvent, RoomLine, RoomLineEvent, RoomNoticeEvent, RoomStatusEvent,
                      TeamTarget)
from koma_sdk.room_types import decode_room_event

FAKE = Path(__file__).resolve().parents[2] / 'conformance' / 'fake_adapter.py'
PARENT_A, PARENT_B = 'sess-coding-a', 'sess-coding-b'
ROOM_A = uuid.uuid5(uuid.NAMESPACE_OID, 'secure-room:' + PARENT_A).hex
ROOM_B = uuid.uuid5(uuid.NAMESPACE_OID, 'secure-room:' + PARENT_B).hex


def epoch(n):
    return str(uuid.uuid5(uuid.NAMESPACE_OID, f'epoch-{n}'))


class RoomConformance(unittest.IsolatedAsyncioTestCase):
    def client(self, scenario='room', *extra, **kwargs):
        c = Client(sys.executable, adapter_args=(str(FAKE), '--scenario', scenario, *extra), close_timeout=.1, **kwargs)
        self.addAsyncCleanup(c.close)
        return c

    async def take(self, room, count):
        out = []
        async def run():
            async for event in room.events():
                out.append(event)
                if len(out) == count:
                    return
        await asyncio.wait_for(run(), 5)
        return out

    async def test_open_attach_result_and_events_in_order(self):
        room = await self.client().room_open(PARENT_A, tail=5)
        self.assertIsInstance(room, Room)
        a = room.attached
        self.assertEqual((a.room.session_id, a.epoch, a.started), (ROOM_A, epoch(1), False))
        self.assertEqual((room.session_id, room.epoch), (ROOM_A, epoch(1)))
        self.assertEqual(a.history.count, 10)
        self.assertEqual([l.line_seq for l in a.tail], [6, 7, 8, 9, 10])
        self.assertIs(a.tail[0].kind, LineKind.TOOL_CALL)
        self.assertEqual((a.tail[0].call_id, a.tail[0].tool), ('call-1', 'http_get'))
        self.assertEqual((a.tail[1].call_id, a.tail[1].ok), ('call-1', True))
        self.assertIn('sdk_user', a.daemon.features)
        self.assertIn('scout', a.roster)
        self.assertTrue(a.status.live)
        events = await self.take(room, 6)
        self.assertEqual([type(e).__name__ for e in events],
                         ['RoomStatusEvent', 'RoomDeltaEvent', 'RoomDeltaEvent', 'RoomLineEvent', 'RoomNoticeEvent',
                          'RoomNoticeEvent'])
        self.assertTrue(all(e.epoch == epoch(1) for e in events))
        self.assertEqual(events[3].line.line_seq, 11)
        self.assertFalse(events[3].recovered)
        self.assertEqual([e.code for e in events[4:]], [NoticeCode.STALLED, NoticeCode.RECOVERED])
        self.assertEqual(events[1].text + events[2].text, 'Looking at the admin panel')
        self.assertTrue(room.live)
        self.assertEqual(room.requests_remaining, 65536 - 1)
        await room.aclose()

    async def test_attach_by_id_and_not_running(self):
        c = self.client()
        with self.assertRaises(RemoteError) as ctx:
            await c.room_attach(ROOM_B)
        self.assertEqual((ctx.exception.code, ctx.exception.reason), ('invalid_state', 'not_running'))
        async with await c.room_attach(ROOM_A) as room:
            self.assertEqual(room.attached.room.parent_session_id, PARENT_A)
        with self.assertRaises(ValueError):
            await c.room_attach(ROOM_A, tail=201)
        with self.assertRaises(ValueError):
            await c.room_attach(ROOM_A, wait_ms=15001)
        with self.assertRaises(ValueError):
            await c.room_open('')

    async def test_open_starts_a_room_that_is_not_live(self):
        async with await self.client().room_open(PARENT_B) as room:
            self.assertTrue(room.attached.started)
            self.assertEqual(room.attached.daemon.features, ())
            self.assertIsNone(room.attached.daemon.template_revision)
            with self.assertRaises(RemoteError) as ctx:
                await room.send('look', attachments=[PastedText('x')])
            self.assertEqual(ctx.exception.reason, 'peer_unsupported')
            sent = await room.send('look at /admin')
            self.assertFalse(sent.acked)
            self.assertEqual(sent.inserts, ())
        c = self.client()
        for parent, reason in (('sess-nostart', 'room_start_failed'), ('sess-coding-c', 'no_main_route'),
                               ('sess-clash', 'room_id_in_use'), ('missing', 'not_found')):
            with self.assertRaises((RemoteError, DeliveryUnknown)) as ctx:
                await c.room_open(parent)
            error = ctx.exception.cause if isinstance(ctx.exception, DeliveryUnknown) else ctx.exception
            self.assertEqual(error.reason, reason)

    async def test_send_variants(self):
        async with await self.client().room_open(PARENT_A) as room:
            plain = await room.send('hello')
            self.assertTrue(plain.acked)
            self.assertEqual(plain.inserts, ())
            targeted = await room.send('go on', to='scout')
            shell = await room.send('ls /srv', shell=True)
            files = await room.send('see', attachments=[ImageFile('/tmp/shot.png'), PastedText('log', 'build')])
            self.assertEqual(files.inserts, ('[Image #1]', '[Pasted text #2]'))
            self.assertEqual(targeted.echo_line_seq + 1, shell.echo_line_seq)
            events = await self.take(room, 6 + 4)
            echoes = [e for e in events if isinstance(e, RoomLineEvent) and e.line.origin is LineOrigin.ADAPTER_ECHO]
            self.assertEqual([e.line.text for e in echoes], ['hello', '@scout go on', '!ls /srv', 'see'])
            self.assertIs(echoes[2].line.kind, LineKind.SHELL)
            with self.assertRaises(RemoteError) as clerk:
                await room.send('x', to='clerk')
            self.assertEqual(clerk.exception.reason, 'clerk_not_addressable')
            with self.assertRaises(RemoteError) as nobody:
                await room.send('x', to='nobody')
            self.assertEqual((nobody.exception.code, nobody.exception.reason), ('not_found', 'unknown_avatar'))
            with self.assertRaises(RemoteError) as refused:
                await room.send('x', attachments=[ImageFile('/tmp/refused.png')])
            self.assertEqual(refused.exception.reason, 'attachment_refused')
            for call in (room.send(' '), room.send('x', shell=True, to='scout'),
                         room.send('x', shell=True, attachments=[PastedText('t')]), room.send('x', shell='yes'),
                         room.send('x', attachments=[PastedText('t')] * 17), room.send('x' * (2 * 1024 * 1024 + 1))):
                with self.assertRaises(ValueError):
                    await call

    async def test_unknown_send_outcome_is_never_retried_and_closes_the_binding(self):
        async with await self.client().room_open(PARENT_A) as room:
            with self.assertRaises(DeliveryUnknown) as ctx:
                await room.send('fake:stream_error')
            self.assertEqual(ctx.exception.cause.reason, 'stream_error')
            self.assertEqual(ctx.exception.session_id, ROOM_A)
            events = []
            async for event in room.events():
                events.append(event)
            self.assertIsInstance(events[-1], RoomClosedEvent)
            self.assertIs(events[-1].reason, CloseReason.STREAM_ERROR)
            self.assertIs(room.closed_reason, CloseReason.STREAM_ERROR)
            self.assertFalse(room.live)
            with self.assertRaises(RemoteError) as closed:
                await room.send('again')
            self.assertEqual(closed.exception.reason, 'room_closed')
            self.assertEqual((await room.status()).live, False)

    async def test_interrupt_and_control(self):
        async with await self.client().room_open(PARENT_A) as room:
            self.assertTrue((await room.interrupt()).sent)
            self.assertTrue((await room.control('pause', 'scout')).sent)
            events = await self.take(room, 6 + 1)
            self.assertIsInstance(events[-1], RoomStatusEvent)
            self.assertEqual(events[-1].status.paused, ('scout',))
            status = await room.status()
            self.assertEqual(status.paused, ('scout',))
            self.assertEqual(next(l for l in status.lanes if l.avatar == 'scout' and l.lane.value == 'work').phase.value,
                             'paused')
            await room.control('redirect', 'scout', note='check /admin first')
            await room.control('resume', 'scout')
            for call in (room.control('wave', 'scout'), room.control('redirect', 'scout'),
                         room.control('pause', 'scout', note='x'), room.control('pause', ''),
                         room.control('redirect', 'scout', note='')):
                with self.assertRaises(ValueError):
                    await call
            with self.assertRaises(RemoteError) as chief:
                await room.control('pause', 'chief')
            self.assertEqual(chief.exception.reason, 'no_work_lane')
            with self.assertRaises(RemoteError) as nobody:
                await room.control('pause', 'nobody')
            self.assertEqual(nobody.exception.reason, 'unknown_avatar')

    async def test_history_paging(self):
        async with await self.client().room_open(PARENT_A) as room:
            newest = await room.history(limit=3)
            self.assertIn(newest.lines[-1].line_seq, (10, 11))
            self.assertTrue(newest.has_more_before)
            self.assertFalse(newest.has_more_after)
            self.assertEqual(newest.first_line_seq, 1)
            below = await room.history(before_line_seq=11, limit=3)
            self.assertEqual([l.line_seq for l in below.lines], [8, 9, 10])
            older = await room.history(before_line_seq=8, limit=3)
            self.assertEqual([l.line_seq for l in older.lines], [5, 6, 7])
            later = await room.history(after_line_seq=7, limit=2)
            self.assertEqual([l.line_seq for l in later.lines], [8, 9])
            self.assertTrue(later.has_more_after)
            chiefs = await room.history(kinds=['chief'])
            self.assertEqual({l.kind for l in chiefs.lines}, {LineKind.CHIEF})
            scout = await room.history(author='scout')
            self.assertTrue(scout.lines and all(l.author == 'scout' for l in scout.lines))
            with self.assertRaises(RemoteError) as ctx:
                await room.history(epoch='an-older-epoch')
            self.assertEqual((ctx.exception.code, ctx.exception.reason), ('invalid_state', 'epoch_changed'))
            for kwargs in (dict(after_line_seq=1, before_line_seq=5), dict(limit=0), dict(limit=201),
                           dict(after_line_seq=-1), dict(kinds='chief')):
                with self.assertRaises(ValueError, msg=kwargs):
                    await room.history(**kwargs)

    async def test_gap_is_recovered_by_history_not_state(self):
        async with await self.client('room_gap').room_open(PARENT_A) as room:
            events = await self.take(room, 1 + 1 + 1 + 3 + 1)
            kinds = [type(e).__name__ for e in events]
            self.assertEqual(kinds, ['RoomStatusEvent', 'RoomLineEvent', 'RoomGapEvent', 'RoomLineEvent',
                                     'RoomLineEvent', 'RoomLineEvent', 'RoomStatusEvent'])
            self.assertEqual([e.line.line_seq for e in events if isinstance(e, RoomLineEvent)], [11, 12, 13, 14])
            self.assertEqual([e.recovered for e in events if isinstance(e, RoomLineEvent)], [False, True, True, True])
            gap = events[2]
            self.assertEqual((gap.after_line_seq, gap.dropped_lines, gap.dropped_deltas), (11, 3, 2))

    async def test_overflow_is_recovered_by_history(self):
        async with await self.client(event_capacity=1).room_open(PARENT_A) as room:
            await asyncio.sleep(.5)
            events = await self.take(room, 2)
            self.assertIsInstance(events[0], RoomLineEvent)
            self.assertEqual((events[0].line.line_seq, events[0].recovered), (11, True))
            self.assertIsInstance(events[1], RoomStatusEvent)

    async def test_takeover_is_reported_and_never_undone_automatically(self):
        async with await self.client('room_takeover').room_open(PARENT_A) as room:
            events = [e async for e in room.events()]
            self.assertEqual([type(e).__name__ for e in events], ['RoomStatusEvent', 'RoomClosedEvent'])
            self.assertIs(events[-1].reason, CloseReason.TAKEN_OVER)
            self.assertIs(room.closed_reason, CloseReason.TAKEN_OVER)
            self.assertEqual(room.epoch, epoch(1))
            await asyncio.sleep(.2)
            self.assertEqual(room.epoch, epoch(1))
            self.assertFalse(room.live)
            self.assertEqual((await room.history(limit=2)).epoch, epoch(1))
            self.assertFalse((await room.status()).live)
            self.assertEqual((await room.shelf_list()).total, 8)
            with self.assertRaises(RemoteError) as ctx:
                await room.interrupt()
            self.assertEqual((ctx.exception.code, ctx.exception.reason), ('invalid_state', 'room_closed'))
            back = await room.reattach()
            self.assertEqual(back.epoch, epoch(2))
            self.assertEqual(room.epoch, epoch(2))
            self.assertTrue(room.live)
            self.assertIsNone(room.closed_reason)
            self.assertTrue((await room.interrupt()).sent)
            await room.send('back again')
            nxt = await self.take(room, 1)
            self.assertIsInstance(nxt[0], RoomLineEvent)
            self.assertEqual((nxt[0].epoch, nxt[0].line.text), (epoch(2), 'back again'))

    async def test_only_one_events_consumer(self):
        async with await self.client().room_open(PARENT_A) as room:
            first = room.events()
            await first.__anext__()
            with self.assertRaises(RuntimeError):
                await room.events().__anext__()
            await first.aclose()

    async def test_disk_reads(self):
        async with await self.client().room_open(PARENT_A) as room:
            shelf = await room.shelf_list(prefix='findings/')
            self.assertEqual([r.key for r in shelf.rows], ['findings/f-001', 'findings/f-002', 'findings/f-003'])
            self.assertEqual(shelf.total, 3)
            paged = await room.shelf_list(limit=2)
            self.assertIsNotNone(paged.next_cursor)
            self.assertEqual(paged.fingerprint, shelf.fingerprint)
            item = await room.item('reports/r-001')
            self.assertEqual((item.kind, item.json['audience']), ('report', 'client'))
            with self.assertRaises(RemoteError) as missing:
                await room.item('findings/none')
            self.assertEqual(missing.exception.code, 'not_found')
            with self.assertRaises(RemoteError) as grammar:
                await room.item('/bad/')
            self.assertEqual(grammar.exception.reason, 'invalid_name')
            trajectory = await room.trajectory()
            goal = trajectory.goals[0]
            self.assertEqual((goal.goal_key, len(goal.nodes), len(goal.edges)), ('trajectory/web-recon', 3, 2))
            self.assertEqual((goal.edges[0].source, goal.edges[0].target, goal.edges[1].rel), ('n1', 'n2', None))
            self.assertEqual((await room.trajectory('trajectory/web-recon')).goals[0].goal_key, goal.goal_key)
            self.assertEqual((await room.trajectory('other')).goals, ())
            reports = await room.reports()
            self.assertEqual([r.key for r in reports.reports], ['reports/r-001', 'reports/r-002'])
            self.assertIsNone(reports.reports[0].markdown)
            self.assertTrue((await room.reports(include_markdown=True)).reports[0].markdown.startswith('# '))
            spend = await room.spend()
            self.assertEqual(spend.goals[0].total.calls, 6)
            self.assertEqual({a.avatar for a in spend.ledger.by_avatar}, {'chief', 'scout', 'analyst'})
            for call in (room.shelf_list(prefix='x' * 201), room.shelf_list(limit=501), room.item(''),
                         room.reports(limit=101), room.reports(include_markdown='yes')):
                with self.assertRaises(ValueError):
                    await call

    async def test_team_reads_and_writes_on_a_live_room(self):
        async with await self.client().room_open(PARENT_A) as room:
            team = await room.team_get()
            self.assertIs(team.target, TeamTarget.ROOM)
            ids = {a.id for a in team.avatars}
            self.assertTrue({'chief', 'clerk', 'scout', 'recon', 'probe'} <= ids)
            self.assertEqual(next(a for a in team.avatars if a.id == 'probe').defined_in.value, 'session')
            self.assertEqual(team.choices_source.value, 'room_daemon')
            self.assertFalse(team.template_pending_restart)
            written = await room.models_set('scout', 'chat', model='model-fast', effort='high',
                                            if_revision=team.revision)
            self.assertIs(written.applies, Applies.LIVE)
            self.assertIs(written.propagation.room, PropagationStatus.RELOADED)
            lane = next(l for a in written.team.avatars if a.id == 'scout' for l in a.lanes if l.lane.value == 'chat')
            self.assertEqual((lane.model.value, lane.effort.value, lane.model.origin.value), ('model-fast', 'high', 'session'))
            with self.assertRaises(RemoteError) as stale:
                await room.models_set('scout', 'chat', effort='low', if_revision=team.revision)
            self.assertEqual((stale.exception.code, stale.exception.reason), ('invalid_state', 'stale'))
            cleared = await room.models_set('scout', 'chat', model=None, effort=None)
            lane = next(l for a in cleared.team.avatars if a.id == 'scout' for l in a.lanes if l.lane.value == 'chat')
            self.assertEqual(lane.effort.origin.value in ('role', 'default', 'base'), True)
            with self.assertRaises(RemoteError) as model:
                await room.models_set('scout', 'chat', model='model-none')
            self.assertEqual((model.exception.code, model.exception.reason), ('not_found', 'unknown_model'))
            with self.assertRaises(RemoteError) as rejected:
                await room.models_set('scout', 'chat', effort='room-rejects')
            self.assertEqual(rejected.exception.reason, 'rejected_by_room')
            self.assertEqual(rejected.exception.detail['warnings'][0]['layer'], 'session')
            with self.assertRaises(RemoteError) as lane:
                await room.models_set('chief', 'work', effort='high')
            self.assertEqual(lane.exception.reason, 'no_such_lane')
            added = await room.avatars_upsert('mapper', role='scout', display='Mapper', alias='mp')
            self.assertEqual(next(a for a in added.team.avatars if a.id == 'mapper').alias_effective, 'MP')
            with self.assertRaises(RemoteError) as dup:
                await room.avatars_upsert('mapper2', role='scout', alias='MP')
            self.assertEqual(dup.exception.reason, 'layer_rejected')
            with self.assertRaises(RemoteError) as immutable:
                await room.avatars_upsert('mapper', role='analyst')
            self.assertEqual(immutable.exception.reason, 'role_immutable')
            removed = await room.avatars_remove('mapper')
            self.assertIsNone(removed.reverted_to)
            with self.assertRaises(RemoteError) as running:
                await room.avatars_remove('probe')
            self.assertEqual(running.exception.reason, 'lane_running')
            for call in (room.models_set('scout', 'chat'), room.models_set('scout', 'both', model='m'),
                         room.models_set('scout', 'chat', effort='has space'), room.models_set('scout', 'chat', effort='x', target='world'),
                         room.avatars_upsert('Bad Id', role='scout'), room.avatars_upsert('ok'),
                         room.avatars_upsert('ok', role='scout', alias='toolong'), room.avatars_remove('x', target='sky')):
                with self.assertRaises(ValueError):
                    await call

    async def test_older_room_and_template_writes_apply_later(self):
        async with await self.client().room_open(PARENT_B) as room:
            written = await room.models_set('scout', 'chat', effort='low')
            self.assertIs(written.applies, Applies.NEXT_RELOAD)
            self.assertIs(written.propagation.room, PropagationStatus.NOT_SUPPORTED)
            template = await room.avatars_upsert('tpl', role='scout', target='template')
            self.assertIs(template.applies, Applies.NEXT_START)
            self.assertIs(template.team.target, TeamTarget.TEMPLATE)
            self.assertIs((await room.team_get('template')).target, TeamTarget.TEMPLATE)

    async def test_host_template_team(self):
        async with self.client('room').host() as host:
            team = await host.room_team_get()
            self.assertIs(team.target, TeamTarget.TEMPLATE)
            self.assertIn('recon', {a.id for a in team.avatars})
            wrote = await host.room_avatars_upsert('tester', role='analyst', display='Tester',
                                                   if_revision=team.revision)
            self.assertIs(wrote.applies, Applies.NEXT_START)
            self.assertIs(wrote.propagation.room, PropagationStatus.NOT_LIVE)
            set_ = await host.room_models_set('tester', 'chat', model='model-bare')
            lane = next(l for a in set_.team.avatars if a.id == 'tester' for l in a.lanes if l.lane.value == 'chat')
            self.assertEqual(lane.model.value, 'bare/model')
            gone = await host.room_avatars_remove('tester')
            self.assertIsNone(gone.reverted_to)
            with self.assertRaises(RemoteError) as stale:
                await host.room_avatars_remove('recon', if_revision='0' * 16)
            self.assertEqual(stale.exception.reason, 'stale')
            with self.assertRaises(RemoteError) as wrong:
                await host._call('room.team.get', {'target': 'room'}, mutation=False)
            self.assertEqual((wrong.exception.reason, wrong.exception.binding), ('wrong_binding', 'unbound'))
            self.assertIn('room_live', wrong.exception.allowed)

    async def test_room_list(self):
        async with self.client().host() as host:
            rooms = await host.room_list()
            self.assertEqual({r.session_id for r in rooms.rooms} >= {ROOM_A, ROOM_B}, True)
            live = await host.room_list(live=True)
            self.assertEqual([r.session_id for r in live.rooms], [ROOM_A])
            self.assertEqual(live.rooms[0].parent_session_id, PARENT_A)
            one = await host.room_list(parent_session_id=PARENT_B)
            self.assertEqual([r.session_id for r in one.rooms], [ROOM_B])
            with self.assertRaises(ValueError):
                await host.room_list(live='yes')

    async def test_close(self):
        c = self.client()
        room = await c.room_open(PARENT_A)
        closed = await room.close(stop=True)
        self.assertEqual((closed.closed, closed.stopped), (True, True))
        self.assertFalse(room.live)
        room = await c.room_open(PARENT_A, wait_ms=0)
        self.assertFalse(room.attached.status.fresh)
        self.assertEqual((await room.close()).stopped, False)

    async def test_room_calls_are_refused_while_unbound(self):
        async with self.client().host() as host:
            with self.assertRaises(RemoteError) as ctx:
                await host._call('room.send', {'text': 'x'}, mutation=True)
            self.assertEqual((ctx.exception.code, ctx.exception.reason), ('invalid_state', 'wrong_binding'))

    async def test_unknown_values_and_events_do_not_fail(self):
        line = dict(line_seq=3, kind='telemetry', author='scout', display=None, alias=None, lane='side',
                    audience='nobody', text='{"x":1}', call_id=None, tool=None, args=None, ok=None, result=None,
                    replay=False, origin='bridge', received_at_ms=1, truncated=False)
        parsed = RoomLine.from_wire(line)
        self.assertIs(parsed.kind, LineKind.OTHER)
        self.assertEqual(parsed.lane.value, 'other')
        self.assertIs(parsed.origin, LineOrigin.OTHER)
        self.assertIsNone(decode_room_event(dict(event='room.future', seq=1, epoch='e')))
        closed = decode_room_event(dict(event='room.closed', seq=1, epoch='e', session_id='s', reason='shutdown'))
        self.assertIs(closed.reason, CloseReason.OTHER)
        with self.assertRaises((ValueError, KeyError)):
            RoomLine.from_wire(dict(line, line_seq=0))

    async def test_old_adapter_without_room_methods(self):
        with self.assertRaises(IncompatibleAdapter):
            await self.client('legacy_hello').room_open(PARENT_A)
