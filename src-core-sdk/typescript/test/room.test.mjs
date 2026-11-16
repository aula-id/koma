import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DeliveryUnknownError, SdkError } from '../dist/index.js';
import { client, collect } from './support.mjs';

const kinds = events => events.map(e => e.event + (e.recovered ? '*' : ''));

test('room open and attach bind a room with its attach result', async () => {
  const c = client('room');
  try {
    const host = c.host();
    const rooms = await host.listRooms({ live: true });
    assert.ok(rooms.rooms.length >= 1 && rooms.rooms.every(r => r.live));
    const room = await c.roomOpen({ parent_session_id: 'sess-coding-a' });
    assert.equal(room.attachResult.started, false); assert.equal(room.attachResult.room.session_id, room.sessionId);
    assert.match(room.epoch, /^[0-9a-f-]{36}$/); assert.equal(room.attachResult.tail.length, 10);
    assert.deepEqual(room.attachResult.history, { first_line_seq: 1, last_line_seq: 10, count: 10, replayed: 10 });
    assert.ok(room.attachResult.roster.includes('scout')); assert.ok(room.requestsRemaining > 0);
    await assert.rejects(c.roomOpen({ parent_session_id: 'sess-coding-c' }), e => e instanceof SdkError && e.reason === 'no_main_route');
    await assert.rejects(c.roomOpen({ parent_session_id: 'sess-nostart' }), e => e instanceof DeliveryUnknownError && e.reason === 'room_start_failed' && e.delivery === 'unknown' && e.detail.log_tail !== undefined);
    await assert.rejects(c.roomAttach({ session_id: 'sess-coding-b' }), e => e.reason === 'not_found' || e.reason === 'not_running');
    const started = await c.roomOpen({ parent_session_id: 'sess-coding-b' });
    assert.equal(started.attachResult.started, true); assert.equal(started.attachResult.daemon.template_revision, null);
    await started.close();
    await room.close();
  } finally { await c.close(); }
});

test('room events arrive in order for the current epoch only', async () => {
  const c = client('room');
  try {
    const room = await c.roomOpen({ parent_session_id: 'sess-coding-a' });
    const events = await collect(room, 6);
    assert.deepEqual(kinds(events), ['room.status', 'room.delta', 'room.delta', 'room.line', 'room.notice', 'room.notice']);
    assert.ok(events.every(e => e.epoch === room.epoch));
    assert.ok(events.every((e, i) => i === 0 || e.seq > events[i - 1].seq));
    assert.equal(events[3].line.line_seq, 11); assert.equal(events[3].line.replay, false);
    assert.deepEqual(events.slice(4).map(e => e.code), ['stalled', 'recovered']);
    assert.equal(events[1].author, 'scout'); assert.equal(events[1].kind, 'text');
    const echo = await room.send({ text: 'hello', to: 'scout' });
    assert.equal(echo.acked, true);
    const [line] = await collect(room, 1);
    assert.equal(line.line.origin, 'adapter_echo'); assert.equal(line.line.text, '@scout hello'); assert.equal(line.line.line_seq, echo.echo_line_seq);
    // Events of another epoch are ignored.
    room.transport.frame({ type: 'event', event: 'room.notice', seq: room.transport.seq + 1, epoch: 'other-epoch', code: 'room', text: 'old' }, 100);
    room.transport.frame({ type: 'event', event: 'room.notice', seq: room.transport.seq + 1, epoch: room.epoch, code: 'room', text: 'new' }, 100);
    assert.deepEqual((await collect(room, 1)).map(e => e.text), ['new']);
  } finally { await c.close(); }
});

test('room.send forms: shell, attachments, old room, refusals, closed binding', async () => {
  const c = client('room');
  try {
    const room = await c.roomOpen({ parent_session_id: 'sess-coding-a' });
    const shell = await room.send({ text: 'ls', shell: true });
    assert.equal(shell.sent, true);
    const files = await room.send({ text: 'look', attachments: [{ kind: 'pasted_text', text: 'body' }, { kind: 'image_file', path: '/fake/a.png' }] });
    assert.equal(files.inserts.length, 2);
    await assert.rejects(room.send({ text: 'x', to: 'clerk' }), e => e.reason === 'clerk_not_addressable');
    await assert.rejects(room.send({ text: 'x', to: 'ghost' }), e => e.reason === 'unknown_avatar');
    await assert.rejects(room.send({ text: 'x', shell: true, to: 'scout' }), e => e.reason === 'conflicting_params');
    await assert.rejects(room.send({ text: 'x', attachments: [{ kind: 'image_file', path: '/fake/refused.png' }] }), e => e.reason === 'attachment_refused' && e.delivery === 'not_sent');
    await assert.rejects(room.send({ text: '   ' }), e => e.code === 'invalid_input');
    await assert.rejects(room.send({ text: 'x', attachments: [{ kind: 'image_file', path: 'relative.png' }] }), e => e.code === 'invalid_input');
    await assert.rejects(room.send({ text: 'x', to: '' }), e => e.code === 'invalid_input');
    await assert.rejects(room.send({ text: 'fake:stream_error' }), e => e instanceof DeliveryUnknownError && e.reason === 'stream_error');
    const closed = (await collect(room, 20, 800)).find(e => e.event === 'room.closed');
    assert.equal(closed.reason, 'stream_error');
    await assert.rejects(room.send({ text: 'again' }), e => e.reason === 'room_closed' && e.binding === 'room_closed');
    assert.equal((await room.status()).live, false);
    const before = room.epoch;
    const reopened = await room.reopen();
    assert.equal(reopened.room.session_id, room.sessionId); assert.notEqual(room.epoch, before);
    assert.equal((await room.status()).live, true);
    await room.close();
    const old = await c.roomOpen({ parent_session_id: 'sess-coding-b' });
    assert.equal((await old.send({ text: 'plain' })).acked, false);
    await assert.rejects(old.send({ text: 'x', attachments: [{ kind: 'pasted_text', text: 'p' }] }), e => e.reason === 'peer_unsupported');
  } finally { await c.close(); }
});

test('room interrupt, control and status events', async () => {
  const c = client('room');
  try {
    const room = await c.roomOpen({ parent_session_id: 'sess-coding-a' });
    assert.deepEqual(await room.interrupt(), { sent: true });
    await assert.rejects(room.control({ action: 'redirect', avatar: 'scout' }), e => e.reason === 'note_required');
    await assert.rejects(room.control({ action: 'pause', avatar: 'chief' }), e => e.reason === 'no_work_lane');
    assert.deepEqual(await room.control({ action: 'pause', avatar: 'scout' }), { sent: true });
    const status = await room.status();
    assert.ok(status.paused.includes('scout')); assert.equal(status.counts_basis, 'first_400_items');
    const events = await collect(room, 20, 800);
    assert.ok(events.some(e => e.event === 'room.status' && e.status.paused.includes('scout')));
  } finally { await c.close(); }
});

test('room history pages by line_seq and refuses another epoch', async () => {
  const c = client('room');
  try {
    const room = await c.roomOpen({ parent_session_id: 'sess-coding-a' });
    const newest = await room.history({ limit: 4 });
    assert.deepEqual(newest.lines.map(l => l.line_seq), [8, 9, 10, 11]); assert.equal(newest.has_more_before, true); assert.equal(newest.has_more_after, false);
    const older = await room.history({ before_line_seq: 7, limit: 100 });
    assert.deepEqual(older.lines.map(l => l.line_seq), [1, 2, 3, 4, 5, 6]); assert.equal(older.has_more_before, false);
    const after = await room.history({ after_line_seq: 8, kinds: ['chief'] });
    assert.ok(after.lines.every(l => l.kind === 'chief' && l.line_seq > 8));
    const call = (await room.history()).lines.find(l => l.kind === 'tool_call');
    const result = (await room.history()).lines.find(l => l.kind === 'tool_result');
    assert.equal(call.call_id, result.call_id);
    await assert.rejects(room.history({ after_line_seq: 1, before_line_seq: 5 }), e => e.reason === 'conflicting_params');
    await assert.rejects(room.history({ epoch: 'not-this-one' }), e => e.reason === 'epoch_changed');
  } finally { await c.close(); }
});

test('room reads from disk: shelf, item, trajectory, reports, spend', async () => {
  const c = client('room');
  try {
    const room = await c.roomOpen({ parent_session_id: 'sess-coding-a' });
    const shelf = await room.shelf();
    assert.ok(shelf.rows.length > 0 && shelf.total >= shelf.rows.length && shelf.fingerprint);
    const first = await room.shelf({ limit: 1 });
    assert.equal(first.rows.length, 1); assert.ok(first.next_cursor);
    const item = await room.item(shelf.rows[0].key);
    assert.equal(item.key, shelf.rows[0].key); assert.equal(typeof item.content, 'string');
    await assert.rejects(room.item('nothing/here'), e => e.code === 'not_found');
    const trajectory = await room.trajectory();
    assert.equal(trajectory.goals[0].goal_key, 'trajectory/web-recon'); assert.ok(trajectory.goals[0].nodes.length > 0);
    assert.equal((await room.trajectory({ goal: 'web-recon' })).goals.length, 1);
    const reports = await room.reports();
    assert.equal(reports.reports.length, 2); assert.equal(reports.reports[0].markdown, null);
    assert.equal(typeof (await room.reports({ include_markdown: true, limit: 1 })).reports[0].markdown, 'string');
    const spend = await room.spend();
    assert.equal(spend.ledger.ledger, 'ok'); assert.ok(spend.ledger.by_avatar.length > 0);
    assert.ok(spend.goals.every(g => g.goal_key));
  } finally { await c.close(); }
});

test('room team: room target by default, template through the host', async () => {
  const c = client('room');
  try {
    const room = await c.roomOpen({ parent_session_id: 'sess-coding-a' });
    const team = await room.team();
    assert.equal(team.target, 'room'); assert.equal(team.choices_source, 'room_daemon');
    assert.ok(team.avatars.some(a => a.id === 'probe' && a.defined_in === 'session'));
    const model = await room.setModel({ avatar: 'scout', lane: 'chat', model: 'model-fast' });
    assert.equal(model.applies, 'live'); assert.equal(model.propagation.room, 'reloaded');
    const lane = model.team.avatars.find(a => a.id === 'scout').lanes.find(l => l.lane === 'chat');
    assert.equal(lane.model.value, 'model-fast'); assert.equal(lane.model.origin, 'session');
    await assert.rejects(room.setModel({ avatar: 'scout', lane: 'chat', effort: 'room-rejects' }), e => e.reason === 'rejected_by_room' && e.delivery === 'not_sent');
    await assert.rejects(room.setModel({ avatar: 'scout', lane: 'chat' }), e => e.reason === 'missing_params');
    await assert.rejects(room.setModel({ avatar: 'scout', lane: 'chat', model: 'nope' }), e => e.reason === 'unknown_model');
    await assert.rejects(room.setModel({ avatar: 'scout', lane: 'chat', model: 'model-fast', if_revision: '0000000000000000' }), e => e.reason === 'stale');
    const added = await room.upsertAvatar({ id: 'newbie', role: 'scout-role-missing' }).catch(e => e);
    assert.ok(added instanceof SdkError);
    await assert.rejects(room.removeAvatar({ id: 'probe' }), e => e.reason === 'lane_running');
    const cleared = await room.setModel({ avatar: 'scout', lane: 'chat', model: null });
    assert.equal(cleared.team.avatars.find(a => a.id === 'scout').lanes.find(l => l.lane === 'chat').model.origin !== 'session', true);
    // The template layer is reachable without a room and applies at the next start.
    const host = c.host();
    const tpl = await host.getTeam();
    assert.equal(tpl.target, 'template');
    const written = await host.upsertTeamAvatar({ id: 'tplbot', role: tpl.avatars.find(a => a.id === 'scout').role, display: 'Tpl Bot', alias: 'tb' });
    assert.equal(written.applies, 'next_start'); assert.ok(written.team.avatars.some(a => a.id === 'tplbot'));
    await assert.rejects(host.upsertTeamAvatar({ id: 'Bad Id', role: 'x' }), e => e.code === 'invalid_request' || e.code === 'invalid_input');
    const model2 = await host.setTeamModel({ avatar: 'tplbot', lane: 'work', model: 'model-deep' });
    assert.equal(model2.applies, 'next_start');
    const removed = await host.removeTeamAvatar({ id: 'tplbot' });
    assert.equal(removed.reverted_to, null);
  } finally { await c.close(); }
});

test('room.gap is recovered by room.history, then status', async () => {
  const c = client('room_gap');
  try {
    const room = await c.roomOpen({ parent_session_id: 'sess-coding-a' });
    const events = await collect(room, 7);
    assert.deepEqual(kinds(events), ['room.status', 'room.line', 'room.gap', 'room.line*', 'room.line*', 'room.line*', 'room.status*']);
    assert.deepEqual(events.filter(e => e.event === 'room.line').map(e => e.line.line_seq), [11, 12, 13, 14]);
    assert.equal(events[2].dropped_lines, 3);
  } finally { await c.close(); }
});

test('client overflow is recovered by room.history without duplicates', async () => {
  const c = client('room');
  try {
    const room = await c.roomOpen({ parent_session_id: 'sess-coding-a' });
    await room.status(); // the post-attach events are queued by now
    const t = room.transport, base = t.seq;
    const frame = (text, n) => ({ type: 'event', event: 'room.notice', seq: base + n, epoch: room.epoch, code: 'room', text });
    t.frame(frame('a', 1), 33 * 1024 * 1024); t.frame(frame('b', 2), 33 * 1024 * 1024);
    assert.equal(t.queue.length, 0);
    t.seq = base; // the adapter's own numbering goes on from where it was
    const events = await collect(room, 2);
    assert.deepEqual(kinds(events), ['room.line*', 'room.status*']);
    assert.equal(events[0].line.line_seq, 11);
    const live = await room.send({ text: 'after' });
    const [echo] = await collect(room, 1);
    assert.equal(echo.line.line_seq, live.echo_line_seq); assert.equal(echo.recovered, undefined);
  } finally { await c.close(); }
});

test('a takeover ends the iterator, reads keep working and nothing re-attaches by itself', async () => {
  const c = client('room_takeover');
  try {
    const room = await c.roomOpen({ parent_session_id: 'sess-coding-a' });
    const first = room.epoch;
    const events = await collect(room, 5, 800);
    assert.deepEqual(kinds(events), ['room.status', 'room.closed']);
    assert.equal(events[1].reason, 'taken_over');
    await assert.rejects(room.send({ text: 'x' }), e => e.reason === 'room_closed');
    assert.equal((await room.status()).live, false);
    assert.ok((await room.history()).lines.length >= 10);
    assert.ok((await room.shelf()).rows.length > 0);
    assert.equal(room.epoch, first, 'the epoch stays until the caller asks for the view back');
    assert.deepEqual(await collect(room, 5, 300), [], 'no further events without an attach');
    const again = await room.reattach();
    assert.notEqual(again.epoch, first); assert.equal(room.epoch, again.epoch);
    assert.equal((await room.send({ text: 'back' })).sent, true);
    const [echo] = await collect(room, 1);
    assert.equal(echo.epoch, again.epoch); assert.equal(echo.line.text, 'back');
  } finally { await c.close(); }
});

test('room.close unbinds and optionally stops; the handle is released', async () => {
  const c = client('room');
  try {
    const room = await c.roomAttach({ session_id: (await c.host().listRooms({ live: true })).rooms[0].session_id });
    assert.deepEqual(await room.close({ stop: true }), { closed: true, stopped: true });
    await assert.rejects(room.status(), e => e instanceof SdkError);
    assert.equal(await room.close(), undefined);
  } finally { await c.close(); }
});

test('room calls in the wrong binding carry the binding errors', async () => {
  const c = client('room');
  try {
    const session = await c.create({ cwd: '/fake/work' });
    await assert.rejects(session.transport.request('room.status', {}), e => e.reason === 'wrong_binding' && e.binding === 'coding' && e.allowed.includes('room_live'));
    const room = await c.roomOpen({ parent_session_id: 'sess-coding-a' });
    await assert.rejects(room.transport.request('state', {}), e => e.reason === 'wrong_binding' && e.binding === 'room_live');
    await assert.rejects(room.transport.request('room.open', { parent_session_id: 'sess-coding-a' }), e => e.reason === 'already_bound');
  } finally { await c.close(); }
});
