import { KomaSelect } from './KomaSelect'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { CircleHelp, FileCog, Package, RefreshCw, Square } from 'lucide-react'
import { useKoma, fileKey } from '../store/koma'
import { codingRequest, workspaceKey, type WorkspaceRef, type CodingTaskRun } from '../lib/coding-service'
import { invalidateCodingConfig } from '../lib/coding-config'
import { showCodingTasks } from './CodingTasks'
import { BrailleSpinner } from './BrailleSpinner'
import { PanelActions, panelButton as button, panelIconButton, panelSelect as selectClass, baseName } from './PanelChrome'

type Chunk = { seq: number; stream: string; text: string }
type TaskOutput = { chunks: Chunk[]; next: number; truncated: boolean; more: boolean }
const liveRun = (run?: CodingTaskRun | null) =>
  !!run && (run.status === 'queued' || run.status === 'running' || run.status === 'stopping')

type Pack = {
  id: string
  label: string
  runtime: string | null
  runtimeName: string
  selected: string
  candidates: string[]
  lsp: string | null
  server: string
  serverOptions: string[]
  adapter: string | null
  adapterName: string
  test: string
}
type Status = { packs: Pack[]; fingerprint: string; platform: string }
type Plan = { id: string; label: string; commands: { command: string; args: string[] }[]; notes: string }

const message = (e: unknown) => String(e instanceof Error ? e.message : e)

/** Click/keyboard help, kept in flow so narrow docks never clip it. */
function PackHelp({ label, children }: { label: string; children: ReactNode }) {
  return (
    <details className="group min-w-0 text-koma-dim">
      <summary aria-label={label} title={label}
        className="flex h-6 w-6 cursor-pointer list-none items-center justify-center rounded hover:bg-koma-hover focus-visible:outline focus-visible:outline-koma-accent [&::-webkit-details-marker]:hidden">
        <CircleHelp size={13} />
      </summary>
      <div className="max-w-sm break-words py-2 text-[11px] leading-relaxed">{children}</div>
    </details>
  )
}

function ComponentRow({ label, name, status, checked, disabled, onChange, help }: {
  label: string; name?: string; status: string; checked: boolean; disabled: boolean
  onChange: (checked: boolean) => void; help: ReactNode
}) {
  return (
    <div className="flex flex-wrap items-start gap-x-2 border-b border-koma-border py-2 last:border-b-0">
      <label className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 py-1">
        <input type="checkbox" aria-label={label} className="accent-koma-accent" checked={checked} disabled={disabled}
          onChange={e => onChange(e.target.checked)} />
        <span>{label}</span>
        {name && <span className="min-w-0 truncate text-koma-dim" title={name}>{' '}{name}</span>}
      </label>
      <span className="py-1 text-[10px] text-koma-dim">{status}</span>
      <PackHelp label={`About ${label}`}>{help}</PackHelp>
    </div>
  )
}

/** Open Language Packs as a BottomPanel tab (not a floating dialog). */
export function showCodingPacks(workspace?: WorkspaceRef, opts?: { notebook?: boolean }) {
  window.dispatchEvent(new CustomEvent('koma-coding-packs', { detail: { workspace, notebook: opts?.notebook } }))
}

/**
 * Language packs dock — same shell as Tasks / Problems / LSP:
 * fill BottomPanel, tool rows, no absolute card / shadow / own close chrome.
 * Native owns installs; this view may close while a pack task keeps running.
 */
export function CodingPacks({ visible: open }: { visible: boolean }) {
  const [workspaces, setWorkspaces] = useState<WorkspaceRef[]>([])
  const [workspace, setWorkspace] = useState<WorkspaceRef | null>(null)
  const [data, setData] = useState<Status | null>(null)
  const [packId, setPackId] = useState('python')
  const [executable, setExecutable] = useState('')
  const [server, setServer] = useState('')
  const [runtime, setRuntime] = useState(false)
  const [notebook, setNotebook] = useState(false)
  const [includeLsp, setIncludeLsp] = useState(true)
  const [includeAdapter, setIncludeAdapter] = useState(true)
  const [view, setView] = useState<'environment' | 'install'>('environment')
  const [plan, setPlan] = useState<Plan | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [loadError, setLoadError] = useState('')
  const [refresh, setRefresh] = useState(0)
  /** Pack install stays on this tab — Tasks is only a secondary escape hatch. */
  const [installRun, setInstallRun] = useState<CodingTaskRun | null>(null)
  const [installChunks, setInstallChunks] = useState<Chunk[]>([])
  const [installFollow, setInstallFollow] = useState(true)
  const installOut = useRef<HTMLPreElement>(null)
  const mutation = useRef(false)
  const generation = useRef(0)
  const scope = workspace ? workspaceKey(workspace) : ''
  const currentHost = useKoma(s => s.remoteState.hostId ?? 'local')
  const currentRoots = useKoma(s => s.settingsValues?.workdir)
  const rootsKey = JSON.stringify(currentRoots ?? [])

  useEffect(() => {
    const onOpen = (event: Event) => {
      const state = useKoma.getState()
      const hostId = state.remoteState.hostId ?? 'local'
      const root = state.coding.activeRoot ?? state.settingsValues?.workdir?.[0]
      const target = (event as CustomEvent<{ workspace?: WorkspaceRef; notebook?: boolean }>).detail
      if (target?.workspace) setWorkspace(target.workspace)
      else if (root) setWorkspace({ hostId, root })
      if (target?.notebook) {
        setPackId('python')
        setNotebook(true)
        setView('install')
      }
      state.setBottomPanelTab('packs')
      setRefresh(n => n + 1)
    }
    window.addEventListener('koma-coding-packs', onOpen)
    return () => window.removeEventListener('koma-coding-packs', onOpen)
  }, [])

  useEffect(() => {
    setWorkspaces(previous =>
      [
        ...new Map(
          [
            ...previous,
            ...JSON.parse(rootsKey).map((root: string) => ({ hostId: currentHost, root })),
            ...(workspace ? [workspace] : []),
          ].map(w => [workspaceKey(w), w]),
        ).values(),
      ],
    )
  }, [rootsKey, currentHost, scope])

  useEffect(() => {
    if (!open || workspace) return
    const state = useKoma.getState()
    const root = state.coding.activeRoot ?? state.settingsValues?.workdir?.[0]
    if (root) setWorkspace({ hostId: currentHost, root })
  }, [open, workspace, currentHost, rootsKey])

  useEffect(() => {
    generation.current++
    setPlan(null)
    setError('')
    setLoadError('')
    setData(null)
    setInstallRun(null)
    setInstallChunks([])
    setBusy(mutation.current)
  }, [scope])

  // Live pack-install progress stays in Language Packs (do not jump to Tasks).
  useEffect(() => {
    if (!open || !workspace || !installRun) return
    const runId = installRun.id
    const controller = new AbortController()
    let timer: ReturnType<typeof setTimeout>
    const pollStatus = async () => {
      try {
        const runs = await codingRequest<CodingTaskRun[]>(workspace, { op: 'taskRuns' }, controller.signal)
        if (controller.signal.aborted) return
        const next = runs.find(r => r.id === runId)
        if (next) setInstallRun(next)
        if (next && !liveRun(next) && next.status === 'succeeded') setRefresh(n => n + 1)
      } catch {
        /* keep last known status */
      } finally {
        if (!controller.signal.aborted && liveRun(installRun)) timer = setTimeout(pollStatus, 1000)
      }
    }
    void pollStatus()
    return () => {
      controller.abort()
      clearTimeout(timer)
    }
  }, [open, scope, installRun?.id, installRun?.status])

  useEffect(() => {
    if (!open || !workspace || !installRun) return
    const runId = installRun.id
    const controller = new AbortController()
    let after = 0
    let retained: Chunk[] = []
    let timer: ReturnType<typeof setTimeout>
    const pollOut = async () => {
      let more = false
      try {
        const value = await codingRequest<TaskOutput>(
          workspace,
          { op: 'taskOutput', runId, after },
          controller.signal,
        )
        if (controller.signal.aborted) return
        after = value.next
        more = value.more
        if (value.chunks.length) {
          const combined = [...retained, ...value.chunks]
          let size = combined.reduce((n, c) => n + c.text.length, 0)
          let begin = 0
          while (size > 1_048_576 || combined.length - begin > 2048) size -= combined[begin++].text.length
          retained = combined.slice(begin)
          setInstallChunks(retained)
        }
      } catch {
        /* keep buffer */
      } finally {
        if (!controller.signal.aborted) timer = setTimeout(pollOut, more ? 100 : 750)
      }
    }
    void pollOut()
    return () => {
      controller.abort()
      clearTimeout(timer)
    }
  }, [open, scope, installRun?.id])

  useEffect(() => {
    if (installFollow && installOut.current) installOut.current.scrollTop = installOut.current.scrollHeight
  }, [installChunks, installFollow])

  useEffect(() => {
    if (!open || !workspace) return
    const abort = new AbortController()
    setData(null)
    setLoadError('')
    void codingRequest<Status>(workspace, { op: 'packs' }, abort.signal)
      .then(v => {
        if (!abort.signal.aborted) setData(v)
      })
      .catch(e => {
        if (!abort.signal.aborted) setLoadError(message(e))
      })
    return () => abort.abort()
  }, [open, scope, refresh])

  const pack = data?.packs.find(p => p.id === packId)
  useEffect(() => {
    setExecutable(pack?.selected ?? '')
    setServer(pack?.server ?? '')
    setPlan(null)
  }, [packId, data])

  const act = async (action: 'plan' | 'apply' | 'select') => {
    if (!workspace || !data || mutation.current) return
    mutation.current = true
    setBusy(true)
    setError('')
    const gen = generation.current
    try {
      if (action === 'plan') {
        const result = await codingRequest<Plan>(workspace, {
          op: 'packPlan',
          packId,
          runtime,
          notebook: packId === 'python' ? notebook : false,
          lsp: includeLsp,
          adapter: includeAdapter && !!pack?.adapterName && pack.adapterName !== 'lldb-dap',
          server,
        })
        if (gen === generation.current) { setInstallRun(null); setPlan(result) }
      } else if (action === 'apply' && plan) {
        const run = await codingRequest<CodingTaskRun>(workspace, { op: 'packApply', planId: plan.id })
        if (gen === generation.current) {
          setPlan(null)
          setView('install')
          setInstallChunks([])
          setInstallFollow(true)
          setInstallRun(run)
        }
      } else if (action === 'select') {
        const s = useKoma.getState()
        const config = s.coding.files[fileKey(workspace.root, '.koma/coding.json')]
        if (
          (s.remoteState.hostId ?? 'local') === workspace.hostId &&
          config &&
          config.content !== config.savedContent
        ) {
          throw new Error('Save or discard your open coding settings before changing the environment.')
        }
        await codingRequest(workspace, {
          op: 'environmentSelect',
          language: packId,
          executable,
          fingerprint: data.fingerprint,
          server: packId === 'php' ? server : undefined,
        })
        invalidateCodingConfig(workspace)
        if (gen === generation.current) setRefresh(n => n + 1)
      }
    } catch (e) {
      if (gen === generation.current) setError(message(e))
    } finally {
      mutation.current = false
      setBusy(false)
    }
  }

  const editConfig = async () => {
    if (!workspace) return
    const target = workspace
    const gen = generation.current
    try {
      await codingRequest(target, { op: 'configEnsure' })
      const state = useKoma.getState()
      if ((state.remoteState.hostId ?? 'local') === target.hostId) {
        state.openCodingFile(target.root, '.koma/coding.json')
      } else if (gen === generation.current) {
        setError('Switch to this host to edit its coding configuration.')
      }
    } catch (e) {
      if (gen === generation.current) setError(message(e))
    }
  }

  if (!open) return null

  return (
    <section aria-label="Language packs" className="flex min-h-0 flex-1 flex-col overflow-hidden text-[11px] text-koma-fg">
      <PanelActions>
        <button
          type="button"
          className={panelIconButton}
          title="Refresh installed components"
          aria-label="Refresh language packs"
          disabled={busy}
          onClick={() => setRefresh(n => n + 1)}
        >
          <RefreshCw size={12} />
        </button>
        <button
          type="button"
          className={panelIconButton}
          title="Open coding configuration"
          aria-label="Open coding configuration"
          disabled={!workspace || currentHost !== workspace.hostId}
          onClick={() => void editConfig()}
        >
          <FileCog size={12} />
        </button>
      </PanelActions>
      <div className="flex flex-none flex-wrap items-center gap-2 border-b border-koma-border px-2 py-1">
        <KomaSelect
          className={`${selectClass} flex-1 basis-40`}
          aria-label="Language pack workspace"
          value={scope}
          disabled={busy}
          onChange={e => {
            generation.current++
            setWorkspace(workspaces.find(w => workspaceKey(w) === e.target.value) ?? null)
          }}
        >
          {!workspace && <option value="">Choose a workspace</option>}
          {workspaces.map(w => (
            <option key={workspaceKey(w)} value={workspaceKey(w)} title={w.root}>
              {w.hostId} · {baseName(w.root)}
            </option>
          ))}
        </KomaSelect>
        <Package size={12} className="flex-none text-koma-dim" />
        <div className="w-40 min-w-0 max-w-full flex-none">
          <KomaSelect
            className={selectClass}
            aria-label="Language"
            value={packId}
            disabled={busy || !data}
            onChange={e => setPackId(e.target.value)}
          >
            {(data?.packs ?? [{ id: 'python', label: 'Python' }]).map(p => (
              <option key={p.id} value={p.id}>{p.label}</option>
            ))}
          </KomaSelect>
        </div>
        <div className="flex items-center gap-1" aria-label="Language pack view">
          {(['environment', 'install'] as const).map(value => (
            <button type="button" key={value} aria-pressed={view === value} onClick={() => setView(value)}
              className={`${button} ${view === value ? 'bg-koma-hover text-koma-fg' : ''}`}>
              {value === 'environment' ? 'Environment' : 'Tools'}
            </button>
          ))}
        </div>
      </div>

      {/* One scroll owner: tools stay reachable, content never shrinks to zero. */}
      <div className="min-h-0 flex-1 overflow-auto px-3 py-2">
        <div className="w-full max-w-3xl space-y-4">
          {(loadError || error) && (
            <div role="alert" className="text-koma-error">{[loadError, error].filter(Boolean).join(' · ')}</div>
          )}
          {!workspace && <p className="text-koma-dim">Choose a coding workspace first.</p>}
          {workspace && !data && !loadError && (
            <div className="flex items-center gap-2 text-koma-dim"><BrailleSpinner size={12} /> Loading packs…</div>
          )}
          {pack && view === 'environment' && <>
            {pack.runtimeName && !['web', 'toml'].includes(packId) && (
              <div className="space-y-2">
                <div className="flex items-start gap-2">
                  <label htmlFor="coding-executable" className="py-1">{packId === 'python' ? 'Python interpreter' : 'Executable'}</label>
                  <PackHelp label="About workspace environment">
                    Used for new tasks, debugging, and notebook defaults. Leave blank for automatic selection.
                    Language servers restart when you apply a change.
                  </PackHelp>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <input id="coding-executable" list="coding-environments" value={executable}
                    onChange={e => setExecutable(e.target.value)} disabled={busy}
                    placeholder="Automatic"
                    className="h-6 min-w-0 flex-1 rounded border border-koma-border bg-koma-panel2 px-2 text-[11px] text-koma-fg" />
                  <datalist id="coding-environments">{pack.candidates.map(v => <option key={v} value={v} />)}</datalist>
                  <button type="button" className={`${button} border border-koma-border`} disabled={busy || !data}
                    onClick={() => void act('select')}>Apply</button>
                </div>
              </div>
            )}
            <div className="flex flex-wrap items-center gap-2 border-t border-koma-border pt-3">
              <span className="text-koma-dim">Language server</span>
              <span>{pack.server}</span>
              <span className="text-koma-dim">{pack.lsp ? 'Installed' : 'Not installed'}</span>
              <button type="button" className={button} onClick={() => setView('install')}>Manage tools</button>
            </div>
          </>}
          {pack && view === 'install' && <>
            {!installRun && <div className="space-y-3">
              <div className="flex items-start gap-2">
                <span className="py-1">Tools for {pack.label}</span>
                <PackHelp label="About installing tools">
                  Select tools to install or update. You will review the commands before anything runs.
                  Project test packages are not included.
                </PackHelp>
              </div>
              <div>
                <ComponentRow label="Language server" name={server || pack.server}
                  status={server === pack.server && pack.lsp ? 'Installed' : 'Not installed'}
                  checked={includeLsp} disabled={busy}
                  onChange={v => { setIncludeLsp(v); setPlan(null) }}
                  help={<>Code completion and diagnostics.{pack.lsp && <span className="mt-1 block break-all">{pack.lsp}</span>}</>} />
                {pack.serverOptions.length > 1 && <div className="w-48 max-w-full py-2">
                  <KomaSelect className={selectClass} aria-label="Language server" value={server}
                    disabled={busy || !includeLsp} onChange={e => { setServer(e.target.value); setPlan(null) }}>
                    {pack.serverOptions.map(id => <option key={id} value={id}>{id}</option>)}
                  </KomaSelect>
                </div>}
                {!!pack.adapterName && <ComponentRow label="Debugger" name={pack.adapterName}
                  status={pack.adapter ? 'Installed' : pack.adapterName === 'lldb-dap' ? 'Manual setup' : 'Not installed'}
                  checked={pack.adapterName === 'lldb-dap' ? false : includeAdapter}
                  disabled={busy || pack.adapterName === 'lldb-dap'}
                  onChange={v => { setIncludeAdapter(v); setPlan(null) }}
                  help={<>{pack.adapterName === 'lldb-dap' ? 'Install LLVM separately and make lldb-dap available on PATH.' : 'Breakpoints and step-by-step debugging.'}
                    {pack.adapter && <span className="mt-1 block break-all">{pack.adapter}</span>}</>} />}
                {!!pack.runtimeName && <ComponentRow label="Runtime" name={pack.runtimeName}
                  status={pack.runtime ? 'Installed' : 'Not installed'} checked={runtime} disabled={busy}
                  onChange={v => { setRuntime(v); setPlan(null) }}
                  help={<>Runs or compiles your code.{pack.runtime && <span className="mt-1 block break-all">{pack.runtime}</span>}</>} />}
                {packId === 'python' && <ComponentRow label="Notebook kernel" name="ipykernel"
                  status="Optional" checked={notebook} disabled={busy}
                  onChange={v => { setNotebook(v); setPlan(null) }}
                  help={<>Runs notebook cells. Installs with pip into the workspace Python, not a notebook override.
                    <span className="mt-1 block break-all">{pack.runtime ?? 'Workspace Python'}</span></>} />}
              </div>
              {!plan && <button type="button" className={`${button} border border-koma-accent/40 text-koma-accent`}
                disabled={busy || !data || !(includeLsp || (includeAdapter && !!pack.adapterName && pack.adapterName !== 'lldb-dap') || (runtime && !!pack.runtimeName) || (packId === 'python' && notebook))}
                onClick={() => void act('plan')}>
                {busy && <BrailleSpinner size={12} />} Review changes
              </button>}
            </div>}

            {plan && !installRun && (
              <div className="space-y-3 border-t border-koma-border pt-3">
                <div>Review changes</div>
                <pre className="whitespace-pre-wrap break-all rounded border border-koma-border bg-koma-panel2 p-3 text-[11px] text-koma-dim">
                  {plan.commands.map(c => [c.command, ...c.args].map(v => JSON.stringify(v)).join(' ')).join('\n')}
                </pre>
                {plan.notes && <PackHelp label="Installation notes">{plan.notes}</PackHelp>}
                <button
                  type="button"
                  className={`${button} border border-koma-accent/40 text-koma-accent`}
                  disabled={busy}
                  onClick={() => void act('apply')}
                >
                  Install / update
                </button>
              </div>
            )}

            {installRun && workspace && (
              <div className="space-y-2 border-t border-koma-border pt-3">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-koma-fg">
                    {installRun.label} · {installRun.status}
                    {installRun.exitCode != null ? ` · exit ${installRun.exitCode}` : ''}
                  </span>
                  {liveRun(installRun) && <BrailleSpinner size={12} />}
                  <label className="ml-auto flex items-center gap-1 text-koma-dim">
                    <input
                      type="checkbox"
                      className="accent-koma-accent"
                      checked={installFollow}
                      onChange={e => setInstallFollow(e.target.checked)}
                    />
                    Follow
                  </label>
                  {liveRun(installRun) && (
                    <button
                      type="button"
                      className={button}
                      title="Stop install"
                      disabled={busy}
                      onClick={() => {
                        void codingRequest(workspace, { op: 'taskStop', runId: installRun.id })
                          .then(run => setInstallRun(run as CodingTaskRun))
                          .catch(e => setError(message(e)))
                      }}
                    >
                      <Square size={12} /> Stop
                    </button>
                  )}
                  <button
                    type="button"
                    className={button}
                    title="Also open this run under Tasks"
                    onClick={() => showCodingTasks(undefined, workspace, installRun.id)}
                  >
                    Open in Tasks
                  </button>
                  {!liveRun(installRun) && (
                    <button type="button" className={button} onClick={() => { setInstallRun(null); setInstallChunks([]); setRefresh(n => n + 1) }}>
                      Done
                    </button>
                  )}
                </div>
                {installRun.error && <div className="text-koma-error">{installRun.error}</div>}
                <pre
                  ref={installOut}
                  tabIndex={0}
                  aria-label="Install output"
                  className="max-h-56 min-h-[8rem] overflow-auto whitespace-pre-wrap break-words rounded border border-koma-border bg-koma-panel2 p-3 font-mono text-[11px] text-koma-dim"
                >
                  {installChunks.length
                    ? installChunks.map(c => c.text).join('')
                    : liveRun(installRun)
                      ? 'Waiting for output…'
                      : 'No output'}
                </pre>
              </div>
            )}
          </>}
        </div>
      </div>
    </section>
  )
}
