import { taskProblems } from '../lib/coding-task-problems'
import { pathToUri } from '../lib/lsp-bridge'
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from 'react'
import { FileCog, Play, RefreshCw, Square, Terminal, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { codingRequest, workspaceKey, type CodingTask, type CodingTaskRun, type WorkspaceRef } from '../lib/coding-service'
import { BrailleSpinner } from './BrailleSpinner'

const CodingTaskTerminal = lazy(() => import('./CodingTaskTerminal'))

type Group = 'run' | 'build' | 'test'
type Definitions = { tasks: CodingTask[]; fingerprint: string }
type Chunk = { seq: number; stream: string; text: string }
type Output = { chunks: Chunk[]; next: number; truncated: boolean; more: boolean }
const button = 'flex h-6 items-center justify-center gap-1 rounded px-1.5 text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:opacity-40 disabled:pointer-events-none'
const live = (run?: CodingTaskRun) => run?.status === 'queued' || run?.status === 'running' || run?.status === 'stopping'
const message = (e: unknown) => String(e instanceof Error ? e.message : e)

export function showCodingTasks(group?: Group, workspace?: WorkspaceRef, runId?: string) {
  window.dispatchEvent(new CustomEvent('koma-coding-tasks', { detail: { group, workspace, runId } }))
}

/** Native owns the processes. This view may close or change scope at any time. */
export function CodingTasks() {
  const [open, setOpen] = useState(false)
  const [workspaces, setWorkspaces] = useState<WorkspaceRef[]>([])
  const [workspace, setWorkspace] = useState<WorkspaceRef | null>(null)
  const [group, setGroup] = useState<Group>('run')
  const [definitions, setDefinitions] = useState<Definitions | null>(null)
  const [taskId, setTaskId] = useState('')
  const [runs, setRuns] = useState<CodingTaskRun[]>([])
  const [runId, setRunId] = useState('')
  const [chunks, setChunks] = useState<Chunk[]>([])
  const [outputFor, setOutputFor] = useState('')
  const [truncated, setTruncated] = useState(false)
  const [configError, setConfigError] = useState('')
  const [error, setError] = useState('')
  const [pollError, setPollError] = useState('')
  const [outputError, setOutputError] = useState('')
  const [busy, setBusy] = useState(false)
  const [refresh, setRefresh] = useState(0)
  const [follow, setFollow] = useState(true)
  const output = useRef<HTMLPreElement>(null)
  const closeButton = useRef<HTMLButtonElement>(null)
  const previousFocus = useRef<HTMLElement | null>(null)
  const requestedRun = useRef('')
  const generation = useRef(0)
  const mutation = useRef(false)
  const mutationEpoch = useRef(0)
  const scope = workspace ? workspaceKey(workspace) : ''
  const currentHost = useKoma(s => s.remoteState.hostId ?? 'local')
  const currentRoots = useKoma(s => s.settingsValues?.workdir)
  const rootsKey = JSON.stringify(currentRoots ?? [])

  const close = () => { setOpen(false); previousFocus.current?.focus() }
  useEffect(() => {
    const onOpen = (event: Event) => {
      const state = useKoma.getState()
      const hostId = state.remoteState.hostId ?? 'local'
      const root = state.coding.activeRoot ?? state.settingsValues?.workdir?.[0]
      previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
      const target = (event as CustomEvent<{ workspace?: WorkspaceRef; runId?: string }>).detail
      if (target?.workspace) setWorkspace(target.workspace)
      else if (root) setWorkspace({ hostId, root })
      requestedRun.current = target?.runId ?? ''
      const requested = (event as CustomEvent<{ group?: Group }>).detail?.group
      if (requested) setGroup(requested)
      setOpen(true); setRefresh(n => n + 1)
    }
    window.addEventListener('koma-coding-tasks', onOpen)
    return () => window.removeEventListener('koma-coding-tasks', onOpen)
  }, [])
  useEffect(() => {
    // Remember visited hosts/roots so their tasks stay reachable after switching.
    setWorkspaces(previous => [...new Map([
      ...previous, ...JSON.parse(rootsKey).map((root: string) => ({ hostId: currentHost, root })),
      ...(workspace ? [workspace] : []),
    ].map(w => [workspaceKey(w), w])).values()])
  }, [rootsKey, currentHost, scope])
  useEffect(() => { if (open) closeButton.current?.focus() }, [open])
  useEffect(() => {
    generation.current++
    setRuns([]); setRunId(''); setError(''); setPollError(''); setBusy(mutation.current)
  }, [scope])

  useEffect(() => {
    if (!open || !workspace) return
    const controller = new AbortController()
    setDefinitions(null); setConfigError('')
    void codingRequest<Definitions>(workspace, { op: 'taskDefinitions' }, controller.signal)
      .then(value => {
        if (controller.signal.aborted) return
        setDefinitions(value)
        setTaskId(id => value.tasks.some(t => t.id === id && (t.group ?? 'run') === group) ? id : value.tasks.find(t => (t.group ?? 'run') === group)?.id ?? '')
      }).catch(e => { if (!controller.signal.aborted) setConfigError(message(e)) })
    return () => controller.abort()
  }, [open, scope, group, refresh])

  useEffect(() => {
    if (!open || !workspace) return
    const controller = new AbortController()
    let timer: ReturnType<typeof setTimeout>
    const poll = async () => {
      const epoch = mutationEpoch.current
      try {
        if (mutation.current) return
        const value = await codingRequest<CodingTaskRun[]>(workspace, { op: 'taskRuns' }, controller.signal)
        if (controller.signal.aborted || epoch !== mutationEpoch.current) return
        setRuns(value); setPollError('')
        const target = requestedRun.current; requestedRun.current = ''
        setRunId(id => value.some(r => r.id === target) ? target : value.some(r => r.id === id) ? id : value[value.length - 1]?.id ?? '')
      } catch (e) { if (!controller.signal.aborted) setPollError(message(e)) }
      finally { if (!controller.signal.aborted) timer = setTimeout(poll, 1000) }
    }
    void poll()
    return () => { controller.abort(); clearTimeout(timer) }
  }, [open, scope, refresh])

  useEffect(() => {
    setOutputFor(`${scope}:${runId}`); setChunks([]); setTruncated(false); setOutputError(''); setFollow(true)
    if (!open || !workspace || !runId) return
    const controller = new AbortController()
    let after = 0
    let retained: Chunk[] = []
    let timer: ReturnType<typeof setTimeout>
    const poll = async () => {
      let more = false
      try {
        const value = await codingRequest<Output>(workspace, { op: 'taskOutput', runId, after }, controller.signal)
        if (controller.signal.aborted) return
        after = value.next; more = value.more
        setOutputError('')
        if (value.truncated) setTruncated(true)
        if (value.chunks.length) {
          const combined = [...retained, ...value.chunks]
          let size = combined.reduce((n, c) => n + c.text.length, 0)
          let begin = 0
          while (size > 1_048_576 || combined.length - begin > 2048) size -= combined[begin++].text.length
          if (begin > 0) setTruncated(true)
          retained = combined.slice(begin)
          setChunks(retained)
        }
      } catch (e) { if (!controller.signal.aborted) setOutputError(message(e)) }
      finally { if (!controller.signal.aborted) timer = setTimeout(poll, more ? 100 : 750) }
    }
    void poll()
    return () => { controller.abort(); clearTimeout(timer) }
  }, [open, scope, runId])
  useEffect(() => {
    if (follow && output.current) output.current.scrollTop = output.current.scrollHeight
  }, [chunks, follow])

  const visibleChunks = outputFor === `${scope}:${runId}` ? chunks : []
  const problems = useMemo(() => taskProblems(visibleChunks.map(c => c.text).join('')), [visibleChunks])
  if (!open) return null
  const choices = definitions?.tasks.filter(t => (t.group ?? 'run') === group) ?? []
  const task = choices.find(t => t.id === taskId)
  const run = runs.find(r => r.id === runId)
  const act = async (action: 'start' | 'stop') => {
    if (!workspace || mutation.current) return
    if (action === 'start' && (!task || !definitions)) return
    if (action === 'stop' && !run) return
    const gen = generation.current
    mutation.current = true; mutationEpoch.current++; setBusy(true); setError('')
    try {
      const value = await codingRequest<CodingTaskRun>(workspace, action === 'start'
        ? { op: 'taskStart', taskId: task!.id, fingerprint: definitions!.fingerprint }
        : { op: 'taskStop', runId: run!.id })
      if (gen !== generation.current) return
      setRuns(previous => [...previous.filter(r => r.id !== value.id), value])
      setRunId(value.id)
    } catch (e) { if (gen === generation.current) setError(message(e)) }
    finally { mutation.current = false; mutationEpoch.current++; setBusy(false) }
  }
  const editConfig = async () => {
    if (!workspace) return
    const target = workspace
    const gen = generation.current
    try {
      await codingRequest(target, { op: 'configEnsure' })
      const state = useKoma.getState()
      if ((state.remoteState.hostId ?? 'local') === target.hostId) state.openCodingFile(target.root, '.koma/coding.json')
      else if (gen === generation.current) setError('Switch to this host to edit its task configuration.')
    } catch (e) { if (gen === generation.current) setError(message(e)) }
  }
  return <section role="region" aria-label="Project tasks" className="absolute inset-x-10 bottom-6 z-40 flex h-80 max-h-[65vh] flex-col overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-xl text-[11px] text-koma-fg" onKeyDown={e => {
    if (e.key === 'Escape') { e.stopPropagation(); close() }
  }}>
    <div className="flex h-8 flex-none items-center gap-2 border-b border-koma-border px-2">
      <Terminal size={12} className="text-koma-dim" /><span className="text-koma-dim">TASKS</span>
      <select aria-label="Task workspace" value={scope} onChange={e => {
        generation.current++
        setWorkspace(workspaces.find(w => workspaceKey(w) === e.target.value) ?? null)
      }} className="min-w-0 flex-1 bg-koma-panel outline-none">
        {!workspace && <option value="">Choose a workspace</option>}
        {workspaces.map(w => <option key={workspaceKey(w)} value={workspaceKey(w)}>{w.hostId} · {w.root}</option>)}
      </select>
      <button className={button} title="Refresh tasks from disk" aria-label="Refresh tasks" onClick={() => setRefresh(n => n + 1)}><RefreshCw size={12} /></button>
      <button className={button} title="Open task configuration" aria-label="Open task configuration" disabled={!workspace || currentHost !== workspace.hostId} onClick={() => void editConfig()}><FileCog size={12} /></button>
      <button ref={closeButton} className={button} title="Close tasks (processes keep running)" aria-label="Close tasks" onClick={close}><X size={12} /></button>
    </div>
    <div className="flex flex-none items-center gap-1 border-b border-koma-border px-2 py-1">
      {(['run', 'build', 'test'] as const).map(value => <button key={value} aria-pressed={value === group} onClick={() => setGroup(value)} className={`${button} ${value === group ? 'bg-koma-hover text-koma-fg' : ''}`}>{value[0].toUpperCase() + value.slice(1)}</button>)}
      <select aria-label="Task" value={task?.id ?? ''} onChange={e => setTaskId(e.target.value)} className="min-w-0 flex-1 bg-koma-panel outline-none">
        {!task && <option value="">{definitions ? 'No tasks configured' : configError ? 'Configuration unavailable' : 'Loading tasks…'}</option>}
        {choices.map(t => <option key={t.id} value={t.id}>{t.label}</option>)}
      </select>
      <button className={button} disabled={!task || busy || runs.some(r => r.taskId === task.id && (live(r) || !r.outputComplete))} onClick={() => void act('start')} title="Run task using saved files">{busy ? <BrailleSpinner size={12} /> : <Play size={12} />}Run</button>
    </div>
    {task && <div className="flex-none truncate border-b border-koma-border px-3 py-1 font-mono text-[10px] text-koma-dim" title={JSON.stringify({ command: task.command, args: task.args, cwd: task.cwd, env: Object.keys(task.env ?? {}), timeoutMs: task.timeoutMs })}>
      {task.command} {(task.args ?? []).map(arg => JSON.stringify(arg)).join(' ')} · cwd: {task.cwd ?? '.'} · saved files
    </div>}
    {(configError || error || pollError || outputError) && <div role="alert" className="max-h-16 flex-none overflow-auto border-b border-koma-border px-3 py-1 text-koma-error">{[configError, error, pollError, outputError].filter(Boolean).join(' · ')}</div>}
    {definitions && !choices.length && <div className="flex-none px-3 py-2 text-koma-dim">Add a {group} task to .koma/coding.json using the configuration button.</div>}
    <div className="flex flex-none items-center gap-2 border-b border-koma-border px-2 py-1">
      <select aria-label="Task run output" title={run ? `${run.command} ${run.args.map(arg => JSON.stringify(arg)).join(' ')} · cwd: ${run.cwd}` : undefined} value={runId} onChange={e => setRunId(e.target.value)} className="min-w-0 flex-1 bg-koma-panel outline-none">
        {!runs.length && <option value="">No runs in this workspace</option>}
        {[...runs].reverse().map(r => <option key={r.id} value={r.id}>{r.label} · {r.status}{r.exitCode !== null ? ` (${r.exitCode})` : ''} · {new Date(r.started).toLocaleTimeString()}</option>)}
      </select>
      {!run?.interactive && <label className="flex items-center gap-1 text-koma-dim"><input type="checkbox" checked={follow} onChange={e => setFollow(e.target.checked)} />Follow</label>}
      <button className={button} disabled={!live(run) || busy || run?.status === 'stopping'} title="Stop task and its subprocesses" onClick={() => void act('stop')}><Square size={11} />Stop</button>
    </div>
    {run?.error && <div className="flex-none px-3 py-1 text-koma-error">{run.error}</div>}
    {truncated && <div className="flex-none px-3 text-koma-dim">Earlier output was discarded. Showing retained output.</div>}
    {problems.length > 0 && <details className="max-h-24 flex-none overflow-auto border-b border-koma-border px-3 py-1"><summary className="cursor-pointer text-koma-dim">{problems.length} source locations</summary>{problems.map(p => <button key={`${p.path}:${p.line}:${p.column}`} className="block w-full truncate text-left text-koma-dim hover:text-koma-fg" title={p.message} onClick={() => {
      if (!workspace || !run) return
      if ((useKoma.getState().remoteState.hostId ?? 'local') !== workspace.hostId) { setError('Switch to this host to open task source locations.'); return }
      useKoma.getState().openDiagnostic(new URL(pathToUri(`${workspace.root}/${run.cwd}`, p.path)).toString(), p.line - 1, p.column - 1)
    }}>{p.path}:{p.line}:{p.column} · {p.message}</button>)}</details>}
    {run?.interactive && workspace ? <Suspense fallback={<div className="p-3 text-koma-dim">Loading terminal…</div>}><CodingTaskTerminal key={`${scope}:${runId}`} workspace={workspace} runId={runId} chunks={visibleChunks} running={run.status === 'running'} onError={setError} /></Suspense> : <pre ref={output} tabIndex={0} aria-label="Task output" className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words px-3 py-2 font-mono text-[11px] select-text" onScroll={() => {
      const node = output.current
      if (node && node.scrollHeight - node.scrollTop - node.clientHeight > 40) setFollow(false)
    }}>{visibleChunks.length ? visibleChunks.map(c => <span key={c.seq} className={c.stream === 'stderr' ? 'text-koma-warn' : c.stream === 'system' ? 'text-koma-dim' : ''}>{c.text}</span>) : <span className="text-koma-dim">{live(run) ? 'Waiting for output…' : 'No output'}</span>}</pre>}
  </section>
}
