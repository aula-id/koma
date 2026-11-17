import { useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { BrailleSpinner } from './BrailleSpinner'
import { ExplorePanel } from './panels/ExplorePanel'
import { GitPanel } from './panels/GitPanel'
import { CodingPanel } from './panels/CodingPanel'
import { McpPanel } from './panels/McpPanel'
import { ConnectorPanel } from './panels/ConnectorPanel'
import { AgentsPanel } from './panels/AgentsPanel'
import { SkillsPanel } from './panels/SkillsPanel'
import { UsagePanel } from './panels/UsagePanel'
import { ImportGraphPanel } from './panels/ImportGraphPanel'
import { StorePanel } from './panels/StorePanel'
import { RemotePanel } from './panels/RemotePanel'
import { DiagramPanel } from './panels/DiagramPanel'
import { DesignPanel } from './panels/DesignPanel'
import { Segmented } from './panels/form'
import { useKoma } from '../store/koma'
import { ErrorBoundary } from './ErrorBoundary'

export type SidebarView = 'explore' | 'git' | 'coding' | 'mcp' | 'connector' | 'importGraph' | 'agents' | 'skills' | 'usage' | 'store' | 'remote' | 'diagram' | 'design'

type SidebarProps = {
  width: number
  view: SidebarView
}

const TITLES: Record<SidebarView, string> = {
  explore: 'Chat',
  git: 'Source Control',
  coding: 'Coding',
  mcp: 'MCP Servers',
  connector: 'Connector',
  importGraph: 'Import Graph',
  agents: 'Agents',
  skills: 'Skills',
  usage: 'Usage',
  store: 'Extensions',
  remote: 'Remote',
  diagram: 'Diagram',
  design: 'Design',
}

function HeaderRefresh({ label, busy, onClick }: { label: string; busy?: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      title={label}
      aria-label={label}
      className="flex h-6 w-6 flex-none items-center justify-center rounded normal-case text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100 focus-visible:outline focus-visible:outline-1 focus-visible:outline-koma-accent disabled:cursor-wait"
    >
      {busy ? <BrailleSpinner size={14} /> : <RefreshCw size={14} />}
    </button>
  )
}

// Sidebar shell: header + the active view's panel. Width from RootLayout state.
export function Sidebar({ width, view }: SidebarProps) {
  const usageScope = useKoma((s) => s.ui.usageScope)
  const setUsageScope = useKoma((s) => s.setUsageScope)
  // No current session (welcome/start screen) — the "session" scope has
  // nothing to filter by, so the toggle is hidden entirely (UsagePanel's own
  // effect forces the scope back to "all" when this goes false).
  const hasSession = useKoma((s) => s.session.id !== null)
  const refreshGitStatus = useKoma((s) => s.refreshGitStatus)
  const refreshRepos = useKoma((s) => s.refreshRepos)
  const refreshGraph = useKoma((s) => s.refreshGraph)
  const refreshUsagePreview = useKoma((s) => s.refreshUsagePreview)
  const refreshMcpStatus = useKoma((s) => s.refreshMcpStatus)
  const refreshImportGraph = useKoma((s) => s.refreshImportGraph)
  // Set true the instant a UsagePreview req fires (mount, scope/session
  // change, or this button) and cleared when the matching reply is applied
  // (see the 'UsagePreview' push case in the store) — drives the button's
  // spinner for the actual fetch lifecycle, not a fixed timeout.
  const usagePreviewBusy = useKoma((s) => s.usagePreviewBusy)
  const mcpStatusBusy = useKoma((s) => s.mcpStatusBusy)
  const refreshSkills = useKoma((s) => s.refreshSkills)
  const skillsLoading = useKoma((s) => s.skillsLoading)
  const [refreshing, setRefreshing] = useState(false)

  const handleRefresh = () => {
    setRefreshing(true)
    refreshGitStatus()
    refreshRepos()
    refreshGraph()
    window.setTimeout(() => setRefreshing(false), 800)
  }

  return (
    <div
      style={{ width, flexBasis: width }}
      className="flex min-w-0 flex-none flex-col overflow-hidden border-r border-koma-border bg-koma-panel"
    >
      <div className="flex h-[35px] flex-none items-center justify-between gap-2 px-5 text-[11px] uppercase tracking-wider text-koma-fg whitespace-nowrap">
        <span className="opacity-80">{TITLES[view]}</span>
        {view === 'usage' && (
          <div className="flex items-center gap-1.5 normal-case tracking-normal">
            {hasSession && (
              <Segmented
                value={usageScope}
                options={[
                  { value: 'all', label: 'all' },
                  { value: 'session', label: 'session' },
                ]}
                onChange={setUsageScope}
              />
            )}
            <HeaderRefresh label="Refresh usage" onClick={refreshUsagePreview} busy={usagePreviewBusy} />
          </div>
        )}
        {view === 'git' && (
          <HeaderRefresh label="Refresh source control" onClick={handleRefresh} busy={refreshing} />
        )}
        {view === 'mcp' && (
          <HeaderRefresh label="Refresh MCP status" onClick={refreshMcpStatus} busy={mcpStatusBusy} />
        )}
        {view === 'skills' && (
          <HeaderRefresh label="Rescan skill locations" onClick={refreshSkills} busy={skillsLoading} />
        )}
        {view === 'importGraph' && (
          <HeaderRefresh label="Refresh import graph" onClick={() => refreshImportGraph()} />
        )}
      </div>
      <div className="relative min-h-0 flex-1">
        {view === 'explore' && <ExplorePanel />}
        {view === 'git' && <GitPanel />}
        {view === 'coding' && (
          <ErrorBoundary label="CodingPanel">
            <CodingPanel />
          </ErrorBoundary>
        )}
        {view === 'mcp' && <McpPanel />}
        {view === 'connector' && <ConnectorPanel />}
        {view === 'importGraph' && <ImportGraphPanel />}
        {view === 'agents' && <AgentsPanel />}
        {view === 'skills' && <SkillsPanel />}
        {view === 'usage' && <UsagePanel />}
        {view === 'store' && <StorePanel />}
        {view === 'remote' && <RemotePanel />}
        {view === 'diagram' && <DiagramPanel />}
        {view === 'design' && <DesignPanel />}
      </div>
    </div>
  )
}
