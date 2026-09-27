import type * as Monaco from 'monaco-editor/esm/vs/editor/editor.api'
import { codingRequest, workspaceKey, type WorkspaceRef } from './coding-service'
export type Breakpoint = { line: number; condition?: string; hitCondition?: string; logMessage?: string }
export type Breakpoints = Record<string, Breakpoint[]>
export type DebugSession = { id: string; label: string; request: string; status: string; error: string | null; generation: number; capabilities: Record<string, unknown>; breakpoints: Record<string, { breakpoints?: { verified: boolean; line?: number; message?: string }[] }> }
const cache = new Map<string, Breakpoints>()
const key = (w: WorkspaceRef) => `koma.debug.breakpoints.${workspaceKey(w)}`
export function breakpoints(w: WorkspaceRef): Breakpoints {
  const k = key(w); const previous = cache.get(k); if (previous) return previous
  try { const value: unknown = JSON.parse(localStorage.getItem(k) ?? '{}'); if (value && typeof value === 'object' && !Array.isArray(value)) { const valid = Object.fromEntries(Object.entries(value).filter(([, v]) => Array.isArray(v)).map(([p, v]) => [p, (v as Breakpoint[]).filter(b => Number.isInteger(b?.line) && b.line > 0).slice(0, 500)])); cache.set(k, valid); return valid } } catch { /* Empty on corrupt preferences. */ }
  cache.set(k, {}); return cache.get(k)!
}
export function setBreakpoints(w: WorkspaceRef, path: string, points: Breakpoint[]) {
  const next = { ...breakpoints(w), [path]: points }; cache.set(key(w), next)
  try { localStorage.setItem(key(w), JSON.stringify(next)) } catch { /* In-memory remains usable. */ }
  window.dispatchEvent(new CustomEvent('koma-breakpoints', { detail: { workspace: w, path, points } }))
}
export function bindDebugEditor(monaco: typeof Monaco, editor: Monaco.editor.IStandaloneCodeEditor, w: WorkspaceRef, path: string) {
  editor.updateOptions({ glyphMargin: true })
  const markers = editor.createDecorationsCollection()
  const paint = () => markers.set((breakpoints(w)[path] ?? []).map(p => ({ range: new monaco.Range(p.line, 1, p.line, 1), options: { isWholeLine: true, glyphMarginClassName: 'koma-breakpoint', glyphMarginHoverMessage: { value: p.logMessage ? `Log: ${p.logMessage}` : p.condition ? `Breakpoint: ${p.condition}` : 'Breakpoint' }, stickiness: monaco.editor.TrackedRangeStickiness.NeverGrowsWhenTypingAtEdges } })))
  paint()
  const change = (e: Event) => { const d = (e as CustomEvent<{ workspace: WorkspaceRef; path: string }>).detail; if (workspaceKey(d.workspace) === workspaceKey(w) && d.path === path) paint() }
  window.addEventListener('koma-breakpoints', change)
  const toggle = (line: number) => { const points = breakpoints(w)[path] ?? []; setBreakpoints(w, path, points.some(p => p.line === line) ? points.filter(p => p.line !== line) : [...points, { line }].sort((a, b) => a.line - b.line)) }
  const mouse = editor.onMouseDown(e => { if (e.target.type === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN && e.target.position) toggle(e.target.position.lineNumber) })
  const action = editor.addAction({ id: 'koma.toggleBreakpoint', label: 'Toggle Breakpoint', keybindings: [monaco.KeyCode.F9], run: () => { const p = editor.getPosition(); if (p) toggle(p.lineNumber) } })
  return () => { action.dispose(); mouse.dispose(); markers.clear(); window.removeEventListener('koma-breakpoints', change) }
}
export async function updateRunningBreakpoints(workspace: WorkspaceRef, path: string, points: Breakpoint[]) {
  const sessions = await codingRequest<DebugSession[]>(workspace, { op: 'debugSessions' })
  await Promise.all(sessions.filter(s => ['running', 'stopped'].includes(s.status)).map(s => codingRequest(workspace, { op: 'debugRequest', sessionId: s.id, command: 'setBreakpoints', arguments: { path, breakpoints: points } })))
}
