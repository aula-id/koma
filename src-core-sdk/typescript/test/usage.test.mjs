import assert from 'node:assert/strict';
import { test } from 'node:test';
import { SdkError } from '../dist/index.js';
import { client } from './support.mjs';

test('usage.query totals, groups and buckets', async () => {
  const c = client('usage'), host = c.host();
  try {
    const plain = await host.queryUsage();
    assert.equal(plain.ledger, 'ok'); assert.equal(plain.groups, undefined); assert.equal(plain.buckets, undefined);
    assert.ok(plain.totals.calls > 0 && plain.to_at > plain.from_at);
    const byRole = await host.queryUsage({ group_by: 'role' });
    assert.ok(byRole.groups.some(g => g.role_kind === 'main') && byRole.groups.some(g => g.role_kind === 'sub' && g.agent));
    assert.equal(byRole.groups_truncated, false);
    const costs = byRole.groups.map(g => g.cost);
    assert.deepEqual(costs, [...costs].sort((a, b) => b - a));
    const sessions = await host.queryUsage({ group_by: 'session' });
    const gone = sessions.groups.find(g => g.key === 'sess-deleted');
    assert.equal(gone.deleted, true); assert.equal(gone.label, null);
    const cut = await host.queryUsage({ group_by: 'model', limit: 1 });
    assert.equal(cut.groups.length, 1);
    const days = await host.queryUsage({ bucket: 'day' });
    assert.ok(days.buckets.length >= 30 && days.buckets.every((b, i) => i === 0 || b.start_at > days.buckets[i - 1].start_at));
    const wide = await host.queryUsage({ from_at: plain.to_at - 60 * 86400 });
    assert.ok(wide.totals.calls > plain.totals.calls);
  } finally { await host.close(); await c.close(); }
});

test('usage.query errors keep their reason and delivery', async () => {
  const c = client('usage'), host = c.host();
  try {
    await assert.rejects(host.queryUsage({ bucket: 'hour', from_at: 0, to_at: 400 * 86400 }), e => e instanceof SdkError && e.reason === 'range_too_large');
    await assert.rejects(host.queryUsage({ session_id: 'sess-slow' }), e => e.code === 'unavailable' && e.reason === 'query_timeout' && e.delivery === 'not_sent');
    await assert.rejects(host.queryUsage({ tz_offset_minutes: 900 }), e => e.reason === 'out_of_range');
  } finally { await host.close(); await c.close(); }
  const none = client('normal'), h2 = none.host();
  try { assert.equal((await h2.queryUsage()).ledger, 'missing'); } finally { await h2.close(); await none.close(); }
});
