import { useEffect, useRef, useState } from 'react'
import { History, RotateCcw, X } from 'lucide-react'
import { useKoma, fileKey } from '../store/koma'
import { codingRequest, type WorkspaceRef } from '../lib/coding-service'

type Journal = { id: string; state: string; files: number }
type Preview = { id: string; state: string; expected: Record<string, string>; files: { path: string; current: string | null; original: string | null; conflict: boolean }[] }
export function showCodingTransactions() { window.dispatchEvent(new Event('koma-coding-transactions')) }
export function CodingTransactions() {
  const [workspace, setWorkspace] = useState<WorkspaceRef | null>(null)
  const [journals, setJournals] = useState<Journal[]>([])
  const [preview, setPreview] = useState<Preview | null>(null)
  const [path, setPath] = useState(''), [error, setError] = useState(''), [busy, setBusy] = useState(false)
  const epoch = useRef(0), container = useRef<HTMLDivElement>(null), focus = useRef<HTMLElement | null>(null)
  const host = useKoma(s => s.remoteState.hostId ?? 'local')
  const close = () => { epoch.current++; setWorkspace(null); focus.current?.focus() }
  useEffect(() => { close() }, [host])
  useEffect(() => {
    const show = () => { const s = useKoma.getState(), root = s.coding.activeRoot ?? s.settingsValues?.workdir?.[0]; if (!root) return; focus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; setWorkspace({ hostId: s.remoteState.hostId ?? 'local', root }); setPreview(null); setError(''); setJournals([]); epoch.current++ }
    window.addEventListener('koma-coding-transactions', show)
    return () => window.removeEventListener('koma-coding-transactions', show)
  }, [])
  useEffect(() => { if (!workspace) return; const abort = new AbortController(); void codingRequest<Journal[]>(workspace, { op: 'resourceJournals' }, abort.signal).then(setJournals).catch(e => { if (!abort.signal.aborted) setError(String(e)) }); return () => abort.abort() }, [workspace])
  const selected = preview?.files.find(f => f.path === path)
  useEffect(() => {
    if (!selected || !container.current) return
    let canceled = false, dispose: (() => void) | undefined
    const element = container.current
    void Promise.all([import('monaco-editor/esm/vs/editor/editor.api'), import('../lib/monaco-setup')]).then(([monaco, setup]) => {
      if (canceled) return
      setup.initMonaco(); monaco.editor.setTheme(setup.applyKomaTheme())
      const before = monaco.editor.createModel(selected.current ?? '', setup.langFromPath(path)), after = monaco.editor.createModel(selected.original ?? '', setup.langFromPath(path))
      const editor = monaco.editor.createDiffEditor(element, { readOnly: true, originalEditable: false, automaticLayout: true, minimap: { enabled: false }, fontFamily: setup.readMonoFont(), fontSize: 12 })
      editor.setModel({ original: before, modified: after }); dispose = () => { editor.dispose(); before.dispose(); after.dispose() }
    }).catch(e => { if (!canceled) setError(String(e)) })
    return () => { canceled = true; dispose?.() }
  }, [selected, path])
  const inspect = async (id: string) => { if (!workspace || busy) return; const ticket = ++epoch.current; setBusy(true); setError(''); setPreview(null); try { const value = await codingRequest<Preview>(workspace, { op: 'resourceRecoveryPreview', transactionId: id }); if (ticket === epoch.current) { setPreview(value); setPath(value.files[0]?.path ?? '') } } catch (e) { if (ticket === epoch.current) setError(String(e)) } finally { setBusy(false) } }
  const restore = async () => {
    if (!workspace || !preview || busy) return
    const ticket = epoch.current; setBusy(true); setError('')
    try {
      const state = useKoma.getState()
      for (const file of preview.files) { const current = state.coding.files[fileKey(workspace.root, file.path)]; if (current && (current.dirty || current.saving || current.loading)) throw new Error(`Save or discard ${file.path} before recovery`) }
      await codingRequest(workspace, { op: 'resourceRecover', transactionId: preview.id, expected: preview.expected })
      window.dispatchEvent(new CustomEvent('koma-coding-disk', { detail: workspace }))
      if (ticket === epoch.current) { setJournals(rows => rows.filter(row => row.id !== preview.id)); setPreview(null) }
    } catch (e) { if (ticket === epoch.current) setError(String(e)) } finally { setBusy(false) }
  }
  if (!workspace) return null
  return <section role="dialog" aria-label="Recover workspace edits" className="absolute inset-10 z-50 flex flex-col overflow-hidden rounded-md border border-koma-border bg-koma-panel text-[11px] text-koma-fg shadow-xl" onKeyDown={e => { if (e.key === 'Escape') close() }}>
    <div className="flex h-9 items-center gap-2 border-b border-koma-border px-3"><History size={13} /><span className="flex-1 truncate">RECOVER WORKSPACE EDITS · {workspace.root}</span><button autoFocus aria-label="Close" onClick={close} className="rounded p-1 text-koma-dim hover:bg-koma-hover"><X size={13} /></button></div>
    {error && <p role="alert" className="px-3 py-2 text-koma-error">{error}</p>}
    <div className="flex min-h-0 flex-1"><div className="w-56 overflow-auto border-r border-koma-border p-2">{journals.map(row => <button key={row.id} disabled={busy} onClick={() => void inspect(row.id)} className={`mb-1 block w-full rounded p-2 text-left hover:bg-koma-hover ${preview?.id === row.id ? 'bg-koma-hover' : ''}`}><span>{row.state} · {row.files} files</span><span className="block truncate text-koma-dim">{row.id}</span></button>)}{!journals.length && <p className="text-koma-dim">No retained transactions.</p>}{preview?.files.map(file => <button key={file.path} onClick={() => setPath(file.path)} className="block w-full truncate p-1 text-left hover:bg-koma-hover">{file.conflict ? 'Conflict · ' : ''}{file.path}</button>)}</div><div className="flex min-w-0 flex-1 flex-col"><p className="px-3 py-1 text-koma-dim">Current disk → Before workspace edit{selected?.original === null ? ' · file will be removed' : ''}</p><div ref={container} className="min-h-0 flex-1" /></div></div>
    <div className="flex justify-end border-t border-koma-border p-2"><button disabled={busy || !preview || preview.files.some(f => f.conflict)} onClick={() => void restore()} className="flex items-center gap-1 rounded border border-koma-border px-2 py-1 hover:bg-koma-hover disabled:opacity-40"><RotateCcw size={12} />Restore transaction</button></div>
  </section>
}
