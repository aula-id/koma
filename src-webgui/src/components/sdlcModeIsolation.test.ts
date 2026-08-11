import assert from 'node:assert/strict'
import type { PushEnvelope } from '../store/koma'

const browser = globalThis as unknown as {
  window?: { ipc?: { postMessage(message: string): void } }
}
browser.window = { ipc: { postMessage: () => {} } }
const { useKoma } = await import('../store/koma')

const snapshot = (
  session: string,
  mode: string,
  extra: Partial<Extract<PushEnvelope, { k: 'Snapshot' }>> = {},
): Extract<PushEnvelope, { k: 'Snapshot' }> => ({
  k: 'Snapshot',
  session,
  state: 'idle',
  messages: [],
  title: session,
  palette: useKoma.getState().palette,
  subagents: [],
  bash: [],
  attachments: [],
  mode,
  ...extra,
})

// Snapshot adopts SDLC fields only in SDLC mode and rejects stale host data otherwise.
useKoma.getState().push(snapshot('sdlc-a', 'sdlc', {
  sdlcPhase: 'execute',
  sdlcGoal: 'goal-a',
  sdlcBranch: 'sdlc/a',
  sdlcOpen: 3,
  sdlcSealed: 2,
  planTodos: [{ content: 'stale plan', status: 'pending', locked: false }],
}))
let session = useKoma.getState().session
assert.equal(session.sdlcPhase, 'execute')
assert.equal(session.sdlcGoal, 'goal-a')
assert.equal(session.sdlcBranch, 'sdlc/a')
assert.equal(session.sdlcOpen, 3)
assert.equal(session.sdlcSealed, 2)
assert.deepEqual(session.planTodos, [])

useKoma.getState().push(snapshot('auto-b', 'auto', {
  sdlcPhase: 'execute',
  sdlcGoal: 'foreign goal',
  sdlcBranch: 'foreign/branch',
  sdlcOpen: 99,
  sdlcSealed: 99,
  planTodos: [{ content: 'foreign plan', status: 'pending', locked: false }],
}))
session = useKoma.getState().session
assert.equal(session.id, 'auto-b')
assert.equal(session.sdlcPhase, null)
assert.equal(session.sdlcGoal, null)
assert.equal(session.sdlcBranch, null)
assert.equal(session.sdlcOpen, null)
assert.equal(session.sdlcSealed, null)
assert.deepEqual(session.planTodos, [])

// Plan rows are adopted only in Plan and an SDLC payload cannot leak alongside them.
useKoma.getState().push(snapshot('plan-c', 'plan', {
  planTodos: [
    { content: 'step 1', status: 'completed', locked: false },
    { content: 'locked rail', status: 'in_progress', locked: true },
  ],
  sdlcPhase: 'integrate',
  sdlcGoal: 'stale mission',
}))
session = useKoma.getState().session
assert.equal(session.planTodos.length, 2)
assert.equal(session.sdlcPhase, null)
assert.equal(session.sdlcGoal, null)

// Status mode changes clear stale rows immediately, before a replacement Snapshot arrives.
useKoma.getState().push({
  k: 'Status',
  session: 'plan-c',
  working: false,
  toast: null,
  mode: 'auto',
})
session = useKoma.getState().session
assert.deepEqual(session.planTodos, [])
assert.equal(session.sdlcPhase, null)

useKoma.getState().push(snapshot('sdlc-d', 'sdlc', {
  sdlcPhase: 'prepare',
  sdlcGoal: 'goal-d',
  sdlcOpen: 1,
  sdlcSealed: 0,
}))
useKoma.getState().push({
  k: 'Status',
  session: 'sdlc-d',
  working: false,
  toast: null,
  mode: 'plan',
})
session = useKoma.getState().session
assert.equal(session.sdlcPhase, null)
assert.equal(session.sdlcGoal, null)
assert.equal(session.sdlcOpen, null)
assert.equal(session.sdlcSealed, null)

console.log('sdlcModeIsolation.test.ts: production push reducer tests passed')
