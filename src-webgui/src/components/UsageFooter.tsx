import { Activity, AlertCircle, AlertTriangle, FoldVertical, Loader2, Server } from 'lucide-react'
import { useKoma, visiblePlanTodos } from '../store/koma'
import { BranchSwitcher } from './BranchSwitcher'

// Human-compact token count: >=10_000 collapses to "12.4k" (one decimal,
// trailing ".0" trimmed); below that the raw integer is shown. Local helper —
// no dep pulled in for a one-line format.
function fmtTokens(n: number): string {
  if (n >= 10_000) {
    const k = n / 1000
    return `${k.toFixed(1).replace(/\.0$/, '')}k`
  }
  return `${Math.round(n)}`
}

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
  const planTodos = useKoma((s) => s.session.planTodos)
  const focusPlanSection = useKoma((s) => s.focusPlanSection)
  const req = useKoma((s) => s.req)
  const gitBranch = useKoma((s) => s.git.branch)
  const gitDetached = useKoma((s) => s.git.detached)
  const gitError = useKoma((s) => s.git.error)
  const remoteState = useKoma((s) => s.remoteState)
  const lspDiagnostics = useKoma((s) => s.lspDiagnostics)
  const problemsOpen = useKoma((s) => s.problemsOpen)
  const toggleProblemsOpen = useKoma((s) => s.toggleProblemsOpen)
  const lspRuntime = useKoma((s) => s.lspRuntime)
  const lspProgress = useKoma((s) => s.lspProgress)
  const lspDrawerOpen = useKoma((s) => s.lspDrawerOpen)
  const toggleLspDrawerOpen = useKoma((s) => s.toggleLspDrawerOpen)
  let errCount = 0
  let warnCount = 0
  for (const list of Object.values(lspDiagnostics)) {
    for (const d of list) {
      if (d.severity === 1) errCount += 1
      else if (d.severity === 2) warnCount += 1
    }
  }
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

  return (
    <div className="flex h-5 w-full flex-none items-center gap-2 border-t border-koma-border bg-koma-panel px-3 font-mono text-[11px] text-koma-dim">
      {/* Mode badge — clickable ONLY in Plan mode: opens the Explore sidebar
          panel and expands its PLAN section (see `focusPlanSection`). */}
      {isPlan ? (
        <button
          onClick={focusPlanSection}
          title="Show plan"
          className="rounded bg-koma-accent/15 px-1 text-koma-accent transition hover:bg-koma-accent/25"
        >
          {planLabel}
        </button>
      ) : (
        <span className="lowercase opacity-80">{mode}</span>
      )}

      {/* Remote host chip — visible across welcome/session whenever the GUI is
          bound to an SSH target (remote hub ready, or live remote session). */}
      {remoteTarget && (
        <span
          title={`Remote: ${remoteTarget}`}
          className="flex min-w-0 max-w-[40%] items-center gap-1 truncate rounded bg-koma-accent/10 px-1 text-koma-accent"
        >
          <Server size={10} className="flex-none opacity-80" />
          <span className="truncate">{remoteTarget}</span>
        </span>
      )}

      {/* Activity pulse — non-interactive, hidden when nothing runs */}
      {hasActivity && <Activity size={12} className="flex-none animate-pulse text-koma-accent" />}

      {/* Current-branch indicator — a clickable branch-switcher trigger.
          Hidden entirely outside a git repo (no error tolerance — a
          stale/unresolved branch name is worse than no indicator) and on
          detached HEAD (no branch name to show as the trigger label). */}
      {!gitError && gitBranch && !gitDetached && <BranchSwitcher variant="footer" />}

      <div className="flex-1" />

      {/* Usage readout */}
      <span className="truncate">
        ↑ {fmtTokens(tokensIn)} · cached {fmtTokens(tokensCached)} · ↓ {fmtTokens(tokensOut)} ·{' '}
        <span className="text-koma-accent">${cost.toFixed(4)}</span>
      </span>

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

      {/* Language Servers badge — live runtime / progress drawer */}
      <button
        type="button"
        onClick={toggleLspDrawerOpen}
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
          <Loader2 size={11} className="animate-spin text-koma-accent" />
        ) : (
          <Server size={11} className={lspError ? 'text-koma-error' : ''} />
        )}
        <span className="tabular-nums">{lspLive}</span>
      </button>

      {/* Problems badge — always visible; expands the cross-tab drawer */}
      <button
        type="button"
        onClick={toggleProblemsOpen}
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
