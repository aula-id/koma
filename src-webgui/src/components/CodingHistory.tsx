import { useEffect, useRef, useState } from 'react'
import { History, RotateCcw, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { emptyFileState, fileKey, type FileReadPush } from '../store/coding'
import { codingRequest, codingWindowId, type CodingBackup, type WorkspaceRef } from '../lib/coding-service'
import { backupCodingDocument, checkpointCodingDocument, flushCodingRecovery, forgetCodingDraft } from '../lib/coding-recovery'
import { BrailleSpinner } from './BrailleSpinner'

type Context = { workspace: WorkspaceRef; path?: string; disk?: boolean }
type Entry = { id?: number; path: string; reason?: string; created?: number; updated?: number; windowId?: string; revision?: number }
export function showCodingHistory(root?: string, path?: string, disk = false) {
  const state = useKoma.getState()
  root ??= state.coding.activeRoot ?? state.settingsValues?.workdir?.[0]
  if (root) window.dispatchEvent(new CustomEvent('koma-coding-history', { detail: { workspace: { hostId: state.remoteState.hostId ?? 'local', root }, path, disk } }))
}

export function CodingHistory() {
  const [context, setContext] = useState<Context | null>(null)
  const [entries, setEntries] = useState<Entry[]>([])
  const [selected, setSelected] = useState<Entry | null>(null)
  const [snapshot, setSnapshot] = useState<{ content: string; backup?: CodingBackup; baseline: string; fingerprint?: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [reload, setReload] = useState(0)
  const [discardArmed, setDiscardArmed] = useState(false)
  const [truncated, setTruncated] = useState(false)
  const container = useRef<HTMLDivElement>(null)
  const closeButton = useRef<HTMLButtonElement>(null)
  const restoreFocus = useRef<HTMLElement | null>(null)
  const hostId = useKoma(s => s.remoteState.hostId ?? 'local')
  const close = () => { setContext(null); requestAnimationFrame(() => restoreFocus.current?.focus()) }
  useEffect(() => {
    const open = (event: Event) => {
      restoreFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
      setContext((event as CustomEvent<Context>).detail)
    }
    const error = (event: Event) => useKoma.setState(s => {
      const id = s.ui.toastSeq + 1
      return { ui: { ...s.ui, toastSeq: id, toast: { id, kind: 'error', text: (event as CustomEvent<string>).detail } } }
    })
    const visibility = () => { if (document.visibilityState === 'hidden') flushCodingRecovery() }
    window.addEventListener('koma-coding-history', open)
    window.addEventListener('koma-coding-recovery-error', error)
    window.addEventListener('beforeunload', flushCodingRecovery)
    document.addEventListener('visibilitychange', visibility)
    return () => {
      window.removeEventListener('koma-coding-history', open)
      window.removeEventListener('koma-coding-recovery-error', error)
      window.removeEventListener('beforeunload', flushCodingRecovery)
      document.removeEventListener('visibilitychange', visibility)
      flushCodingRecovery()
    }
  }, [])
  useEffect(() => { setContext(null) }, [hostId])
  useEffect(() => {
    setSelected(null); setSnapshot(null); setEntries([]); setError(null); setTruncated(false)
    if (!context) return
    closeButton.current?.focus()
    const controller = new AbortController()
    setBusy(true)
    const load = context.disk
      ? Promise.resolve({ documents: [{ id: -1, path: context.path!, reason: 'Current disk version', created: Date.now() }], truncated: false })
      : context.path
      ? codingRequest<Entry[]>(context.workspace, { op: 'history', path: context.path }, controller.signal).then(rows => ({ documents: rows.map(r => ({ ...r, path: context.path! })), truncated: false }))
      : codingRequest<{ documents: Entry[]; truncated: boolean }>(context.workspace, { op: 'backups' }, controller.signal)
    void load.then(result => {
      if (controller.signal.aborted) return
      setEntries(result.documents); setTruncated(result.truncated)
      if (context.disk) setSelected(result.documents[0] ?? null)
    }).catch(e => { if (!controller.signal.aborted) setError(e.message) })
      .finally(() => { if (!controller.signal.aborted) setBusy(false) })
    return () => controller.abort()
  }, [context, reload])
  useEffect(() => {
    setSnapshot(null); setDiscardArmed(false)
    if (!context || !selected) return
    const controller = new AbortController()
    setBusy(true); setError(null)
    const load = context.disk
      ? codingRequest<Omit<FileReadPush, 'k'>>(context.workspace, { op: 'read', path: selected.path }, controller.signal).then(value => {
        if (value.error || value.binary || value.tooLarge || value.content == null) throw new Error(value.error ?? 'Disk file cannot be loaded as text. Your buffer is unchanged.')
        return { content: value.content, fingerprint: value.fingerprint }
      })
      : selected.id != null
      ? codingRequest<{ content: string }>(context.workspace, { op: 'historyRead', checkpoint: selected.id }, controller.signal).then(value => ({ content: value.content }))
      : codingRequest<CodingBackup>(context.workspace, { op: 'backupRead', path: selected.path, windowId: selected.windowId!, revision: selected.revision! }, controller.signal).then(backup => ({ content: backup.content, backup }))
    void load.then(value => {
      if (controller.signal.aborted) return
      const current = useKoma.getState().coding.files[fileKey(context.workspace.root, selected.path)]
      const backup = 'backup' in value ? value.backup as CodingBackup : undefined
      setSnapshot({ ...value, baseline: current?.content ?? backup?.savedContent ?? '' })
    }).catch(e => { if (!controller.signal.aborted) setError(e.message) })
      .finally(() => { if (!controller.signal.aborted) setBusy(false) })
    return () => controller.abort()
  }, [selected, context])
  useEffect(() => {
    if (!snapshot || !container.current || !selected) return
    const element = container.current
    let stopped = false
    let dispose: (() => void) | undefined
    // Keep Monaco out of the startup bundle; comparison editors load on demand.
    void Promise.all([import('monaco-editor/esm/vs/editor/editor.api'), import('../lib/monaco-setup')]).then(([monaco, { initMonaco, langFromPath, readMonoFont, applyKomaTheme }]) => {
      if (stopped) return
      initMonaco()
      monaco.editor.setTheme(applyKomaTheme())
      const original = monaco.editor.createModel(snapshot.baseline, langFromPath(selected.path))
      const modified = monaco.editor.createModel(snapshot.content, langFromPath(selected.path))
      const editor = monaco.editor.createDiffEditor(element, {
        readOnly: true, originalEditable: false, automaticLayout: true, minimap: { enabled: false },
        renderSideBySide: true, fontFamily: readMonoFont(), fontSize: 12, scrollBeyondLastLine: false,
      })
      editor.setModel({ original, modified })
      dispose = () => { editor.dispose(); original.dispose(); modified.dispose() }
    }).catch(error => { if (!stopped) setError(error instanceof Error ? error.message : String(error)) })
    return () => { stopped = true; dispose?.() }
  }, [snapshot, selected])
  const restore = async (keepEditor = false) => {
    if (!context || !selected || !snapshot) return
    const { workspace } = context
    const key = fileKey(workspace.root, selected.path)
    const before = useKoma.getState().coding.files[key]
    if (before?.saving || before?.loading) { setError('Wait for this file to finish loading or saving.'); return }
    if (snapshot.backup && before?.dirty && before.content !== snapshot.content) { setError('This file has unsaved edits. Save or discard them before recovering another draft.'); return }
    if (before && before.content !== snapshot.baseline) { setError('The editor changed while the preview was open. Select the entry again.'); return }
    setBusy(true); setError(null)
    try {
      if (before?.content != null) await checkpointCodingDocument(workspace, selected.path, before.content, 'Before restore')
      const state = useKoma.getState()
      if ((state.remoteState.hostId ?? 'local') !== workspace.hostId || state.coding.files[key] !== before) throw new Error('The workspace or document changed. Reopen the preview.')
      const backup = snapshot.backup
      if (!backup && !before) throw new Error('Open the document before restoring its history.')
      const savedContent = context.disk ? snapshot.content : before?.savedContent ?? backup?.savedContent ?? null
      const content = keepEditor ? before!.content! : snapshot.content
      const fingerprint = context.disk ? snapshot.fingerprint! : before?.fingerprint ?? backup?.fingerprint ?? ''
      // Keep the original fingerprint: externally changed disk files must still reject Save.
      useKoma.setState(s => ({ coding: { ...s.coding, files: { ...s.coding.files,
        [key]: { ...(before ?? emptyFileState()), content, savedContent, fingerprint, dirty: content !== savedContent, manualSaveRequired: content !== savedContent, loading: false, conflict: context.disk ? false : before?.conflict ?? false, error: null },
      } } }))
      if (content !== savedContent) backupCodingDocument(workspace, selected.path, { content, savedContent, fingerprint })
      else forgetCodingDraft(workspace, selected.path)
      state.openCodingFile(workspace.root, selected.path)
      setContext(null)
    } catch (e) { setError(e instanceof Error ? e.message : String(e)) }
    finally { setBusy(false) }
  }
  const discard = async () => {
    if (!context || !selected?.windowId) return
    if (selected.windowId === codingWindowId) { setError('This window owns the draft. Discard it by closing or reverting the editor.'); return }
    if (!discardArmed) { setDiscardArmed(true); return }
    setBusy(true); setError(null)
    try {
      await codingRequest(context.workspace, { op: 'forgetBackup', windowId: selected.windowId, path: selected.path, revision: selected.revision! })
      setReload(n => n + 1)
    } catch (e) { setError(e instanceof Error ? e.message : String(e)) }
    finally { setBusy(false) }
  }
  if (!context) return null
  return <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/30 p-5" onMouseDown={close}>
    <div role="dialog" aria-modal="true" aria-label={context.disk ? 'Resolve Disk Changes' : context.path ? 'Local History' : 'Recover Unsaved Files'} className="flex h-[min(620px,85vh)] w-[min(1000px,95vw)] flex-col overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-xl" onMouseDown={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape') { e.stopPropagation(); close() } }}>
      <div className="flex h-9 flex-none items-center gap-2 border-b border-koma-border px-3 text-[12px] text-koma-fg">
        <History size={13}/><span className="min-w-0 flex-1 truncate">{context.disk ? `Resolve Disk Changes · ${context.path}` : context.path ? `Local History · ${context.path}` : `Recover Unsaved Files · ${context.workspace.root}`}</span>
        {busy && <BrailleSpinner size={13}/>}
        <button ref={closeButton} aria-label="Close" onClick={close} className="rounded p-1 text-koma-dim hover:bg-koma-hover"><X size={13}/></button>
      </div>
      {error && <div className="border-b border-koma-border px-3 py-2 text-[12px] text-koma-error">{error}</div>}
      <div className="flex min-h-0 flex-1">
        <div className="w-56 flex-none overflow-y-auto border-r border-koma-border py-1">
          {entries.map(entry => <button key={entry.id ?? `${entry.windowId}:${entry.path}`} onClick={() => setSelected({ ...entry })} className={`w-full px-3 py-2 text-left text-[11px] text-koma-fg ${selected?.id != null ? selected.id === entry.id ? 'bg-koma-hover' : '' : selected?.windowId === entry.windowId && selected?.path === entry.path ? 'bg-koma-hover' : ''}`}>
            <div className="truncate">{entry.reason ?? entry.path}</div><div className="text-[10px] text-koma-dim">{new Date(entry.created ?? entry.updated ?? 0).toLocaleString()}{entry.windowId === codingWindowId ? ' · This window' : ''}</div>
          </button>)}
          {!busy && !entries.length && <div className="px-3 py-2 text-[12px] text-koma-dim">No saved entries.</div>}
          {truncated && <div className="px-3 py-2 text-[11px] text-koma-dim">Showing the latest 200 drafts.</div>}
        </div>
        <div className="flex min-w-0 flex-1 flex-col">
          {snapshot ? <><div className="border-b border-koma-border px-3 py-1 text-[10px] text-koma-dim">Current buffer / original baseline → Selected snapshot</div><div ref={container} className="min-h-0 flex-1"/></> : <div className="p-4 text-[12px] text-koma-dim">Select an entry to compare.</div>}
        </div>
      </div>
      <div className="flex flex-none items-center justify-end gap-2 border-t border-koma-border px-3 py-2 text-[11px]">
        {selected?.windowId && <button disabled={busy || selected.windowId === codingWindowId} onClick={() => void discard()} className="rounded px-2 py-1 text-koma-dim hover:bg-koma-hover disabled:opacity-40">{discardArmed ? 'Confirm discard' : 'Discard draft'}</button>}
        {context.disk && <button disabled={busy || !snapshot} onClick={() => void restore(true)} className="rounded border border-koma-border px-2 py-1 text-koma-fg hover:bg-koma-hover disabled:opacity-40">Keep editor version</button>}
        <button disabled={busy || !snapshot} onClick={() => void restore()} className="flex items-center gap-1 rounded border border-koma-border px-2 py-1 text-koma-fg hover:bg-koma-hover disabled:opacity-40"><RotateCcw size={12}/>{context.disk ? 'Use disk version' : 'Restore to editor'}</button>
      </div>
    </div>
  </div>
}
