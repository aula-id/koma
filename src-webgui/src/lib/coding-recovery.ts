import { codingRequest, codingWindowId, documentKey, type CodingBackup, type WorkspaceRef } from './coding-service'

type Draft = Pick<CodingBackup, 'content' | 'savedContent' | 'fingerprint'>
type PendingDraft = { workspace: WorkspaceRef; document: CodingBackup; timer: ReturnType<typeof setTimeout> }
const pending = new Map<string, PendingDraft>()
const revisions = new Map<string, number>()

function revision(key: string) {
  const next = (revisions.get(key) ?? 0) + 1
  revisions.set(key, next)
  return next
}
function report(error: unknown) {
  window.dispatchEvent(new CustomEvent('koma-coding-recovery-error', { detail: `Recovery: ${error instanceof Error ? error.message : String(error)}` }))
}
export function checkpointCodingDocument(workspace: WorkspaceRef, path: string, content: string, reason: string): Promise<unknown> {
  return codingRequest(workspace, { op: 'checkpoint', path, content, reason })
}
export function recordCodingHistory(workspace: WorkspaceRef, path: string, content: string, reason: string) {
  void checkpointCodingDocument(workspace, path, content, reason).catch(report)
}
function flush(key: string) {
  const entry = pending.get(key)
  if (!entry) return
  pending.delete(key)
  clearTimeout(entry.timer)
  void codingRequest(entry.workspace, { op: 'backup', document: entry.document }).catch(report)
}
export function flushCodingRecovery() {
  for (const key of pending.keys()) flush(key)
}

/** Snapshot identity now, so later host/session changes cannot retarget a draft. */
export function backupCodingDocument(workspace: WorkspaceRef, path: string, draft: Draft) {
  const key = documentKey(workspace, path)
  const previous = pending.get(key)
  const document: CodingBackup = { ...draft, path, windowId: codingWindowId, revision: revision(key) }
  // Throttle (rather than debounce): continuous typing still reaches durable storage.
  if (previous) { previous.document = document; return }
  const timer = setTimeout(() => flush(key), 300)
  pending.set(key, { workspace: { ...workspace }, document, timer })
}
export function forgetCodingDraft(workspace: WorkspaceRef, path: string) {
  const key = documentKey(workspace, path)
  const previous = pending.get(key)
  if (previous) { clearTimeout(previous.timer); pending.delete(key) }
  void codingRequest(workspace, { op: 'forgetBackup', path, windowId: codingWindowId, revision: revision(key) }).catch(report)
}
