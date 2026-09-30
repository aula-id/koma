import assert from 'node:assert/strict'
import { emptyDesignFile, emptyDesignFileUi, claimDesignRead, initialDesign, normalizeDesignSlice } from './design.ts'
import { fileKey } from './coding.ts'

{
  const missing = normalizeDesignSlice({ pendingCreate: null, docs: { x: emptyDesignFile() } })
  assert.deepEqual(missing.fileUi, {})
  assert.equal(missing.panelTabId, null)
  assert.ok(missing.docs.x)
}

{
  const root = '/ws'
  const path = '.koma/ui.kdsgn'
  const key = fileKey(root, path)
  const readReq = 'read-1'
  const before = {
    ...initialDesign,
    panelTabId: 'design:/ws:.koma/ui.kdsgn',
    fileUi: { [key]: { ...emptyDesignFileUi(), selection: ['n1'] } },
    pendingCreate: { root, path, createReq: 'create-1', readReq },
    docs: { [key]: emptyDesignFile({ loading: true, readReq }) },
  }
  const claimed = claimDesignRead(before, {
    root,
    path,
    requestId: readReq,
    content: '',
    fingerprint: 'fp1',
    binary: false,
    tooLarge: false,
    error: null,
  })
  assert.ok(claimed)
  assert.ok(claimed.save)
  assert.equal(claimed.design.fileUi[key]?.selection[0], 'n1')
  assert.equal(claimed.design.panelTabId, before.panelTabId)
  assert.equal(claimed.design.docs[key]?.saving, true)
}

{
  const claimed = claimDesignRead(
    { pendingCreate: null, docs: {}, panelTabId: null, fileUi: undefined as never },
    {
      root: '/ws',
      path: '.koma/x.kdsgn',
      requestId: 'nope',
      content: '',
      fingerprint: '',
      binary: false,
      tooLarge: false,
      error: null,
    },
  )
  assert.equal(claimed, null)
}

console.log('design.test.ts ok')
