import { useEffect, useRef, useState } from 'react'
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import { X } from 'lucide-react'
import { useKoma, type CodingFileState } from '../store/koma'
import { type WorkspaceRef } from '../lib/coding-service'
import { queryCodingLanguage } from '../lib/coding-language'
import { applyStagedEdit, stageWorkspaceEdit, type WorkspaceEdit, type StagedEdit } from '../lib/coding-edits'
import { type EditPosition } from '../lib/workspace-edit-text'
import { initMonaco, langFromPath, readMonoFont } from '../lib/monaco-setup'
import { BrailleSpinner } from './BrailleSpinner'

export type RefactorContext = {
  workspace: WorkspaceRef; path: string; position: EditPosition; word?: string
  mode: 'rename' | 'edit'; edit?: WorkspaceEdit; label?: string
  snapshot: Record<string, CodingFileState>; generation: number
}
export function showCodingRefactor(context: RefactorContext) {
  window.dispatchEvent(new CustomEvent('koma-coding-refactor', { detail: context }))
}
export function CodingRefactor() {
  const [context, setContext] = useState<RefactorContext | null>(null)
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
      generation.current++; setContext(context); setStaged(null); setName(context.word ?? ''); setError(null); setBusy(false)
    }
    window.addEventListener('koma-coding-refactor', open)
    return () => window.removeEventListener('koma-coding-refactor', open)
  }, [])
  useEffect(() => { generation.current++; setContext(null) }, [host])
  const stage = async (context: RefactorContext, edit: WorkspaceEdit, label: string) => {
    const run = generation.current
    setBusy(true); setError(null)
    try {
      const staged = await stageWorkspaceEdit(context.workspace, edit, label, context.snapshot, context.generation)
      if (run !== generation.current) return
      setStaged(staged); setSelected(0)
      if (!staged.files.length) setError('The language server returned no changes.')
    } catch (e) { if (run === generation.current) setError(e instanceof Error ? e.message : String(e)) }
    finally { if (run === generation.current) setBusy(false) }
  }
  useEffect(() => {
    if (!context) return
    if (context.mode === 'edit' && context.edit) void stage(context, context.edit, context.label ?? 'Code action')
    else { input.current?.focus(); input.current?.select() }
  }, [context])
  useEffect(() => {
    const file = staged?.files[selected]
    if (!file || !view.current) return
    initMonaco()
    const original = monaco.editor.createModel(file.before, langFromPath(file.path))
    const modified = monaco.editor.createModel(file.after, langFromPath(file.path))
    const editor = monaco.editor.createDiffEditor(view.current, { readOnly: true, originalEditable: false, automaticLayout: true, minimap: { enabled: false }, fontFamily: readMonoFont(), fontSize: 12, scrollBeyondLastLine: false })
    editor.setModel({ original, modified })
    return () => { editor.dispose(); original.dispose(); modified.dispose() }
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
  const close = () => { generation.current++; setContext(null) }
  const apply = async () => {
    if (!staged) return
    setBusy(true); setError(null)
    try { await applyStagedEdit(staged); close() }
    catch (e) { setError(e instanceof Error ? e.message : String(e)) }
    finally { setBusy(false) }
  }
  if (!context) return null
  return <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/30 p-5" onMouseDown={() => { if (!busy) close() }}>
    <div role="dialog" aria-modal="true" aria-label="Preview Workspace Edit" className="flex h-[min(600px,85vh)] w-[min(1000px,95vw)] flex-col overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-xl" onMouseDown={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape' && !busy) close() }}>
      <div className="flex h-9 flex-none items-center gap-2 border-b border-koma-border px-3 text-[12px] text-koma-fg"><span className="flex-1">{context.mode === 'rename' ? 'Rename Symbol' : context.label ?? 'Code Action'} · Preview</span>{busy && <BrailleSpinner size={13}/>}<button disabled={busy} onClick={close} aria-label="Close" className="rounded p-1 text-koma-dim hover:bg-koma-hover"><X size={13}/></button></div>
      {context.mode === 'rename' && <form onSubmit={e => { e.preventDefault(); void rename() }} className="flex flex-none items-center gap-2 border-b border-koma-border px-3 py-2"><input ref={input} value={name} onChange={e => { setName(e.target.value); setStaged(null) }} disabled={busy} aria-label="New symbol name" className="min-w-0 flex-1 rounded border border-koma-border bg-transparent px-2 py-1 text-[12px] text-koma-fg outline-none"/><button disabled={busy || !name.trim()} className="rounded border border-koma-border px-2 py-1 text-[11px] text-koma-fg hover:bg-koma-hover disabled:opacity-40">Preview rename</button></form>}
      {error && <div className="border-b border-koma-border px-3 py-2 text-[12px] text-koma-error">{error}</div>}
      <div className="flex min-h-0 flex-1"><div className="w-52 flex-none overflow-y-auto border-r border-koma-border py-1">{staged?.files.map((f, i) => <button key={f.path} onClick={() => setSelected(i)} className={`w-full truncate px-3 py-2 text-left text-[11px] text-koma-fg ${i === selected ? 'bg-koma-hover' : 'hover:bg-koma-hover'}`} title={f.path}>{f.path}</button>)}</div><div ref={view} className="min-w-0 flex-1"/></div>
      <div className="flex flex-none items-center justify-between gap-2 border-t border-koma-border px-3 py-2 text-[11px] text-koma-dim"><span>{staged?.files.length ?? 0} files · Changes open as unsaved buffers</span><button disabled={busy || !staged?.files.length} onClick={() => void apply()} className="rounded border border-koma-border px-2 py-1 text-koma-fg hover:bg-koma-hover disabled:opacity-40">Apply to editors</button></div>
    </div>
  </div>
}
