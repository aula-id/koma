import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ButtonHTMLAttributes,
  type ReactNode,
} from 'react'
import { useKoma } from '../store/koma'
import {
  gitRequest,
  type GitAction,
  type GitToolTab,
} from '../lib/gitWorkbench'

const titles = {
  diff: 'Diff',
  resolve: 'Resolve',
  rebase: 'Interactive rebase',
  branches: 'Branches',
  tags: 'Tags',
  stashes: 'Stashes',
  reflog: 'Reflog',
  blame: 'Blame',
  remotes: 'Remotes',
}
export function openGitTool(
  view: GitToolTab['view'],
  opts: Partial<
    Pick<
      GitToolTab,
      'root' | 'path' | 'target' | 'staged' | 'commit' | 'oldPath'
    >
  > = {},
) {
  const root = opts.root ?? useKoma.getState().git.root
  if (!root) return
  const id = `gittool:${JSON.stringify([root, view, opts.path, opts.target, opts.commit, opts.staged])}`
  const title = `${titles[view]}${opts.path ? `: ${opts.path.split('/').pop()}` : ''}`
  useKoma.setState((s) => ({
    ui: {
      ...s.ui,
      tabs: s.ui.tabs.some((t) => t.id === id)
        ? s.ui.tabs
        : [...s.ui.tabs, { ...opts, root, id, kind: 'gitTool', view, title }],
      activeTabId: id,
    },
  }))
}
export function setGitDirty(id: string, dirty: boolean) {
  useKoma.setState((s) => ({
    ui: {
      ...s.ui,
      tabs: s.ui.tabs.map((t) =>
        t.id === id && t.kind === 'gitTool' && t.dirty !== dirty
          ? { ...t, dirty }
          : t,
      ),
    },
  }))
}
const locks = new Set<string>()
const listeners = new Set<() => void>()
const emit = () => listeners.forEach((fn) => fn())
const subscribe = (fn: () => void) => {
  listeners.add(fn)
  return () => {
    listeners.delete(fn)
  }
}
const reads = new Set([
  'diff',
  'conflict',
  'commitMessage',
  'stashes',
  'stashInspect',
  'reflog',
  'blame',
  'remotes',
  'rebasePlan',
])
export function useGitWork(root: string) {
  const [loading, setLoading] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const locked = useSyncExternalStore(subscribe, () => locks.has(root))
  const sessionId = useKoma((s) => s.session.id)
  const activeRoot = useKoma((s) => s.activeRepoRoot ?? s.git.root)
  const active = activeRoot === root
  const generation = useRef(0)
  useEffect(() => {
    generation.current++
    return () => {
      generation.current++
    }
  }, [sessionId, root])
  const run = useCallback(
    async <T,>(action: GitAction): Promise<T | undefined> => {
      const s = useKoma.getState()
      if (
        (s.activeRepoRoot ?? s.git.root) !== root ||
        s.session.id !== sessionId
      ) {
        setError('Select this repository before continuing.')
        return
      }
      const mutation = !reads.has(action.kind)
      if (mutation && locks.has(root)) {
        setError('Another Git operation is still running.')
        return
      }
      if (!window.komaIpc) {
        setError('Git operations require the native Koma application.')
        return
      }
      const epoch = generation.current
      if (mutation) {
        locks.add(root)
        emit()
      }
      setLoading((n) => n + 1)
      setError(null)
      try {
        const data = await gitRequest<T>(root, action, s.req)
        if (
          epoch !== generation.current ||
          useKoma.getState().session.id !== sessionId
        )
          return
        return data
      } catch (e) {
        if (epoch === generation.current)
          setError(e instanceof Error ? e.message : String(e))
      } finally {
        setLoading((n) => n - 1)
        if (mutation) {
          locks.delete(root)
          emit()
          const current = useKoma.getState()
          if (
            current.session.id === sessionId &&
            (current.activeRepoRoot ?? current.git.root) === root
          ) {
            current.refreshGitStatus()
            current.refreshBranches()
            current.refreshGraph()
            current.refreshStashes()
          }
        }
      }
    },
    [root, sessionId],
  )
  return { run, busy: loading > 0 || locked, error, setError, active }
}
export type GitWork = ReturnType<typeof useGitWork>
export const buttonClass =
  'inline-flex items-center justify-center gap-1 rounded border border-koma-border px-2 py-1 text-[11px] text-koma-fg hover:bg-koma-hover disabled:cursor-not-allowed disabled:opacity-35'
export function Button(props: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      {...props}
      className={`${buttonClass} ${props.className ?? ''}`}
    />
  )
}
export function Actions({ children }: { children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-1.5 px-3 py-2">
      {children}
    </div>
  )
}
export function Note({ children }: { children: ReactNode }) {
  return <p className="px-3 py-2 text-[11px] text-koma-dim">{children}</p>
}
export function Confirm({
  message,
  label = 'Confirm',
  busy,
  onConfirm,
  onCancel,
}: {
  message: string
  label?: string
  busy?: boolean
  onConfirm: () => void
  onCancel: () => void
}) {
  return (
    <div
      className="flex flex-wrap items-center gap-2 border-y border-koma-warn/40 bg-koma-warn/10 px-3 py-2 text-[11px] text-koma-warn"
      role="alert"
    >
      <span className="min-w-0 flex-1">{message}</span>
      <Button disabled={busy} onClick={onConfirm}>
        {label}
      </Button>
      <Button disabled={busy} onClick={onCancel}>
        Cancel
      </Button>
    </div>
  )
}
export function MessageInput({
  value,
  onChange,
  disabled,
  label = 'Message',
}: {
  value: string
  onChange: (v: string) => void
  disabled?: boolean
  label?: string
}) {
  return (
    <textarea
      aria-label={label}
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value)}
      rows={3}
      className="w-full resize-y rounded border border-koma-border bg-koma-bg px-2 py-1.5 font-mono text-[12px] text-koma-fg outline-none focus:border-koma-grip"
    />
  )
}
export function openCommit(oid: string) {
  const s = useKoma.getState()
  s.openGraphTab()
  s.selectCommit(oid)
}
