import { Activity, FoldVertical } from 'lucide-react'
import { useKoma } from '../store/koma'

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

// ~20px statusline pinned under the composer (TUI statusline grammar, ported
// 1:1 into the chat column footer): mode badge + a live-run pulse on the left,
// the usage readout + compact button on the right. Mounted as the LAST child
// of ChatView's flex-col — never on the Start Screen / onboarding, since those
// screens render instead of ChatView entirely (routes/index.tsx gates it on
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
  const req = useKoma((s) => s.req)

  // Awareness pulse: anything currently running in the Explore BASH/AGENTS
  // sidepanel lists — same "running" state token those panels key off.
  const hasActivity =
    subagents.some((a) => a.status === 'running') || bash.some((b) => b.status === 'running')

  const isPlan = mode === 'plan'

  return (
    <div className="flex h-5 flex-none items-center gap-2 border-t border-koma-border bg-koma-panel px-2 font-mono text-[11px] text-koma-dim">
      {/* Mode badge */}
      {isPlan ? (
        <span className="rounded bg-koma-accent/15 px-1 text-koma-accent">PLAN</span>
      ) : (
        <span className="lowercase opacity-80">{mode}</span>
      )}

      {/* Activity pulse — non-interactive, hidden when nothing runs */}
      {hasActivity && <Activity size={12} className="flex-none animate-pulse text-koma-accent" />}

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
    </div>
  )
}
