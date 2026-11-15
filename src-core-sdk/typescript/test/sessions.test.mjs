import assert from 'node:assert/strict';
import { existsSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { DeliveryUnknownError, SdkError } from '../dist/index.js';
import { client } from './support.mjs';

test('sessions.list orders, filters and pages by cursor', async () => {
  const c = client('sessions'), host = c.host();
  try {
    const all = await host.listSessions();
    assert.ok(all.sessions.length > 8);
    const stamps = all.sessions.map(s => s.updated_at ?? -1);
    assert.deepEqual(stamps, [...stamps].sort((a, b) => b - a));
    const row = all.sessions.find(s => s.session_id === 'sess-coding-a');
    assert.equal(row.name, 'Alpha'); assert.equal(row.live, true); assert.equal(row.working, true); assert.equal(row.kind, 'coding');
    assert.equal(all.sessions.find(s => s.session_id === 'sess-live-nr').name, null);
    const rooms = await host.listSessions({ kind: 'room' });
    assert.ok(rooms.sessions.length >= 2 && rooms.sessions.every(s => s.kind === 'room' && s.working === null));
    const seen = []; let cursor;
    do {
      const page = await host.listSessions({ limit: 3, cursor });
      assert.ok(page.sessions.length <= 3); seen.push(...page.sessions.map(s => s.session_id)); cursor = page.next_cursor ?? undefined;
    } while (cursor);
    assert.deepEqual(seen, all.sessions.map(s => s.session_id));
    await assert.rejects(host.listSessions({ cursor: 'nonsense' }), e => e instanceof SdkError && e.reason === 'bad_cursor');
    await assert.rejects(host.listSessions({ limit: 501 }), e => e.reason === 'out_of_range');
  } finally { await host.close(); await c.close(); }
});

test('sessions.rename: not live, live elsewhere, busy', async () => {
  const c = client('sessions'), host = c.host();
  try {
    const done = await host.renameSession({ session_id: 'sess-coding-b', name: '  Renamed  ' });
    assert.equal(done.session.name, 'Renamed');
    await assert.rejects(host.renameSession({ session_id: 'sess-coding-a', name: 'x' }),
      e => e instanceof SdkError && e.code === 'invalid_state' && e.reason === 'live' && e.delivery === 'not_sent');
    await assert.rejects(host.renameSession({ session_id: 'sess-busy', name: 'x' }), e => e.reason === 'sqlite_busy');
    await assert.rejects(host.renameSession({ session_id: 'nope', name: 'x' }), e => e.code === 'not_found');
    await assert.rejects(host.renameSession({ session_id: '', name: 'x' }), e => e.code === 'invalid_input');
  } finally { await host.close(); await c.close(); }
});

test('sessions.delete and sessions.stop report their outcomes', async () => {
  const c = client('sessions'), host = c.host();
  try {
    await assert.rejects(host.deleteSession({ session_id: 'sess-locked' }), e => e.reason === 'session_locked');
    await assert.rejects(host.deleteSession({ session_id: 'sess-coding-a' }), e => e.reason === 'live');
    await assert.rejects(host.deleteSession({ session_id: 'sess-partial' }),
      e => e instanceof DeliveryUnknownError && e.reason === 'partial_delete' && e.written === true && e.sessionId === 'sess-partial');
    assert.equal((await host.deleteSession({ session_id: 'sess-partial' })).deleted, true);
    await assert.rejects(host.deleteSession({ session_id: 'sess-partial' }), e => e.code === 'not_found');
    const parent = await host.deleteSession({ session_id: 'sess-coding-b' });
    assert.equal(parent.kind, 'coding'); assert.equal(typeof parent.orphaned_room_session_id, 'string');
    assert.deepEqual(await host.stopSession({ session_id: 'nope' }), { stopped: true, was_live: false });
    assert.equal((await host.stopSession({ session_id: 'sess-coding-a' })).was_live, true);
    await assert.rejects(host.stopSession({ session_id: 'sess-stuck' }), e => e instanceof DeliveryUnknownError && e.reason === 'still_running');
    assert.equal((await host.listSessions({ limit: 1 })).sessions.length, 1, 'the adapter stays up after an unknown outcome');
  } finally { await host.close(); await c.close(); }
});

test('create{name} applies the name, reports a failed rename and is gated on its feature', async () => {
  const c = client('sessions');
  try {
    const named = await c.create({ cwd: '/fake/work', name: '  My session ' });
    assert.equal(named.sessionInfo.name, 'My session'); assert.deepEqual(named.warnings, []);
    const row = (await named.listSessions()).sessions.find(s => s.session_id === named.sessionId);
    assert.equal(row.name, 'My session'); assert.equal(row.bound, true);
    await named.close();
    const failed = await c.create({ cwd: '/fake/work', name: 'fail-rename' });
    assert.equal(failed.warnings[0].code, 'rename_failed'); assert.equal(failed.sessionInfo.name, failed.sessionId);
    for (const name of ['', '   ', 'x'.repeat(201), 'tab\there', 'nul\u0000']) {
      await assert.rejects(c.create({ cwd: '/fake/work', name }), e => e.code === 'invalid_input', JSON.stringify(name));
    }
  } finally { await c.close(); }
  const log = join(tmpdir(), `koma-create-name-${process.pid}.log`);
  rmSync(log, { force: true });
  const old = client('legacy_hello', { env: { KOMA_FAKE_LOG: log } });
  try {
    await assert.rejects(old.create({ cwd: '/fake/work', name: 'x' }), e => e instanceof SdkError && e.code === 'unsupported_feature' && e.delivery === 'not_sent');
    if (existsSync(log)) assert.equal(readFileSync(log, 'utf8').includes('create'), false);
    assert.equal((await old.create({ cwd: '/fake/work' })).warnings.length, 0);
  } finally { await old.close(); rmSync(log, { force: true }); }
});

test('session and room handles reach the host calls in their own binding', async () => {
  const c = client('sessions');
  try {
    const s = await c.create({ cwd: '/fake/work' });
    assert.ok((await s.listSessions()).sessions.some(x => x.session_id === s.sessionId));
    assert.equal((await s.renameSession({ session_id: s.sessionId, name: 'Bound' })).session.name, 'Bound');
    await assert.rejects(s.deleteSession({ session_id: s.sessionId }), e => e.reason === 'bound_self');
  } finally { await c.close(); }
});

test('a legacy adapter fails the new calls locally', async () => {
  const c = client('legacy_hello'), host = c.host();
  try {
    assert.equal(await host.supports('sessions.list'), false);
    await assert.rejects(host.listSessions(), e => e.code === 'unsupported_method' && e.delivery === 'not_sent');
  } finally { await host.close(); await c.close(); }
});
