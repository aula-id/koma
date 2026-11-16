import assert from 'node:assert/strict';
import { test } from 'node:test';
import { SdkError } from '../dist/index.js';
import { validateEvent, validateResult } from '../dist/validate.js';
import { METHODS, SESSION_METHODS, USAGE_METHODS, SKILL_METHODS, ROOM_METHODS, CONFIG_METHODS } from '../dist/types.js';

const line = { line_seq: 3, kind: 'future_kind', author: 'scout', display: null, alias: null, lane: 'side', audience: 'nobody', text: 't', call_id: null, tool: null, args: null, ok: null, result: null, replay: false, origin: 'elsewhere', received_at_ms: 1, truncated: false };
const status = { session_id: 's', live: true, fresh: true, age_ms: null, now_at_ms: 1, counts: { open: 0, verified: 0, returned: 0, reports: 0 }, counts_basis: 'new_basis', paused: [], paused_truncated: false,
  lanes: [{ avatar: 'a', display: 'A', alias: 'A', role: 'r', lane: null, phase: 'dreaming', task: '', started_at_ms: 0, last_event_at_ms: 0, steps: 0, last_tool: '', error: '' }], lanes_truncated: false,
  events: [], digest: '', timeline: '', trajectory_outline: '', shelf_fingerprint: 'f', truncated: false };
const row = { session_id: 's', name: null, workdir: null, kind: 'something_new', created_at: null, updated_at: null, live: false, working: null, locked: false, bound: false, parent_session_id: null, room_session_id: null, room_live: false };
const skill = { name: 'n', scope: 'cloud', tier: 'extra-root:3', description: '', triggers: [], triggers_truncated: false, tags: [], tags_truncated: false, pinned: false, editable: false, generation: 'g', path: '/p', effective: true, shadowed_by: null };

const rejected = (method, result, params = {}) => assert.throws(() => validateResult(method, result, params), e => e instanceof SdkError && e.code === 'protocol', method);

test('every method has a result validator', () => {
  for (const method of [...METHODS, ...CONFIG_METHODS, ...SESSION_METHODS, ...USAGE_METHODS, ...SKILL_METHODS, ...ROOM_METHODS]) {
    try { validateResult(method, {}, {}); } catch (e) { assert.ok(e instanceof SdkError && !/No result validator/.test(e.message), method); }
  }
});

test('open enum-like strings pass validation', () => {
  validateResult('sessions.list', { sessions: [row], next_cursor: null }, {});
  validateResult('usage.query', { from_at: 1, to_at: 2, ledger: 'compressed', totals: { calls: 0, tokens_in: 0, tokens_cached: 0, tokens_out: 0, cost: 0 }, groups: [{ calls: 0, tokens_in: 0, tokens_cached: 0, tokens_out: 0, cost: 0.5, key: 'k', label: null, role_kind: 'meta', kind: 'virtual' }], groups_truncated: false }, {});
  validateResult('skills.list', { project_workdir: null, skills: [skill], diagnostics: [], diagnostics_truncated: false, next_cursor: null }, {});
  validateResult('room.status', status, {});
  validateResult('room.history', { epoch: 'e', first_line_seq: 3, last_line_seq: 3, lines: [line], has_more_before: false, has_more_after: false }, { epoch: 'e' });
  validateEvent({ type: 'event', event: 'room.line', seq: 1, epoch: 'e', line });
  validateEvent({ type: 'event', event: 'room.notice', seq: 1, epoch: 'e', code: 'brand_new', text: '' });
  validateEvent({ type: 'event', event: 'room.closed', seq: 1, epoch: 'e', session_id: 's', reason: 'new_reason' });
  validateEvent({ type: 'event', event: 'room.delta', seq: 1, epoch: 'e', author: 'a', lane: 'x', kind: 'y', text: '' });
});

test('wrong shapes are protocol errors', () => {
  rejected('sessions.list', { sessions: [{ ...row, live: 'yes' }], next_cursor: null });
  rejected('sessions.list', { sessions: [row], next_cursor: 5 });
  rejected('sessions.rename', { session: row }, { session_id: 'other' });
  rejected('sessions.stop', { stopped: false, was_live: false });
  const message = { id: 1, role: 'user', text: 't', created_at: 5 };
  const transcript = { session_id: 's', live: false, messages: [message], next_since: null };
  validateResult('sessions.transcript', transcript, { session_id: 's' });
  validateResult('sessions.transcript', { ...transcript, messages: [{ ...message, truncated: true }], next_since: 1 }, { session_id: 's' });
  rejected('sessions.transcript', transcript, { session_id: 'other' });
  rejected('sessions.transcript', { ...transcript, messages: [{ ...message, id: 0 }] }, { session_id: 's' });
  rejected('sessions.transcript', { ...transcript, messages: [{ ...message, role: 'robot' }] }, { session_id: 's' });
  rejected('sessions.transcript', { ...transcript, messages: [{ ...message, truncated: false }] }, { session_id: 's' });
  rejected('sessions.transcript', { ...transcript, next_since: 0 }, { session_id: 's' });
  rejected('sessions.transcript', { ...transcript, live: 'no' }, { session_id: 's' });
  rejected('usage.query', { from_at: 1, to_at: 2, ledger: '', totals: {} });
  rejected('skills.get', { ...skill, instruction: 1 });
  rejected('room.send', { sent: true, acked: true, inserts: [], echo_line_seq: -1 });
  rejected('room.history', { epoch: 'e', first_line_seq: null, last_line_seq: null, lines: [], has_more_before: false, has_more_after: false }, { epoch: 'other' });
  rejected('room.status', { ...status, lanes: [{}] });
  rejected('room.models.set', { team: {}, applies: 'live', propagation: {} }, { target: 'room' });
  assert.throws(() => validateEvent({ type: 'event', event: 'room.line', seq: 1, epoch: '', line }), SdkError);
  assert.throws(() => validateEvent({ type: 'event', event: 'room.line', seq: 1, epoch: 'e', line: { ...line, line_seq: 0 } }), SdkError);
  assert.throws(() => validateEvent({ type: 'event', event: 'room.gap', seq: 1, epoch: 'e', after_line_seq: 1, dropped_lines: -1, dropped_deltas: 0 }), SdkError);
});
