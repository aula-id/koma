import { useKoma, type CodingFileState } from '../store/koma'
import { emptyFileState, fileKey, type FileReadPush } from '../store/coding'
import { codingRequest, type WorkspaceRef } from './coding-service'
import { backupCodingDocument, checkpointCodingDocument, forgetCodingDraft } from './coding-recovery'
import { uriToPath, splitRootPath } from './lsp-bridge'
import { applyWorkspaceTextEdits, type TextEdit } from './workspace-edit-text'

export type WorkspaceEdit = { changes?: Record<string, TextEdit[]>; documentChanges?: Array<{ textDocument?: { uri: string; version?: number | null }; edits?: TextEdit[]; kind?: string }> }
export type StagedFile = { path: string; before: string; after: string; fingerprint: string; initial?: CodingFileState; savedContent: string | null }
export type StagedEdit = { workspace: WorkspaceRef; files: StagedFile[]; label: string; generation: number }
const undo: StagedEdit[] = []

export async function stageWorkspaceEdit(workspace: WorkspaceRef, edit: WorkspaceEdit, label: string, snapshot: Record<string, CodingFileState>, generation: number): Promise<StagedEdit> {
  const changes = new Map<string, TextEdit[]>()
  const add = (uri: string, edits: TextEdit[]) => {
    const absolute = uriToPath(uri)
    const target = absolute ? splitRootPath(absolute, [workspace.root]) : null
    if (!target || !target.path) throw new Error('The edit includes a file outside this workspace')
    changes.set(target.path, [...(changes.get(target.path) ?? []), ...edits])
  }
  for (const [uri, edits] of Object.entries(edit.changes ?? {})) add(uri, edits)
  for (const change of edit.documentChanges ?? []) {
    if (change.kind || !change.textDocument || !change.edits) throw new Error('This action requires file creation, rename or deletion, which is not supported yet')
    add(change.textDocument.uri, change.edits)
  }
  if (changes.size > 100) throw new Error('This edit affects more than 100 files; narrow its scope')
  const files: StagedFile[] = []
  let size = 0
  for (const [path, edits] of changes) {
    const initial = snapshot[fileKey(workspace.root, path)]
    if (initial?.saving || initial?.loading || initial?.conflict) throw new Error(`Resolve the pending operation or conflict in ${path} first`)
    const read = initial?.content != null ? initial : await codingRequest<Omit<FileReadPush, 'k'>>(workspace, { op: 'read', path })
    if (read.error || read.binary || read.tooLarge || read.content == null) throw new Error(read.error ?? `Cannot edit ${path} as text`)
    const after = applyWorkspaceTextEdits(read.content, edits)
    size += read.content.length + after.length
    if (size > 20 * 1024 * 1024) throw new Error('Workspace edit exceeds the 20 MiB preview limit')
    if (after !== read.content) files.push({ path, before: read.content, after, fingerprint: read.fingerprint, savedContent: initial?.savedContent ?? read.content, initial })
  }
  return { workspace, files, label, generation }
}
function assertCurrent(edit: StagedEdit, reverse = false) {
  const state = useKoma.getState()
  if ((state.remoteState.hostId ?? 'local') !== edit.workspace.hostId || state.coding._sessionGen !== edit.generation) throw new Error('The workspace changed. Generate a new preview.')
  for (const file of edit.files) {
    const current = state.coding.files[fileKey(edit.workspace.root, file.path)]
    if (current?.saving || current?.loading || current?.conflict) throw new Error(`Resolve the pending operation in ${file.path} first`)
    if (reverse ? current?.content !== file.after : current !== file.initial) throw new Error(`${file.path} changed after this preview. Generate a new preview.`)
  }
}
export async function applyStagedEdit(edit: StagedEdit) {
  assertCurrent(edit)
  // Verify disk for closed documents, and retain a durable inverse before
  // changing any buffer. Fail the entire operation on any preflight error.
  for (const file of edit.files) {
    const disk = await codingRequest<Omit<FileReadPush, 'k'>>(edit.workspace, { op: 'read', path: file.path })
    if (disk.error || disk.fingerprint !== file.fingerprint) throw new Error(`${file.path} changed on disk; no buffers were changed`)
    await checkpointCodingDocument(edit.workspace, file.path, file.before, `Before ${edit.label}`)
  }
  assertCurrent(edit)
  useKoma.setState(s => {
    const files = { ...s.coding.files }
    for (const file of edit.files) files[fileKey(edit.workspace.root, file.path)] = {
      ...(file.initial ?? emptyFileState()), content: file.after, savedContent: file.savedContent,
      fingerprint: file.fingerprint, dirty: file.after !== file.savedContent, manualSaveRequired: true, loading: false, error: null,
    }
    return { coding: { ...s.coding, files } }
  })
  undo.push(edit)
  while (undo.length > 10) undo.shift()
  for (const file of edit.files) {
    backupCodingDocument(edit.workspace, file.path, { content: file.after, savedContent: file.savedContent, fingerprint: file.fingerprint })
    useKoma.getState().openCodingFile(edit.workspace.root, file.path)
  }
}
export function undoWorkspaceEdit() {
  const edit = undo[undo.length - 1]
  if (!edit) throw new Error('No workspace edit to undo')
  assertCurrent(edit, true)
  useKoma.setState(s => {
    const files = { ...s.coding.files }
    for (const file of edit.files) {
      const key = fileKey(edit.workspace.root, file.path)
      files[key] = { ...files[key], content: file.before, dirty: file.before !== files[key].savedContent, manualSaveRequired: true }
    }
    return { coding: { ...s.coding, files } }
  })
  undo.pop()
  for (const file of edit.files) {
    const state = useKoma.getState().coding.files[fileKey(edit.workspace.root, file.path)]
    if (state.dirty) backupCodingDocument(edit.workspace, file.path, { content: file.before, savedContent: state.savedContent, fingerprint: state.fingerprint })
    else forgetCodingDraft(edit.workspace, file.path)
  }
}
