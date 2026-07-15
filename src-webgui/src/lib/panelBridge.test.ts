import assert from 'node:assert/strict'

// koma.ts reads browser globals during actions (see store/coding.test.ts),
// and panelBridge.ts imports useKoma from koma.ts — set up the bridge before
// dynamically importing either module.
const browser = globalThis as unknown as {
  window?: {
    ipc?: { postMessage(message: string): void }
    location?: { origin: string }
  }
}
const ipcCalls: unknown[] = []
browser.window = {
  ipc: {
    postMessage: (message: string) => {
      ipcCalls.push(JSON.parse(message))
    },
  },
}

const { useKoma } = await import('../store/koma')
const {
  registerPanelFrame,
  unregisterPanelFrame,
  postToPanel,
  handlePanelMessage,
} = await import('./panelBridge')

function fakeWindow(): { postMessage: (msg: unknown, targetOrigin: string) => void; calls: unknown[][] } {
  const calls: unknown[][] = []
  return {
    calls,
    postMessage: (msg, targetOrigin) => {
      calls.push([msg, targetOrigin])
    },
  }
}

// ---- registry: register / replace / unregister --------------------------
{
  const win1 = fakeWindow()
  const win2 = fakeWindow()

  registerPanelFrame('ext1', 'panelA', win1 as unknown as Window)
  assert.equal(postToPanel('ext1', 'panelA', { hello: 1 }), true)
  assert.equal(win1.calls.length, 1)
  assert.deepEqual(win1.calls[0][0], { hello: 1 })

  // Re-registering the same key (reload) replaces the entry — the old
  // Window stops receiving traffic.
  registerPanelFrame('ext1', 'panelA', win2 as unknown as Window)
  assert.equal(postToPanel('ext1', 'panelA', { hello: 2 }), true)
  assert.equal(win1.calls.length, 1, 'stale window must not receive further posts')
  assert.equal(win2.calls.length, 1)

  unregisterPanelFrame('ext1', 'panelA')
  assert.equal(postToPanel('ext1', 'panelA', { hello: 3 }), false)
  assert.equal(win2.calls.length, 1, 'unregistered window must not receive further posts')
}

// ---- postToPanel: unknown key returns false ------------------------------
{
  assert.equal(postToPanel('nope', 'nope', { x: 1 }), false)
}

// ---- handlePanelMessage: wrong / unregistered source is ignored ---------
{
  const registered = fakeWindow()
  const stranger = fakeWindow()
  registerPanelFrame('ext2', 'panelA', registered as unknown as Window)
  ipcCalls.length = 0

  handlePanelMessage({
    source: stranger as unknown as Window,
    origin: '',
    data: { koma: 'panel', v: 1, kind: 'msg', reqId: 'r1', payload: {} },
  })

  assert.equal(registered.calls.length, 0)
  assert.equal(ipcCalls.length, 0)
  unregisterPanelFrame('ext2', 'panelA')
}

// ---- handlePanelMessage: oversized payload gets a local reply -----------
{
  const win = fakeWindow()
  registerPanelFrame('ext3', 'panelA', win as unknown as Window)
  useKoma.setState((s) => ({ session: { ...s.session, id: 'sess-1' } }))
  ipcCalls.length = 0

  const bigPayload = { blob: 'x'.repeat(300000) }
  handlePanelMessage({
    source: win as unknown as Window,
    origin: '',
    data: { koma: 'panel', v: 1, kind: 'msg', reqId: 'r2', payload: bigPayload },
  })

  assert.equal(ipcCalls.length, 0, 'oversized payload must not be forwarded to the daemon')
  assert.equal(win.calls.length, 1)
  assert.deepEqual(win.calls[0][0], {
    koma: 'host',
    v: 1,
    kind: 'reply',
    reqId: 'r2',
    ok: false,
    error: 'payload too large',
  })
  unregisterPanelFrame('ext3', 'panelA')
}

// ---- handlePanelMessage: bad origin is dropped (no reply, no forward) ---
{
  const win = fakeWindow()
  registerPanelFrame('ext4', 'panelA', win as unknown as Window)
  ipcCalls.length = 0

  handlePanelMessage({
    source: win as unknown as Window,
    origin: 'https://evil.example',
    data: { koma: 'panel', v: 1, kind: 'msg', reqId: 'r3', payload: {} },
  })

  assert.equal(win.calls.length, 0)
  assert.equal(ipcCalls.length, 0)
  unregisterPanelFrame('ext4', 'panelA')
}

// ---- handlePanelMessage: no active session replies locally --------------
{
  const win = fakeWindow()
  registerPanelFrame('ext5', 'panelA', win as unknown as Window)
  useKoma.setState((s) => ({ session: { ...s.session, id: null } }))
  ipcCalls.length = 0

  handlePanelMessage({
    source: win as unknown as Window,
    origin: '',
    data: { koma: 'panel', v: 1, kind: 'msg', reqId: 'r4', payload: { a: 1 } },
  })

  assert.equal(ipcCalls.length, 0)
  assert.equal(win.calls.length, 1)
  assert.deepEqual(win.calls[0][0], {
    koma: 'host',
    v: 1,
    kind: 'reply',
    reqId: 'r4',
    ok: false,
    error: 'no active koma session',
  })
  unregisterPanelFrame('ext5', 'panelA')
}

// ---- handlePanelMessage: happy path forwards via req ---------------------
{
  const win = fakeWindow()
  registerPanelFrame('ext6', 'panelB', win as unknown as Window)
  useKoma.setState((s) => ({ session: { ...s.session, id: 'sess-1' } }))
  ipcCalls.length = 0

  handlePanelMessage({
    // A real origin carries no path — the panel origin is exactly
    // `koma://extension` (or `http://koma.extension` on Windows); the strict
    // allowlist in handlePanelMessage matches the bare origin, not a prefix.
    source: win as unknown as Window,
    origin: 'koma://extension',
    data: { koma: 'panel', v: 1, kind: 'msg', reqId: 'r5', payload: { a: 1 } },
  })

  assert.equal(win.calls.length, 0, 'happy path is forwarded, not answered locally')
  assert.equal(ipcCalls.length, 1)
  assert.deepEqual(ipcCalls[0], {
    t: 'req',
    r: 'ExtPanelMsg',
    extId: 'ext6',
    panelId: 'panelB',
    reqId: 'r5',
    payload: { a: 1 },
  })
  unregisterPanelFrame('ext6', 'panelB')
}

console.log('panelBridge.test.ts: all assertions passed')
