import assert from 'node:assert/strict'
import { codingTabId, isMarkdownPath, markdownLocalPath } from '../lib/markdownPreview'
import { emptyFileState, fileKey, initialCoding } from './coding'
import { useKoma } from './koma'

const root = '/workspace'
const path = 'docs/README.md'
const key = fileKey(root, path)
const sourceId = codingTabId(root, path)
const previewId = codingTabId(root, path, true)
const requests: Parameters<ReturnType<typeof useKoma.getState>['req']>[0][] = []
const initialUi = useKoma.getState().ui
function reset() {
  requests.length = 0
  useKoma.setState({
    ui: initialUi,
    coding: { ...initialCoding, files: {}, _readReq: {} },
    req: (req) => { requests.push(req) },
  })
}

assert.equal(isMarkdownPath('docs/README.MD'), true)
assert.equal(isMarkdownPath('notes.markdown'), true)
assert.equal(isMarkdownPath('page.mdx'), false)
assert.equal(isMarkdownPath('app.ts'), false)
assert.equal(markdownLocalPath(path, '../assets/a%20b.png'), 'assets/a b.png')
assert.equal(markdownLocalPath(path, './guide.md#setup'), 'docs/guide.md')
assert.equal(markdownLocalPath(path, '/assets/logo.svg'), 'assets/logo.svg')
assert.equal(markdownLocalPath(path, '../../outside.png'), null)
assert.equal(markdownLocalPath(path, '%2e%2e/%2e%2e/outside.png'), null)
assert.equal(markdownLocalPath(path, 'bad%ZZ.png'), null)
assert.equal(markdownLocalPath(path, 'https://example.com/a.png'), null)
assert.equal(markdownLocalPath(path, '//example.com/a.png'), null)
assert.equal(markdownLocalPath(path, '#setup'), null)

// Opening a preview alone reads the file, without opening an editor or writing.
reset()
useKoma.getState().openCodingFile(root, path, { preview: true })
assert.equal(useKoma.getState().ui.activeTabId, previewId)
assert.equal(useKoma.getState().ui.tabs.some((t) => t.id === sourceId), false)
assert.equal(requests.length, 1)
assert.equal(requests[0].r, 'FileRead')
const requestId = useKoma.getState().coding._readReq[key]
useKoma.getState().push({
  k: 'FileRead', root, path, requestId, content: '# Saved', fingerprint: 'fp',
  binary: false, tooLarge: false, error: null,
})
const original = useKoma.getState().coding.files[key]
useKoma.getState().openCodingFile(root, path, { preview: true })
useKoma.getState().openCodingFile(root, path)
assert.equal(requests.length, 1)
assert.equal(useKoma.getState().ui.activeTabId, sourceId)
assert.equal(useKoma.getState().coding.files[key], original)
assert.equal(original.dirty, false)

// Preview always shares unsaved content; opening/closing it never saves, loses
// edits, or closes the source's language-server document.
useKoma.getState().updateCodingContent(root, path, '# Unsaved')
const edited = useKoma.getState().coding.files[key]
requests.length = 0
useKoma.getState().openCodingFile(root, path, { preview: true })
assert.equal(useKoma.getState().coding.files[key], edited)
assert.equal(useKoma.getState().coding.files[key].content, '# Unsaved')
useKoma.getState().closeTab(previewId)
assert.equal(useKoma.getState().ui.tabs.some((t) => t.id === previewId), false)
assert.equal(useKoma.getState().coding.files[key], edited)
assert.equal(requests.some((r) => r.r === 'FileSave' || r.r === 'LspDidClose'), false)
useKoma.getState().closeTab(sourceId)
assert.equal(useKoma.getState().ui.tabs.some((t) => t.id === sourceId), true)

// Explicit source discard restores saved text for a surviving preview.
useKoma.getState().openCodingFile(root, path, { preview: true })
useKoma.getState().closeTab(sourceId, { force: true })
assert.equal(useKoma.getState().coding.files[key].content, '# Saved')
assert.equal(useKoma.getState().coding.files[key].dirty, false)
assert.equal(useKoma.getState().ui.tabs.some((t) => t.id === previewId), true)
useKoma.getState().openCodingFile(root, path)
assert.equal(useKoma.getState().coding.files[key].content, '# Saved')

// Rename preserves distinct preview/source ids and each pane's placement.
useKoma.getState().splitTab(previewId, useKoma.getState().ui.activeGroupId, 'after', 'row')
const previewGroup = useKoma.getState().ui.tabGroup[previewId]
const sourceGroup = useKoma.getState().ui.tabGroup[sourceId]
useKoma.getState().push({
  k: 'FileRename', root, oldPath: 'docs', newPath: 'guide', requestId: 'rename', error: null,
})
const renamedPath = 'guide/README.md'
const renamedPreviewId = codingTabId(root, renamedPath, true)
const renamedSourceId = codingTabId(root, renamedPath)
assert.equal(useKoma.getState().ui.tabGroup[renamedPreviewId], previewGroup)
assert.equal(useKoma.getState().ui.tabGroup[renamedSourceId], sourceGroup)
assert.equal(useKoma.getState().ui.groupActive[previewGroup], renamedPreviewId)
assert.equal(useKoma.getState().coding.files[fileKey(root, renamedPath)].content, '# Saved')
useKoma.getState().push({ k: 'FileDelete', root, path: 'guide', requestId: 'delete', error: null })
assert.equal(useKoma.getState().ui.tabs.some((t) => t.id === renamedPreviewId || t.id === renamedSourceId), false)

// A preview request cannot turn non-Markdown code into a preview tab.
reset()
useKoma.setState({ coding: { ...initialCoding, files: { [fileKey(root, 'app.ts')]: emptyFileState({ content: 'code' }) } } })
useKoma.getState().openCodingFile(root, 'app.ts', { preview: true })
assert.equal(useKoma.getState().ui.activeTabId, codingTabId(root, 'app.ts'))
console.log('Markdown preview tests passed')
