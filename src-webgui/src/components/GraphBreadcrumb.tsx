import { Activity, ChartScatter, GitBranch } from 'lucide-react'
import { useKoma } from '../store/koma'

// GK2: the graph tab's breadcrumb bar — a thin header sitting above the graph
// body, visible in BOTH rail and bubble mode. LEFT is light git context (the
// current branch); this is the seed of the fuller GitKraken breadcrumb GK4
// will enrich (ahead/behind, upstream, etc — deliberately not here yet).
// RIGHT is the Rail-line/Bubble mode switch. Kept as its own component so
// GraphTab.tsx doesn't grow further — it's already a big file.
export function GraphBreadcrumb() {
  const branch = useKoma((s) => s.git.branch)
  const detached = useKoma((s) => s.git.detached)
  const graphMode = useKoma((s) => s.graph.graphMode)
  const setGraphMode = useKoma((s) => s.setGraphMode)

  const branchLabel = detached ? 'detached' : (branch ?? '—')

  return (
    <div className="flex flex-none items-center gap-2 border-b border-koma-border px-3 py-1.5 text-[12px]">
      {/* Light git context (LEFT) — the breadcrumb seed; GK4 enriches this. */}
      <GitBranch size={13} className="flex-none text-koma-dim opacity-70" />
      <span className="truncate font-mono text-koma-dim">{branchLabel}</span>

      <span className="flex-1" />

      {/* Rail-line / Bubble mode switch (RIGHT) */}
      <div className="flex flex-none rounded border border-koma-border p-0.5">
        <button
          type="button"
          onClick={() => setGraphMode('rail')}
          title="Rail-line view"
          aria-label="Rail-line view"
          aria-pressed={graphMode === 'rail'}
          className={`flex items-center gap-1 rounded px-2 py-0.5 text-[11px] transition-colors ${
            graphMode === 'rail'
              ? 'bg-koma-accent text-koma-bg opacity-100'
              : 'text-koma-fg opacity-55 hover:opacity-80'
          }`}
        >
          <GitBranch size={12} className="flex-none" />
          Rail-line
        </button>
        <button
          type="button"
          onClick={() => setGraphMode('bubble')}
          title="Bubble view"
          aria-label="Bubble view"
          aria-pressed={graphMode === 'bubble'}
          className={`flex items-center gap-1 rounded px-2 py-0.5 text-[11px] transition-colors ${
            graphMode === 'bubble'
              ? 'bg-koma-accent text-koma-bg opacity-100'
              : 'text-koma-fg opacity-55 hover:opacity-80'
          }`}
        >
          <Activity size={12} className="flex-none" />
          Bubble
        </button>
      </div>
    </div>
  )
}

// GK5 will replace this with the real activity/bubble view. Exported so
// GraphTab can render it directly without an extra prop-drilled placeholder.
export function GraphBubblePlaceholder() {
  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-2 px-6 text-center text-koma-dim">
      <ChartScatter size={28} className="opacity-50" />
      <span className="text-[13px] font-medium opacity-80">Activity view</span>
      <span className="text-[11px] opacity-50">Coming soon</span>
    </div>
  )
}
