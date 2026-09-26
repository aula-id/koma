// Native coding RPC is independent of the foreground chat session. Always bind
// workspace identity before dispatch; never infer it when a reply arrives.
export type WorkspaceRef = { hostId: string; root: string }
export type CodingReply = {
  k: 'CodingReply'
  id: string
  workspace: WorkspaceRef
  result?: unknown
  error?: string
}
export type CodingOperation =
  | { op: 'hello' | 'configRead' | 'backups' }
  | { op: 'paths'; query: string }
  | { op: 'replacePreview'; options: { query: string; replacement: string; caseSensitive: boolean; wholeWord: boolean; isRegex: boolean; includeGlob: string | null; excludeGlob: string | null } }
  | { op: 'lsp'; body: Record<string, unknown> }
  | { op: 'lspQuery'; path: string; method: string; params: Record<string, unknown> }
  | { op: 'read'; path: string }
  | { op: 'inspect'; paths: string[] }
  | { op: 'save'; path: string; content: string; fingerprint: string }
  | { op: 'configWrite'; config: CodingConfig }
  | { op: 'backup'; document: CodingBackup }
  | { op: 'backupRead'; windowId: string; path: string; revision: number }
  | { op: 'forgetBackup'; windowId: string; path: string; revision: number }
  | { op: 'checkpoint'; path: string; content: string; reason: string }
  | { op: 'history'; path: string }
  | { op: 'historyRead'; checkpoint: number }

export type CodingConfig = {
  version: 1
  editor?: Record<string, unknown>
  languages?: Record<string, unknown>
  toolchains?: Record<string, unknown>
  environment?: Record<string, string>
  snippets?: Record<string, unknown>
  tasks?: unknown[]
  debug?: unknown[]
  tests?: unknown[]
}
export type CodingBackup = {
  windowId: string
  path: string
  revision: number
  content: string
  savedContent: string | null
  fingerprint: string
  view?: { discarded?: boolean; [key: string]: unknown }
  updated?: number
}
type Pending = {
  workspace: WorkspaceRef
  resolve: (value: unknown) => void
  reject: (error: Error) => void
  cleanup: () => void
}
const pending = new Map<string, Pending>()
let sequence = 0
const windowId = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`
export const codingWindowId = windowId

export function workspaceKey(workspace: WorkspaceRef): string {
  return JSON.stringify([workspace.hostId, workspace.root])
}
export function documentKey(workspace: WorkspaceRef, path: string): string {
  return JSON.stringify([workspace.hostId, workspace.root, path])
}

export function codingRequest<T>(workspace: WorkspaceRef, operation: CodingOperation, signal?: AbortSignal): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    if (signal?.aborted) { reject(new Error('Canceled')); return }
    const id = `coding-${windowId}-${++sequence}`
    const finishError = (error: Error) => {
      const entry = pending.get(id)
      if (!entry) return
      pending.delete(id)
      entry.cleanup()
      reject(error)
    }
    const abort = () => finishError(new Error('Canceled'))
    const timer = setTimeout(() => finishError(new Error('Coding operation timed out; check its outcome before retrying')), 30_000)
    pending.set(id, {
      workspace: { ...workspace }, resolve: (value) => resolve(value as T), reject,
      cleanup: () => { clearTimeout(timer); signal?.removeEventListener('abort', abort) },
    })
    signal?.addEventListener('abort', abort, { once: true })
    try {
      if (!window.ipc?.postMessage) throw new Error('Native coding service is unavailable')
      window.ipc.postMessage(JSON.stringify({ t: 'coding', request: { id, workspace, ...operation } }))
    } catch (error) {
      finishError(error instanceof Error ? error : new Error(String(error)))
    }
  })
}

export function resolveCodingReply(reply: CodingReply): void {
  const entry = pending.get(reply.id)
  if (!entry || workspaceKey(entry.workspace) !== workspaceKey(reply.workspace)) return
  pending.delete(reply.id)
  entry.cleanup()
  if (reply.error) entry.reject(new Error(reply.error))
  else entry.resolve(reply.result)
}
