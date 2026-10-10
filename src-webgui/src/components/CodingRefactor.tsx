import { showToast } from '../lib/toast'
import { useEffect, useRef, useState } from 'react'
import { X } from 'lucide-react'
import { useKoma, fileKey, type CodingFileState } from '../store/koma'
import { codingRequest, type WorkspaceRef } from '../lib/coding-service'
import { queryCodingLanguage } from '../lib/coding-language'
import { applyStagedEdit, stageWorkspaceEdit, type WorkspaceEdit, type StagedEdit } from '../lib/coding-edits'
import { type EditPosition } from '../lib/workspace-edit-text'
import { BrailleSpinner } from './BrailleSpinner'

export type RefactorContext = {
  workspace: WorkspaceRef; path: string; position: EditPosition; word?: string
  mode: 'rename' | 'edit' | 'prepared' | 'action'; action?: { title: string; edit?: WorkspaceEdit; command?: unknown; data?: unknown }; ticket?: string; edit?: WorkspaceEdit; staged?: StagedEdit; label?: string
  snapshot: Record<string, CodingFileState>; generation: number
}
export function showCodingRefactor(context: RefactorContext) {
  window.dispatchEvent(new CustomEvent('koma-coding-refactor', { detail: context }))
}
export function CodingRefactor() {
  const notificationSession = useKoma(state => state.session.id)
  const [context, setContext] = useState<RefactorContext | null>(null)
  const [command, setCommand] = useState<{ command: string; arguments?: unknown[] } | null>(null)
  const contextRef = useRef<RefactorContext | null>(null)
  contextRef.current = context
  const [name, setName] = useState('')
  const [staged, setStaged] = useState<StagedEdit | null>(null)
  const [selected, setSelected] = useState(0)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const view = useRef<HTMLDivElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const generation = useRef(0)
  const host = useKoma(s => s.remoteState.hostId ?? 'local')
  useEffect(() => {
    const open = (event: Event) => {
      const context = (event as CustomEvent<RefactorContext>).detail
      const previous = contextRef.current
      if (previous?.ticket) void codingRequest(previous.workspace, { op: 'lspEditReply', ticket: previous.ticket, applied: false, reason: 'Preview replaced' }).catch(() => {})
      generation.current++; setCommand(null); setContext(context); setStaged(null); setName(context.word ?? ''); setError(null); setBusy(false)
    }
    const serverEdit = async (event: Event) => {
      const { workspace, ticket } = (event as CustomEvent<{ workspace: WorkspaceRef; ticket: string }>).detail
      try {
        const value = await codingRequest<{ edit: WorkspaceEdit; label?: string }>(workspace, { op: 'lspEditPreview', ticket })
        const state = useKoma.getState()
        if ((state.remoteState.hostId ?? 'local') !== workspace.hostId || contextRef.current) throw new Error('Another preview or workspace is active')
        showCodingRefactor({ workspace, ticket, path: '', position: { line: 0, character: 0 }, mode: 'edit', edit: value.edit, label: value.label ?? 'Language server edit', snapshot: state.coding.files, generation: state.coding._sessionGen })
      } catch (e) { void codingRequest(workspace, { op: 'lspEditReply', ticket, applied: false, reason: String(e) }).catch(() => {}) }
    }
    window.addEventListener('koma-coding-refactor', open)
    window.addEventListener('koma-lsp-edit', serverEdit)
    return () => { window.removeEventListener('koma-coding-refactor', open); window.removeEventListener('koma-lsp-edit', serverEdit) }
  }, [])
  useEffect(() => { generation.current++; const current = contextRef.current; if (current?.ticket) void codingRequest(current.workspace, { op: 'lspEditReply', ticket: current.ticket, applied: false, reason: 'Host changed' }).catch(() => {}); setContext(null) }, [host])
  const stage = async (context: RefactorContext, edit: WorkspaceEdit, label: string) => {
    const run = generation.current
    setBusy(true); setError(null)
    try {
      const staged = await stageWorkspaceEdit(context.workspace, edit, label, context.snapshot, context.generation)
      if (run !== generation.current) return false
      setStaged(staged); setSelected(0)
      if (!staged.files.length) setError('The language server returned no changes.')
      return true
    } catch (e) { if (run === generation.current) setError(e instanceof Error ? e.message : String(e)); return false }
    finally { if (run === generation.current) setBusy(false) }
  }
  useEffect(() => {
    if (!context) return
    if (context.mode === 'action' && context.action) {
      const epoch = generation.current
      setBusy(true)
      void (async () => {
        let action = context.action!
        if (action.data != null) {
          try { action = await queryCodingLanguage(context.workspace, context.path, 'codeAction/resolve', action) }
          catch (error) { if ((!action.edit && !action.command) || !String(error).includes('does not resolve code actions')) throw error }
        }
        if (epoch !== generation.current) return
        const value = typeof action.command === 'string' ? { command: action.command, arguments: (action as unknown as { arguments?: unknown[] }).arguments } : action.command
        if (action.edit && !await stage(context, action.edit, action.title)) return
        if (!action.edit && !value) throw new Error('The language server returned no executable action')
        if (epoch === generation.current && value && typeof value === 'object' && 'command' in value && typeof value.command === 'string') setCommand(value as { command: string; arguments?: unknown[] })
      })().catch(error => { if (epoch === generation.current) setError(String(error)) }).finally(() => { if (epoch === generation.current) setBusy(false) })
    } else if (context.mode === 'prepared' && context.staged) { setStaged(context.staged); setSelected(0) }
    else if (context.mode === 'edit' && context.edit) void stage(context, context.edit, context.label ?? 'Code action')
    else { input.current?.focus(); input.current?.select() }
  }, [context])
  useEffect(() => {
    const file = staged?.files[selected]
    if (!file || !view.current) return
    const element = view.current
    let stopped = false
    let dispose: (() => void) | undefined
    // Keep Monaco out of the startup bundle; comparison editors load on demand.
    void Promise.all([import('monaco-editor/esm/vs/editor/editor.api'), import('../lib/monaco-setup')]).then(([monaco, { initMonaco, langFromPath, readMonoFont, applyKomaTheme }]) => {
      if (stopped) return
      initMonaco()
      monaco.editor.setTheme(applyKomaTheme())
      const original = monaco.editor.createModel(file.before, langFromPath(file.path))
      const modified = monaco.editor.createModel(file.after, langFromPath(file.path))
      const editor = monaco.editor.createDiffEditor(element, { readOnly: true, originalEditable: false, automaticLayout: true, minimap: { enabled: false }, fontFamily: readMonoFont(), fontSize: 12, scrollBeyondLastLine: false })
      editor.setModel({ original, modified })
      dispose = () => { editor.dispose(); original.dispose(); modified.dispose() }
    }).catch(error => { if (!stopped) setError(error instanceof Error ? error.message : String(error)) })
    return () => { stopped = true; dispose?.() }
  }, [staged, selected])
  const rename = async () => {
    if (!context || !name.trim()) return
    const run = generation.current
    setBusy(true); setError(null)
    try {
      const edit = await queryCodingLanguage<WorkspaceEdit | null>(context.workspace, context.path, 'textDocument/rename', { position: context.position, newName: name.trim() })
      if (run !== generation.current) return
      if (!edit) throw new Error('The symbol cannot be renamed at this position.')
      await stage(context, edit, 'Rename symbol')
    } catch (e) { if (run === generation.current) setError(e instanceof Error ? e.message : String(e)) }
    finally { if (run === generation.current) setBusy(false) }
  }
  const close = (applied = false) => { generation.current++; contextRef.current = null; if (context?.ticket && !applied) void codingRequest(context.workspace, { op: 'lspEditReply', ticket: context.ticket, applied: false, reason: 'User canceled the preview' }).catch(() => {}); setContext(null) }
  const apply = async () => {
    if (!context || (!staged && !command)) return
    setBusy(true); setError(null)
    try {
      const state = useKoma.getState()
      if ((state.remoteState.hostId ?? 'local') !== context.workspace.hostId || state.coding._sessionGen !== context.generation || (context.path && state.coding.files[fileKey(context.workspace.root, context.path)] !== context.snapshot[fileKey(context.workspace.root, context.path)])) throw new Error('The originating document changed; request a new code action')
      if (context.ticket) await codingRequest(context.workspace, { op: 'lspEditPreview', ticket: context.ticket })
      if (staged?.files.length) await applyStagedEdit(staged)
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
      const { flushPendingLspDidChange } = await import('../lib/monaco-lsp')
      for (const file of staged?.files ?? []) if (file.existsAfter !== false) await flushPendingLspDidChange(context.workspace.root, file.path)
      if (context.ticket) await codingRequest(context.workspace, { op: 'lspEditReply', ticket: context.ticket, applied: true })
      close(true)
      if (command) {
        try { await codingRequest(context.workspace, { op: 'lspCommand', path: context.path, params: command }) }
        catch (error) { showToast(String(error), 'error', { session: notificationSession, source: 'coding' }) }
      }
    }
    catch (e) { setError(e instanceof Error ? e.message : String(e)) }
    finally { setBusy(false) }
  }
  if (!context) return null
  return <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/30 p-5" onMouseDown={() => { if (!busy) close() }}>
    <div role="dialog" aria-modal="true" aria-label="Preview Workspace Edit" className="flex h-[min(600px,85vh)] w-[min(1000px,95vw)] flex-col overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-xl" onMouseDown={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape' && !busy) close() }}>
      <div className="flex h-9 flex-none items-center gap-2 border-b border-koma-border px-3 text-[12px] text-koma-fg"><span className="flex-1">{context.mode === 'rename' ? 'Rename Symbol' : context.label ?? 'Code Action'} · Preview</span>{busy && <BrailleSpinner size={13}/>}<button disabled={busy} onClick={() => close()} aria-label="Close" className="rounded p-1 text-koma-dim hover:bg-koma-hover"><X size={13}/></button></div>
      {context.mode === 'rename' && <form onSubmit={e => { e.preventDefault(); void rename() }} className="flex flex-none items-center gap-2 border-b border-koma-border px-3 py-2"><input ref={input} value={name} onChange={e => { setName(e.target.value); setStaged(null) }} disabled={busy} aria-label="New symbol name" className="min-w-0 flex-1 rounded border border-koma-border bg-transparent px-2 py-1 text-[12px] text-koma-fg outline-none"/><button disabled={busy || !name.trim()} className="rounded border border-koma-border px-2 py-1 text-[11px] text-koma-fg hover:bg-koma-hover disabled:opacity-40">Preview rename</button></form>}
      {error && <div className="border-b border-koma-border px-3 py-2 text-[12px] text-koma-error">{error}</div>}
      {command && <div className="border-b border-koma-border px-3 py-2 text-[11px] text-koma-dim">Run language server command: {command.command}<pre className="max-h-20 overflow-auto whitespace-pre-wrap break-all">{JSON.stringify(command.arguments ?? [])}</pre></div>}
      <div className="flex min-h-0 flex-1"><div className="w-52 flex-none overflow-y-auto border-r border-koma-border py-1">{staged?.files.map((f, i) => <button key={f.path} onClick={() => setSelected(i)} className={`w-full truncate px-3 py-2 text-left text-[11px] text-koma-fg ${i === selected ? 'bg-koma-hover' : 'hover:bg-koma-hover'}`} title={f.path}>{staged.resources ? f.existed ? f.existsAfter ? 'Edit · ' : 'Delete · ' : 'Create · ' : ''}{f.path}</button>)}</div><div ref={view} className="min-w-0 flex-1"/></div>
      <div className="flex flex-none items-center justify-between gap-2 border-t border-koma-border px-3 py-2 text-[11px] text-koma-dim"><span>{staged?.files.length ?? 0} files · {staged?.resources ? 'Applies file changes to disk; Undo is available' : 'Changes open as unsaved buffers'}</span><button disabled={busy || (!staged?.files.length && !command)} onClick={() => void apply()} className="rounded border border-koma-border px-2 py-1 text-koma-fg hover:bg-koma-hover disabled:opacity-40">{command ? staged?.files.length ? 'Apply and run command' : 'Run command' : staged?.resources ? 'Apply file changes' : 'Apply to editors'}</button></div>
    </div>
  </div>
}
