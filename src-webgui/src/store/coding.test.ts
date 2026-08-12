import assert from 'node:assert/strict'
import {
  emptyFileState,
  fileKey,
  initialCoding,
  reduceFileCreate,
  reduceFileDelete,
  reduceFileRename,
  reduceFileSave,
  reduceFileTree,
  type CodingSlice,
} from './coding'

const root = '/ws'
const withCaches = (partial: Partial<CodingSlice> = {}): CodingSlice => ({
  ...initialCoding,
  ...partial,
  dirs: partial.dirs ?? {},
  files: partial.files ?? {},
  _readReq: partial._readReq ?? {},
  _treeReq: partial._treeReq ?? {},
})

// These are the production reducers imported from ./coding (not test copies).
{
  const key = fileKey(root, '')
  const coding = withCaches({ _treeReq: { [key]: 'new' } })
  assert.equal(
    reduceFileTree(coding, {
      k: 'FileTree', root, path: '', requestId: 'old', entries: [], error: null,
    }),
    coding,
  )
}

{
  const key = fileKey(root, 'src')
  const coding = withCaches({ dirs: { [key]: { entries: [], loading: false, error: null } } })
  assert.equal(
    reduceFileCreate(coding, {
      k: 'FileCreate', root, path: 'src/new.ts', requestId: 'create-1', error: null,
    }).dirs[key],
    undefined,
  )
  assert.equal(
    reduceFileCreate(coding, {
      k: 'FileCreate', root, path: 'src/new.ts', requestId: 'create-2', error: 'denied',
    }),
    coding,
  )
}

// Directory rename/delete remap and drop the real coding caches, respectively.
{
  const coding = withCaches({
    dirs: {
      [fileKey(root, 'src')]: { entries: [], loading: false, error: null },
      [fileKey(root, 'src/nested')]: { entries: [], loading: false, error: null },
    },
    files: { [fileKey(root, 'src/nested/a.ts')]: emptyFileState({ content: 'a' }) },
  })
  const renamed = reduceFileRename(coding, {
    k: 'FileRename', root, oldPath: 'src', newPath: 'lib', requestId: 'rename-1', error: null,
  })
  assert.ok(renamed.dirs[fileKey(root, 'lib/nested')])
  assert.ok(renamed.files[fileKey(root, 'lib/nested/a.ts')])
  const deleted = reduceFileDelete(renamed, {
    k: 'FileDelete', root, path: 'lib', requestId: 'delete-1', error: null,
  })
  assert.equal(deleted.dirs[fileKey(root, 'lib/nested')], undefined)
  assert.equal(deleted.files[fileKey(root, 'lib/nested/a.ts')], undefined)
}

// FileSave success clears dirty/saving; conflict keeps dirty content.
{
  const key = fileKey(root, 'src/a.ts')
  const dirty = withCaches({
    files: {
      [key]: emptyFileState({
        content: 'edited',
        savedContent: 'old',
        fingerprint: 'fp1',
        dirty: true,
        saving: true,
      }),
    },
  })
  const saved = reduceFileSave(dirty, {
    k: 'FileSave', root, path: 'src/a.ts', requestId: 'save-1', fingerprint: 'fp2', error: null,
  })
  assert.equal(saved.files[key]?.dirty, false)
  assert.equal(saved.files[key]?.saving, false)
  assert.equal(saved.files[key]?.savedContent, 'edited')
  assert.equal(saved.files[key]?.fingerprint, 'fp2')

  const conflicted = reduceFileSave(dirty, {
    k: 'FileSave', root, path: 'src/a.ts', requestId: 'save-2', fingerprint: '', error: 'conflict',
  })
  assert.equal(conflicted.files[key]?.dirty, true)
  assert.equal(conflicted.files[key]?.saving, false)
  assert.equal(conflicted.files[key]?.conflict, true)
  assert.equal(conflicted.files[key]?.content, 'edited')
}

// Safe autosave preconditions (mirrors CodeEditorTab's 750ms guard).
function canAutosave(file: ReturnType<typeof emptyFileState>, enabled: boolean): boolean {
  if (!enabled) return false
  if (!file.dirty || file.content == null) return false
  if (file.saving || file.conflict || file.binary || file.tooLarge || file.error) return false
  if (file.content === (file.savedContent ?? '')) return false
  return true
}
{
  const base = emptyFileState({
    content: 'edited',
    savedContent: 'old',
    dirty: true,
  })
  assert.equal(canAutosave(base, true), true)
  assert.equal(canAutosave(base, false), false)
  assert.equal(canAutosave({ ...base, saving: true }, true), false)
  assert.equal(canAutosave({ ...base, conflict: true }, true), false)
  assert.equal(canAutosave({ ...base, content: 'old', dirty: false }, true), false)
  assert.equal(canAutosave({ ...base, binary: true }, true), false)
  assert.equal(canAutosave({ ...base, error: 'nope' }, true), false)
}

// koma.ts reads browser globals during actions, so provide only the bridge needed
// by this Node test before importing the store. The test still exercises req's
// production missing-IPC path below.
const browser = globalThis as unknown as {
  window?: {
    ipc?: { postMessage(message: string): void }
    confirm?: (message?: string) => boolean
  }
}
browser.window = {
  ipc: { postMessage: () => {} },
  confirm: () => {
    throw new Error('window.confirm must not be used for dirty coding closes')
  },
}
const { useKoma } = await import('./koma')

const codingTabs = [
  { id: 'chat', kind: 'chat' as const },
  { id: `coding:${root}:src/a.ts`, kind: 'codingFile' as const, root, path: 'src/a.ts', title: 'a.ts' },
  { id: `coding:${root}:src/nested/b.ts`, kind: 'codingFile' as const, root, path: 'src/nested/b.ts', title: 'b.ts' },
  { id: 'coding:/other:src/a.ts', kind: 'codingFile' as const, root: '/other', path: 'src/a.ts', title: 'a.ts' },
]
useKoma.setState((s) => ({
  coding: withCaches(),
  ui: { ...s.ui, tabs: codingTabs, activeTabId: `coding:${root}:src/nested/b.ts`, toast: null },
}))
useKoma.getState().push({
  k: 'FileRename', root, oldPath: 'src', newPath: 'lib', requestId: 'rename', error: null,
})
let tabs = useKoma.getState().ui.tabs
assert.ok(tabs.some((t) => t.id === `coding:${root}:lib/a.ts`))
assert.ok(tabs.some((t) => t.id === `coding:${root}:lib/nested/b.ts`))
assert.equal(useKoma.getState().ui.activeTabId, `coding:${root}:lib/nested/b.ts`)
assert.ok(tabs.some((t) => t.id === 'coding:/other:src/a.ts'))

useKoma.getState().push({
  k: 'FileDelete', root, path: 'lib', requestId: 'delete', error: null,
})
tabs = useKoma.getState().ui.tabs
assert.equal(tabs.some((t) => t.kind === 'codingFile' && t.root === root), false)
assert.equal(useKoma.getState().ui.activeTabId, 'chat')

// Dirty close is a no-op without force; force discards without window.confirm.
{
  const id = `coding:${root}:dirty.ts`
  const key = fileKey(root, 'dirty.ts')
  useKoma.setState((s) => ({
    coding: withCaches({
      files: {
        [key]: emptyFileState({ content: 'x', savedContent: '', dirty: true }),
      },
    }),
    ui: {
      ...s.ui,
      tabs: [
        { id: 'chat', kind: 'chat' as const },
        { id, kind: 'codingFile' as const, root, path: 'dirty.ts', title: 'dirty.ts' },
      ],
      activeTabId: id,
      toast: null,
    },
  }))
  useKoma.getState().closeTab(id)
  assert.ok(useKoma.getState().ui.tabs.some((t) => t.id === id))
  useKoma.getState().closeTab(id, { force: true })
  assert.equal(useKoma.getState().ui.tabs.some((t) => t.id === id), false)
  assert.equal(useKoma.getState().ui.activeTabId, 'chat')
}

// SettingsValues carries codingAutosave through the store reducer.
{
  useKoma.getState().push({
    k: 'SettingsValues',
    name: 's',
    workdir: ['/ws'],
    shortSend: true,
    slidingCache: false,
    bashSaving: true,
    codingAutosave: true,
    internetMode: 'simple',
    palette: 'dark',
    effort: '',
    subagentMaxTurns: 500,
  })
  assert.equal(useKoma.getState().settingsValues?.codingAutosave, true)
  useKoma.getState().push({
    k: 'SettingsValues',
    name: 's',
    workdir: ['/ws'],
    shortSend: true,
    slidingCache: false,
    bashSaving: true,
    codingAutosave: false,
    internetMode: 'simple',
    palette: 'dark',
    effort: '',
    subagentMaxTurns: 500,
  })
  assert.equal(useKoma.getState().settingsValues?.codingAutosave, false)
}

// Missing IPC is a production req behavior, not an inlined helper implementation.
browser.window = {
  confirm: () => {
    throw new Error('window.confirm must not be used for dirty coding closes')
  },
}
useKoma.getState().req({ r: 'FileDelete', root, path: 'gone', requestId: 'missing-ipc' })
assert.match(useKoma.getState().ui.toast?.text ?? '', /IPC unavailable/)
assert.equal(useKoma.getState().ui.toast?.kind, 'error')

browser.window = {
  ipc: { postMessage: () => { throw new Error('bridge down') } },
  confirm: () => {
    throw new Error('window.confirm must not be used for dirty coding closes')
  },
}
useKoma.getState().req({ r: 'FileCreate', root, path: 'new', kind: 'file', requestId: 'throwing-ipc' })
assert.match(useKoma.getState().ui.toast?.text ?? '', /IPC error.*bridge down/)
assert.equal(useKoma.getState().ui.toast?.kind, 'error')

console.log('coding.test.ts: all assertions passed')
