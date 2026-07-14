import { useEffect } from 'react'
import type { ReactNode } from 'react'
import { Activity, Archive, ArchiveRestore, GitBranch, RefreshCw } from 'lucide-react'
import { useKoma } from '../store/koma'
import { BranchSwitcher } from './BranchSwitcher'
import { GitPushMenu } from './GitPushMenu'

// A compact icon-only toolbar button (Fetch/Push/Stash/Pop) — mirrors
// GitPanel's SyncButton idiom, shrunk to fit the breadcrumb's thin bar.
// `badge` renders a small trailing count (ahead / stash length) when > 0.
function ToolbarButton({
  title,
  onClick,
  disabled,
  badge,
  children,
}: {
  title: string
  onClick: () => void
  disabled?: boolean
  badge?: number
  children: ReactNode
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      className="flex flex-none items-center gap-0.5 rounded px-1.5 py-0.5 text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-30"
    >
      {children}
      {typeof badge === 'number' && badge > 0 && (
        <span className="font-mono text-[10px] leading-none">{badge}</span>
      )}
    </button>
  )
}

// GK2: the graph tab's breadcrumb bar — a thin header sitting above the graph
// body, visible in BOTH rail and bubble mode. LEFT is light git context (the
// current branch). GK4c enriches this with a compact GitKraken-style action
// toolbar (Fetch/Push/Branch/Stash/Pop) between the branch context and the
// Rail-line/Bubble mode switch (RIGHT) — repo-level ops, so they're kept
// visible in both graph modes rather than gated on rail-only.
export function GraphBreadcrumb() {
  const branch = useKoma((s) => s.git.branch)
  const detached = useKoma((s) => s.git.detached)
  const ahead = useKoma((s) => s.git.ahead)
  const staged = useKoma((s) => s.git.staged)
  const unstaged = useKoma((s) => s.git.unstaged)
  const conflicted = useKoma((s) => s.git.conflicted)
  const remoteBusy = useKoma((s) => s.remoteBusy)
  const stashes = useKoma((s) => s.stashes)
  const gitFetch = useKoma((s) => s.gitFetch)
  const gitStash = useKoma((s) => s.gitStash)
  const gitStashPop = useKoma((s) => s.gitStashPop)
  const refreshStashes = useKoma((s) => s.refreshStashes)
  const refreshBranches = useKoma((s) => s.refreshBranches)
  const graphMode = useKoma((s) => s.graph.graphMode)
  const setGraphMode = useKoma((s) => s.setGraphMode)

  const branchLabel = detached ? 'detached' : (branch ?? '—')
  // A clean working tree has nothing to stash — mirrors GitPanel's own
  // `clean` gate (staged/unstaged/conflicted all empty).
  const workTreeClean = staged.length === 0 && unstaged.length === 0 && conflicted.length === 0

  // Fire on mount (tab opened) so the toolbar's counts/enabled-states are
  // correct right away rather than waiting on some OTHER surface (GitPanel/
  // GraphRefTree) to have refreshed them first — GraphRefTree only mounts in
  // rail mode, but this toolbar is visible in both.
  useEffect(() => {
    refreshStashes()
    refreshBranches()
  }, [refreshStashes, refreshBranches])

  return (
    <div className="flex flex-none items-center gap-2 border-b border-koma-border px-3 py-1.5 text-[12px]">
      {/* Light git context (LEFT) — the breadcrumb seed; GK4 enriches this. */}
      <GitBranch size={13} className="flex-none text-koma-dim opacity-70" />
      <span className="truncate font-mono text-koma-dim">{branchLabel}</span>

      {/* GitKraken-style action toolbar (GK4c): Fetch / Push / Branch / Stash / Pop */}
      <div className="flex flex-none items-center gap-0.5 rounded border border-koma-border p-0.5">
        <ToolbarButton title="Fetch" onClick={gitFetch} disabled={!!remoteBusy}>
          <RefreshCw size={12} />
        </ToolbarButton>
        <GitPushMenu compact badge={ahead ?? 0} />
        {/* Reuses BranchSwitcher's own self-contained popover (icon variant) —
            same trigger + portal idiom as GitPanel's header, so "Branch"
            here opens the identical branch-switcher rather than a duplicate. */}
        <BranchSwitcher variant="icon" />
        <ToolbarButton title="Stash changes" onClick={gitStash} disabled={workTreeClean}>
          <Archive size={12} />
        </ToolbarButton>
        <ToolbarButton
          title={stashes.length > 0 ? `Pop stash (${stashes.length})` : 'Pop stash'}
          onClick={gitStashPop}
          disabled={stashes.length === 0}
          badge={stashes.length}
        >
          <ArchiveRestore size={12} />
        </ToolbarButton>
      </div>

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
