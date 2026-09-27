import { useEffect, useRef, useState } from 'react'
import { Package, RefreshCw, X } from 'lucide-react'
import { useKoma, fileKey } from '../store/koma'
import { codingRequest, workspaceKey, type WorkspaceRef, type CodingTaskRun } from '../lib/coding-service'
import { invalidateCodingConfig } from '../lib/coding-config'
import { showCodingTasks } from './CodingTasks'

type Pack = { id: string; label: string; runtime: string | null; runtimeName: string; selected: string; candidates: string[]; lsp: string | null; server: string; adapter: string | null; adapterName: string; test: string }
type Status = { packs: Pack[]; fingerprint: string; platform: string }
type Plan = { id: string; label: string; commands: { command: string; args: string[] }[]; notes: string }
const button = 'rounded px-2 py-1 text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:opacity-40'
export function showCodingPacks() { window.dispatchEvent(new Event('koma-coding-packs')) }
export function CodingPacks() {
  const [open, setOpen] = useState(false)
  const [workspace, setWorkspace] = useState<WorkspaceRef | null>(null)
  const [known, setKnown] = useState<WorkspaceRef[]>([])
  const [data, setData] = useState<Status | null>(null)
  const [packId, setPackId] = useState('python')
  const [executable, setExecutable] = useState('')
  const [runtime, setRuntime] = useState(false)
  const [plan, setPlan] = useState<Plan | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [refresh, setRefresh] = useState(0)
  const mutation = useRef(false)
  const generation = useRef(0)
  const focus = useRef<HTMLElement | null>(null)
  const closeRef = useRef<HTMLButtonElement>(null)
  const scope = workspace ? workspaceKey(workspace) : ''
  useEffect(() => {
    const show = () => {
      const s = useKoma.getState(); const hostId = s.remoteState.hostId ?? 'local'
      const root = s.coding.activeRoot ?? s.settingsValues?.workdir?.[0]
      focus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
      setKnown(old => [...new Map([...old, ...(s.settingsValues?.workdir ?? []).map(root => ({ hostId, root }))].map(w => [workspaceKey(w), w])).values()])
      if (root) setWorkspace({ hostId, root }); setOpen(true); setRefresh(n => n + 1)
    }
    window.addEventListener('koma-coding-packs', show)
    return () => window.removeEventListener('koma-coding-packs', show)
  }, [])
  useEffect(() => { if (open) closeRef.current?.focus() }, [open])
  useEffect(() => {
    generation.current++; setPlan(null); setError(''); setData(null)
    if (!open || !workspace) return
    const abort = new AbortController()
    void codingRequest<Status>(workspace, { op: 'packs' }, abort.signal).then(v => { if (!abort.signal.aborted) setData(v) }).catch(e => { if (!abort.signal.aborted) setError(String(e.message ?? e)) })
    return () => abort.abort()
  }, [open, scope, refresh])
  const pack = data?.packs.find(p => p.id === packId)
  useEffect(() => { setExecutable(pack?.selected ?? ''); setPlan(null) }, [packId, data])
  const close = () => { generation.current++; setOpen(false); focus.current?.focus() }
  const act = async (action: 'plan' | 'apply' | 'select') => {
    if (!workspace || !data || mutation.current) return
    mutation.current = true; setBusy(true); setError(''); const gen = generation.current
    try {
      if (action === 'plan') {
        const result = await codingRequest<Plan>(workspace, { op: 'packPlan', packId, runtime })
        if (gen === generation.current) setPlan(result)
      } else if (action === 'apply' && plan) {
        const run = await codingRequest<CodingTaskRun>(workspace, { op: 'packApply', planId: plan.id })
        if (gen === generation.current) { setPlan(null); close(); showCodingTasks(undefined, workspace, run.id) }
      } else if (action === 'select') {
        const s = useKoma.getState()
        const config = s.coding.files[fileKey(workspace.root, '.koma/coding.json')]
        if ((s.remoteState.hostId ?? 'local') === workspace.hostId && config && config.content !== config.savedContent) throw new Error('Save or discard your open coding settings before changing the environment.')
        await codingRequest(workspace, { op: 'environmentSelect', language: packId, executable, fingerprint: data.fingerprint })
        invalidateCodingConfig(workspace)
        if (gen === generation.current) setRefresh(n => n + 1)
      }
    } catch (e) { if (gen === generation.current) setError(String(e instanceof Error ? e.message : e)) }
    finally { mutation.current = false; setBusy(false) }
  }
  if (!open) return null
  return <section role="dialog" aria-label="Language packs" className="absolute inset-x-10 bottom-6 z-40 flex max-h-[75vh] flex-col rounded-md border border-koma-border bg-koma-panel text-[11px] text-koma-fg shadow-xl" onKeyDown={e => { if (e.key === 'Escape') { e.stopPropagation(); close() } }}>
    <div className="flex h-8 items-center gap-2 border-b border-koma-border px-2">
      <Package size={12} /><span className="text-koma-dim">LANGUAGE PACKS</span>
      <select aria-label="Language pack workspace" className="min-w-0 flex-1 bg-koma-panel" value={scope} disabled={busy} onChange={e => setWorkspace(known.find(w => workspaceKey(w) === e.target.value) ?? null)}>{known.map(w => <option key={workspaceKey(w)} value={workspaceKey(w)}>{w.hostId} · {w.root}</option>)}</select>
      <button className={button} title="Refresh installed components" disabled={busy} onClick={() => setRefresh(n => n + 1)}><RefreshCw size={12} /></button>
      <button ref={closeRef} className={button} aria-label="Close language packs" onClick={close}><X size={12} /></button>
    </div>
    <div className="overflow-auto p-3 space-y-3">
      {!workspace && <p>Select a coding workspace first.</p>}
      {data && <>
        <select aria-label="Language" className="bg-koma-panel border border-koma-border rounded px-2 py-1" value={packId} disabled={busy} onChange={e => setPackId(e.target.value)}>{data.packs.map(p => <option key={p.id} value={p.id}>{p.label}</option>)}</select>
        {pack && <>
          <dl className="grid grid-cols-[90px_1fr] gap-1 break-all">
            <dt className="text-koma-dim">Runtime</dt><dd>{pack.runtime ?? (pack.runtimeName ? `${pack.runtimeName} · missing` : 'Not required')}</dd>
            <dt className="text-koma-dim">Language server</dt><dd>{pack.lsp ?? `${pack.server} · missing`}</dd>
            <dt className="text-koma-dim">Debugger</dt><dd>{pack.adapter ?? (pack.adapterName ? `${pack.adapterName} · missing` : 'Not applicable')}</dd>
            <dt className="text-koma-dim">Tests</dt><dd>{pack.test || 'Not applicable'}</dd>
          </dl>
          {pack.runtimeName && !['web', 'toml'].includes(packId) && <div className="flex items-center gap-2">
            <label htmlFor="coding-executable" className="text-koma-dim">Executable</label><input id="coding-executable" list="coding-environments" value={executable} onChange={e => setExecutable(e.target.value)} placeholder="Automatic (host PATH)" className="min-w-0 flex-1 rounded border border-koma-border bg-koma-panel px-2 py-1" />
            <datalist id="coding-environments">{pack.candidates.map(v => <option key={v} value={v} />)}</datalist>
            <button className={button} disabled={busy} onClick={() => void act('select')}>Use for workspace</button>
          </div>}
          <p className="text-koma-dim">Environment changes apply to new tasks. Reopen language-server documents after changing an interpreter.</p>
          <div className="flex items-center gap-3"><label className="flex items-center gap-1"><input type="checkbox" checked={runtime} disabled={busy} onChange={e => { setRuntime(e.target.checked); setPlan(null) }} />Include runtime / compiler</label><button className={button} disabled={busy} onClick={() => void act('plan')}>Review install / update</button></div>
        </>}
      </>}
      {plan && <div className="rounded border border-koma-border p-2 space-y-2"><p>{plan.label} · install / update on {workspace?.hostId}</p><pre className="whitespace-pre-wrap break-all text-koma-dim">{plan.commands.map(c => [c.command, ...c.args].map(v => JSON.stringify(v)).join(' ')).join('\n')}</pre><p className="text-koma-dim">{plan.notes}</p><button className={button} disabled={busy} onClick={() => void act('apply')}>Install / update</button></div>}
      {error && <p role="alert" className="text-koma-error">{error}</p>}
    </div>
  </section>
}
