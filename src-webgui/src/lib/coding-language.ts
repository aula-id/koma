import { codingRequest, workspaceKey, type WorkspaceRef } from './coding-service'

const lanes = new Map<string, Promise<void>>()
/** Preserve document notification order and expose an ACK barrier to queries. */
export function sendCodingLanguage(workspace: WorkspaceRef, body: Record<string, unknown>): Promise<void> {
  const key = workspaceKey(workspace)
  const next = (lanes.get(key) ?? Promise.resolve()).catch(() => {}).then(async () => {
    await codingRequest(workspace, { op: 'lsp', body })
  })
  lanes.set(key, next)
  void next.finally(() => { if (lanes.get(key) === next) lanes.delete(key) }).catch(() => {})
  return next
}
export async function queryCodingLanguage<T>(workspace: WorkspaceRef, path: string, method: string, params: Record<string, unknown>): Promise<T> {
  await lanes.get(workspaceKey(workspace))
  return codingRequest<T>(workspace, { op: 'lspQuery', path, method, params })
}
