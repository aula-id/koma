import { useKoma, type CodingFileState } from '../store/koma'
import { emptyFileState, fileKey } from '../store/coding'
import { codingRequest, type WorkspaceRef } from './coding-service'
import { checkpointCodingDocument } from './coding-recovery'
import { uriToPath, splitRootPath } from './lsp-bridge'
import { applyWorkspaceTextEdits } from './workspace-edit-text'
import type { WorkspaceEdit, StagedEdit, StagedFile } from './coding-edits'
type Snapshot = { path: string; content: string | null; exists: boolean; fingerprint: string }
type Virtual = { snapshot: Snapshot; content: string | null; formatFrom: string; initial?: CodingFileState }
type Outcome = { id?: string; files: { path: string; exists: boolean; fingerprint: string }[] }
export async function stageResourceEdit(workspace: WorkspaceRef, edit: WorkspaceEdit, label: string, snapshot: Record<string, CodingFileState>, generation: number): Promise<StagedEdit> {
  const files = new Map<string, Virtual>()
  const path = (uri?: string) => { const abs = uri ? uriToPath(uri) : null; const target = abs ? splitRootPath(abs, [workspace.root]) : null; if (!target?.path) throw new Error('Resource edit includes a path outside this workspace'); return target.path }
  let size = 0
  const get = async (path: string) => {
    const value = files.get(path); if (value) return value
    if (files.size >= 100) throw new Error('Resource edit exceeds 100 files')
    const initial = snapshot[fileKey(workspace.root, path)]
    if (initial && (initial.saving || initial.loading || initial.conflict || initial.content !== initial.savedContent)) throw new Error(`Save or resolve ${path} before applying a file resource edit`)
    const [disk] = await codingRequest<Snapshot[]>(workspace, { op: 'resourceInspect', paths: [path] })
    if (initial?.content != null && (initial.fingerprint !== disk.fingerprint || initial.content !== disk.content)) throw new Error(`${path} changed on disk; reload it before previewing this action`)
    size += disk.content?.length ?? 0; if (size > 20 * 1024 * 1024) throw new Error('Resource edit exceeds 20 MiB')
    const entry = { snapshot: disk, content: disk.content, formatFrom: path, initial }; files.set(path, entry); return entry
  }
  for (const [uri, edits] of Object.entries(edit.changes ?? {})) { const file = await get(path(uri)); if (file.content == null) throw new Error('Text edit targets a missing file'); file.content = applyWorkspaceTextEdits(file.content, edits) }
  for (const change of edit.documentChanges ?? []) {
    if (change.kind === 'create') { const file = await get(path(change.uri)); if (file.content != null && !change.options?.overwrite) { if (change.options?.ignoreIfExists) continue; throw new Error('Create target already exists') } file.content = ''; file.formatFrom = path(change.uri) }
    else if (change.kind === 'rename') {
      const sourcePath = path(change.oldUri), targetPath = path(change.newUri); if (sourcePath === targetPath) continue
      const source = await get(sourcePath), target = await get(targetPath)
      if (source.content == null) throw new Error('Rename source does not exist')
      if (target.content != null && !change.options?.overwrite) { if (change.options?.ignoreIfExists) continue; throw new Error('Rename target already exists') }
      target.content = source.content; target.formatFrom = source.formatFrom; source.content = null
    } else if (change.kind === 'delete') { const file = await get(path(change.uri)); if (file.content == null && !change.options?.ignoreIfNotExists) throw new Error('Delete target does not exist'); file.content = null }
    else if (change.textDocument && change.edits) { const file = await get(path(change.textDocument.uri)); if (file.content == null) throw new Error('Text edit targets a missing file'); file.content = applyWorkspaceTextEdits(file.content, change.edits) }
    else throw new Error('Unsupported resource operation')
  }
  const staged: StagedFile[] = [...files].map(([path, file]) => ({ path, before: file.snapshot.content ?? '', after: file.content ?? '', fingerprint: file.snapshot.fingerprint, savedContent: file.snapshot.content, initial: file.initial, existed: file.snapshot.exists, existsAfter: file.content != null, formatFrom: file.formatFrom }))
  if (staged.reduce((n, f) => n + f.before.length + f.after.length, 0) > 20 * 1024 * 1024) throw new Error('Resource edit exceeds 20 MiB')
  return { workspace, files: staged, label, generation, resources: true }
}
function sync(edit: StagedEdit, outcome: Outcome, reverse: boolean) {
  // A context switch or typing during the disk operation must never discard a buffer.
  const current = useKoma.getState()
  if ((current.remoteState.hostId ?? 'local') !== edit.workspace.hostId || current.coding._sessionGen !== edit.generation) return
  const transfers = edit.files.filter(f => f.existsAfter && !f.existed && f.formatFrom && f.formatFrom !== f.path && edit.files.some(source => source.path === f.formatFrom && source.existed && !source.existsAfter)).map(f => ({ from: reverse ? f.path : f.formatFrom!, to: reverse ? f.formatFrom! : f.path }))
    .filter(({ from, to }) => { const source = current.coding.files[fileKey(edit.workspace.root, from)], target = current.coding.files[fileKey(edit.workspace.root, to)]; return source && !source.dirty && !source.saving && !source.loading && (!target || !target.dirty && !target.saving && !target.loading) })
  useKoma.setState(s => {
    const files = { ...s.coding.files }
    for (const { from, to } of transfers) { files[fileKey(edit.workspace.root, to)] = files[fileKey(edit.workspace.root, from)]; delete files[fileKey(edit.workspace.root, from)] }
    for (const file of edit.files) {
      if (transfers.some(t => t.from === file.path)) continue
      const result = outcome.files.find(v => v.path === file.path)!
      const key = fileKey(edit.workspace.root, file.path), present = files[key]
      const transfer = transfers.find(t => t.to === file.path)
      const origin = transfer ? edit.files.find(f => f.path === transfer.from)! : file
      const expected = reverse && origin.existsAfter ? origin.after : origin.before
      if (present?.content != null && present.content !== expected) { files[key] = { ...present, conflict: true, manualSaveRequired: true, error: 'The file changed during a resource edit. Your buffer was preserved.' }; continue }
      const content = reverse ? file.before : file.after
      files[key] = result.exists ? { ...(present ?? emptyFileState()), content, savedContent: content, fingerprint: result.fingerprint, dirty: false, conflict: false, loading: false, error: null, manualSaveRequired: false } : { ...(present ?? emptyFileState()), conflict: true, error: 'File removed by workspace edit. Undo the workspace edit to restore it.', manualSaveRequired: true }
    }
    return { coding: { ...s.coding, files } }
  })
  for (const { from, to } of transfers) useKoma.getState().push({ k: 'FileRename', root: edit.workspace.root, oldPath: from, newPath: to, requestId: '', error: null })
  window.dispatchEvent(new CustomEvent('koma-coding-disk', { detail: edit.workspace }))
}
export async function applyResourceEdit(edit: StagedEdit, verify: () => void) {
  for (const file of edit.files) if (file.existed) await checkpointCodingDocument(edit.workspace, file.path, file.before, `Before ${edit.label}`)
  verify()
  const outcome = await codingRequest<Outcome>(edit.workspace, { op: 'resourceApply', changes: edit.files.map(f => ({ path: f.path, fingerprint: f.fingerprint, after: f.existsAfter ? f.after : null, formatFrom: f.formatFrom })) })
  edit.transactionId = outcome.id
  sync(edit, outcome, false)
  if ((useKoma.getState().remoteState.hostId ?? 'local') === edit.workspace.hostId) for (const file of edit.files) if (!file.existed && file.existsAfter) useKoma.getState().openCodingFile(edit.workspace.root, file.path)
}
export async function undoResourceEdit(edit: StagedEdit) {
  if (!edit.transactionId) throw new Error('Resource undo is unavailable')
  const state = useKoma.getState()
  if ((state.remoteState.hostId ?? 'local') !== edit.workspace.hostId) throw new Error('Return to the original host to undo')
  edit.generation = state.coding._sessionGen
  for (const file of edit.files) { const current = state.coding.files[fileKey(edit.workspace.root, file.path)]; if (current && (current.saving || current.loading || current.dirty || (file.existsAfter && current.content !== file.after))) throw new Error(`Save or discard subsequent changes in ${file.path} before undoing`) }
  const outcome = await codingRequest<Outcome>(edit.workspace, { op: 'resourceUndo', transactionId: edit.transactionId })
  sync(edit, outcome, true)
}
