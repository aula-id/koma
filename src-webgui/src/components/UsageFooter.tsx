import { pageRect } from '../lib/uiScale'
import { useEffect, useRef, useState } from 'react'
import { Activity, AlertCircle, AlertTriangle, FoldVertical, Server, Terminal } from 'lucide-react'
import { showCodingTasks } from './CodingTasks'
import { useKoma, visiblePlanTodos } from '../store/koma'
import { BranchSwitcher } from './BranchSwitcher'
import { BrailleSpinner } from './BrailleSpinner'
import { fmtBytes, fmtTokens, UsageDash } from './UsageDash'

// ~20px statusline pinned along the bottom of the whole main area (TUI
// statusline grammar, ported 1:1): mode badge + a live-run pulse on the left,
// the usage readout + compact button on the right. Mounted as the LAST row of
// TabbedMain's flex-col — spans sidebar-edge to window-edge and stays visible
// across chat + diff tabs. Never on the Start Screen / onboarding, since those
// screens render instead of TabbedMain entirely (routes/index.tsx gates it on
// an attached session).
export function UsageFooter() {
  const mode = useKoma((s) => s.session.mode)
  const subagents = useKoma((s) => s.session.subagents)
  const bash = useKoma((s) => s.session.bash)
  const working = useKoma((s) => s.session.working)
  const tokensIn = useKoma((s) => s.session.tokensIn)
  const tokensCached = useKoma((s) => s.session.tokensCached)
  const tokensOut = useKoma((s) => s.session.tokensOut)
  const cost = useKoma((s) => s.session.cost)
  const memWindow = useKoma((s) => s.session.memWindow)
  const memAgent = useKoma((s) => s.session.memAgent)
  const memServices = useKoma((s) => s.session.memServices)
  const memSystem = useKoma((s) => s.session.memSystem)
  const [usageOpen, setUsageOpen] = useState(false)
  const usageRef = useRef<HTMLDivElement>(null)
  const usageMenuRef = useRef<HTMLDivElement>(null)
  const [usageRect, setUsageRect] = useState<DOMRect | null>(null)
  const planTodos = useKoma((s) => s.session.planTodos)
  const focusPlanSection = useKoma((s) => s.focusPlanSection)
  const req = useKoma((s) => s.req)
  const gitBranch = useKoma((s) => s.git.branch)
  const gitDetached = useKoma((s) => s.git.detached)
  const gitError = useKoma((s) => s.git.error)
  const remoteState = useKoma((s) => s.remoteState)
  const errCount = useKoma((s) => s.lspDiagCounts.errors)
  const warnCount = useKoma((s) => s.lspDiagCounts.warnings)
  const tasksOpen = useKoma((s) => s.bottomPanelTab === 'tasks')
  const setBottomPanelTab = useKoma((s) => s.setBottomPanelTab)
  const problemsOpen = useKoma((s) => s.bottomPanelTab === 'problems')
  const toggleProblemsOpen = useKoma((s) => s.toggleProblemsOpen)
  const lspRuntime = useKoma((s) => s.lspRuntime)
  const lspProgress = useKoma((s) => s.lspProgress)
  const lspDrawerOpen = useKoma((s) => s.bottomPanelTab === 'lsp')
  const toggleLspDrawerOpen = useKoma((s) => s.toggleLspDrawerOpen)
  const problemTotal = errCount + warnCount
  const lspBusy =
    lspRuntime.some((s) => s.phase === 'starting' || s.phase === 'working') ||
    Object.values(lspProgress).some((p) => p && !p.error && p.pct < 100)
  const lspError = lspRuntime.some((s) => s.phase === 'error')
  const lspLive = lspRuntime.length
  const lspTitle = lspLive
    ? lspRuntime
        .map((s) => {
          const st =
            s.phase === 'working'
              ? s.title
                ? `${s.title}${s.percentage != null ? ` ${s.percentage}%` : ''}`
                : 'working'
              : s.phase
          return `${s.name}: ${st}`
        })
        .join('\n')
    : 'No language servers running'
  // Live remote target for the statusline chip (hub-ready OR attached-connected).
  const remoteTarget =
    (remoteState.state === 'ready' || remoteState.state === 'connected') &&
    remoteState.user &&
    remoteState.host
      ? `${remoteState.user}@${remoteState.host}`
      : null

  // Awareness pulse: anything currently running in the Explore BASH/AGENTS
  // sidepanel lists — same "running" state token those panels key off.
  const hasActivity =
    subagents.some((a) => a.status === 'running') || bash.some((b) => b.status === 'running')

  const isPlan = mode === 'plan'
  const visiblePlan = visiblePlanTodos(planTodos)
  const planDone = visiblePlan.filter((t) => t.status === 'completed').length
  // "PLAN 3/7" once a checklist exists (locked rails excluded from the count);
  // plain "PLAN" before the model has written one yet (mode flips to plan
  // before the first checklist call lands).
  const planLabel = visiblePlan.length > 0 ? `PLAN ${planDone}/${visiblePlan.length}` : 'PLAN'
  const memTotal = memWindow + memAgent + memServices
  const memKnown = memSystem > 0 || memTotal > 0
  const usageTitle = `↑ ${fmtTokens(tokensIn)} · cached ${fmtTokens(tokensCached)} · ↓ ${fmtTokens(tokensOut)} · $${cost.toFixed(4)}${memKnown ? ` · ${fmtBytes(memTotal)}` : ''}`

  useEffect(() => {
    if (!usageOpen) {
      setUsageRect(null)
      return
    }
    const update = () => {
      if (usageRef.current) setUsageRect(pageRect(usageRef.current.getBoundingClientRect()))
    }
    update()
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node
      if (usageRef.current?.contains(t) || usageMenuRef.current?.contains(t)) return
      setUsageOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setUsageOpen(false)
    }
    window.addEventListener('scroll', update, true)
    window.addEventListener('resize', update)
    window.addEventListener('mousedown', onDoc)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('scroll', update, true)
      window.removeEventListener('resize', update)
      window.removeEventListener('mousedown', onDoc)
      window.removeEventListener('keydown', onKey)
    }
  }, [usageOpen])

  // Viewport max-* only — footer spans the whole main column, not a split pane.
  // Collapse long chips before they shove usage off-screen on narrow windows.
  return (
    <div className="flex h-5 w-full min-w-0 flex-none items-center gap-2 overflow-hidden border-t border-koma-border bg-koma-panel px-3 font-mono text-[11px] text-koma-dim max-[720px]:gap-1.5 max-[720px]:px-2 max-[520px]:gap-1 max-[520px]:px-1.5">
      {/* Mode badge — clickable ONLY in Plan mode: opens the Explore sidebar
          panel and expands its PLAN section (see `focusPlanSection`). */}
      {isPlan ? (
        <button
          onClick={focusPlanSection}
          title="Show plan"
          className="flex-none rounded bg-koma-accent/15 px-1 text-koma-accent transition hover:bg-koma-accent/25"
        >
          {planLabel}
        </button>
      ) : (
        <span className="flex-none lowercase opacity-80">{mode}</span>
      )}

      {/* Remote host chip — visible across welcome/session whenever the GUI is
          bound to an SSH target (remote hub ready, or live remote session). */}
      {remoteTarget && (
        <span
          title={`Remote: ${remoteTarget}`}
          className="flex min-w-0 max-w-[40%] items-center gap-1 truncate rounded bg-koma-accent/10 px-1 text-koma-accent max-[720px]:max-w-[28%] max-[520px]:max-w-none"
        >
          <Server size={10} className="flex-none opacity-80" />
          <span className="truncate max-[520px]:hidden">{remoteTarget}</span>
        </span>
      )}

      {/* Activity pulse — non-interactive, hidden when nothing runs */}
      {hasActivity && <Activity size={12} className="flex-none animate-pulse text-koma-accent" />}

      {/* Current-branch indicator — a clickable branch-switcher trigger.
          Hidden entirely outside a git repo (no error tolerance — a
          stale/unresolved branch name is worse than no indicator) and on
          detached HEAD (no branch name to show as the trigger label). */}
      {!gitError && gitBranch && !gitDetached && (
        <span className="min-w-0 max-[520px]:hidden">
          <BranchSwitcher variant="footer" />
        </span>
      )}

      <div className="min-w-0 flex-1" />

      {/* Usage readout — opens the memory / token card. Short form under
          720px, cost-only under 520px. The card is portaled; this row clips. */}
      <div ref={usageRef} className="relative min-w-0 max-[520px]:flex-none">
        <button
          type="button"
          onClick={() => setUsageOpen((open) => !open)}
          aria-expanded={usageOpen}
          aria-haspopup="dialog"
          aria-label="Koma usage"
          title={usageTitle}
          className="block min-w-0 max-w-full truncate border-0 bg-transparent p-0 text-left font-mono text-[11px] text-koma-dim hover:text-koma-fg"
        >
          <span className="max-[520px]:hidden">
            <span className="max-[720px]:hidden">
              ↑ {fmtTokens(tokensIn)} · cached {fmtTokens(tokensCached)} · ↓ {fmtTokens(tokensOut)} ·{' '}
            </span>
            <span className="hidden max-[720px]:inline">
              ↑{fmtTokens(tokensIn)} ↓{fmtTokens(tokensOut)}{' '}
            </span>
            <span className="text-koma-accent">${cost.toFixed(4)}</span>
            {memKnown && <span className="max-[720px]:hidden"> · {fmtBytes(memTotal)}</span>}
          </span>
          <span className="hidden text-koma-accent max-[520px]:inline">${cost.toFixed(4)}</span>
        </button>
        {usageOpen && usageRect && <UsageDash rect={usageRect} menuRef={usageMenuRef} />}
      </div>

      {/* Compact button */}
      <button
        onClick={() => req({ r: 'Compact' })}
        disabled={working}
        aria-label="Compact context"
        title="Compact context"
        className={`flex h-4 w-4 flex-none items-center justify-center rounded transition-colors ${
          working ? 'text-koma-dim opacity-40' : 'text-koma-dim hover:text-koma-fg'
        }`}
      >
        <FoldVertical size={12} />
      </button>

      {/* Workspace tasks retain their own process/output scope. */}
      <button type="button" onClick={() => tasksOpen ? setBottomPanelTab(null) : showCodingTasks()} aria-expanded={tasksOpen} aria-controls="workspace-bottom-panel" aria-label="Project tasks" title="Run / Build / Test tasks" className={`flex h-4 flex-none items-center gap-1 rounded px-1 ${tasksOpen ? 'bg-koma-accent/15 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}>
        <Terminal size={11} /><span className="max-[720px]:hidden">Tasks</span>
      </button>
      {/* Language Servers badge — live runtime / progress drawer */}
      <button
        type="button"
        onClick={toggleLspDrawerOpen}
        aria-expanded={lspDrawerOpen}
        aria-controls="workspace-bottom-panel"
        aria-label="Language servers"
        title={lspTitle}
        className={`flex h-4 flex-none items-center gap-1 rounded px-1 transition-colors ${
          lspDrawerOpen
            ? 'bg-koma-accent/15 text-koma-accent'
            : lspError
              ? 'text-koma-error hover:bg-koma-hover'
              : lspBusy || lspLive
                ? 'text-koma-fg hover:bg-koma-hover'
                : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
        }`}
      >
        {lspBusy ? (
          <BrailleSpinner size={11} className="text-koma-accent" />
        ) : (
          <Server size={11} className={lspError ? 'text-koma-error' : ''} />
        )}
        <span className="tabular-nums">{lspLive}</span>
      </button>

      {/* Problems badge — always visible; expands the cross-tab drawer */}
      <button
        type="button"
        onClick={toggleProblemsOpen}
        aria-expanded={problemsOpen}
        aria-controls="workspace-bottom-panel"
        aria-label="Problems"
        title={problemTotal ? `${errCount} errors, ${warnCount} warnings` : 'No problems'}
        className={`flex h-4 flex-none items-center gap-1 rounded px-1 transition-colors ${
          problemsOpen
            ? 'bg-koma-accent/15 text-koma-accent'
            : problemTotal
              ? 'text-koma-fg hover:bg-koma-hover'
              : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
        }`}
      >
        {errCount > 0 ? (
          <AlertCircle size={11} className="text-koma-error" />
        ) : (
          <AlertTriangle size={11} className={warnCount ? 'text-koma-warn' : ''} />
        )}
        <span className="tabular-nums">{problemTotal}</span>
      </button>
    </div>
  )
}
