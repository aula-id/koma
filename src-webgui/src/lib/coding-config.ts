import { codingRequest, workspaceKey, type CodingConfig, type WorkspaceRef } from './coding-service'

type Entry = { promise: Promise<CodingConfig>; expires: number }
const cache = new Map<string, Entry>()
export function getCodingConfig(workspace: WorkspaceRef): Promise<CodingConfig> {
  const key = workspaceKey(workspace)
  const existing = cache.get(key)
  if (existing && existing.expires > Date.now()) return existing.promise
  const promise = codingRequest<CodingConfig>(workspace, { op: 'configRead' })
  const entry = { promise, expires: Date.now() + 5000 }
  cache.set(key, entry)
  void promise.catch(() => { if (cache.get(key) === entry) cache.delete(key) })
  return promise
}
export function invalidateCodingConfig(workspace: WorkspaceRef) {
  cache.delete(workspaceKey(workspace))
  window.dispatchEvent(new CustomEvent('koma-coding-config', { detail: workspace }))
}
export function editorPreferences(config: CodingConfig, language: string): Record<string, unknown> {
  const override = config.languages?.[language]
  const editor = override && typeof override === 'object' && 'editor' in override ? override.editor : null
  return { ...config.editor, ...(editor && typeof editor === 'object' && !Array.isArray(editor) ? editor : {}) }
}
export type CodingSnippet = { prefix: string; body: string; description?: string }
export function codingSnippets(config: CodingConfig, language: string): CodingSnippet[] {
  const values = [config.snippets?.['*'], config.snippets?.[language]].flatMap(v => Array.isArray(v) ? v : [])
  return values.flatMap(value => {
    if (!value || typeof value !== 'object' || typeof value.prefix !== 'string' || !value.prefix) return []
    const body = typeof value.body === 'string' ? value.body : Array.isArray(value.body) && value.body.every((line: unknown) => typeof line === 'string') ? value.body.join('\n') : null
    return body == null ? [] : [{ prefix: value.prefix, body, description: typeof value.description === 'string' ? value.description : undefined }]
  })
}
