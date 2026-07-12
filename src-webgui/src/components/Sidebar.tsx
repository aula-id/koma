import { useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { BrailleSpinner } from './BrailleSpinner'
import { ExplorePanel } from './panels/ExplorePanel'
import { GitPanel } from './panels/GitPanel'
import { CodingPanel } from './panels/CodingPanel'
import { McpPanel } from './panels/McpPanel'
import { ConnectorPanel } from './panels/ConnectorPanel'
import { AgentsPanel } from './panels/AgentsPanel'
import { UsagePanel } from './panels/UsagePanel'
import { StorePanel } from './panels/StorePanel'
import { Segmented } from './panels/form'
import { useKoma } from '../store/koma'

export type SidebarView = 'explore' | 'git' | 'coding' | 'mcp' | 'connector' | 'agents' | 'usage' | 'store'

type SidebarProps = {
  width: number
  view: SidebarView
}

const TITLES: Record<SidebarView, string> = {
  explore: 'Explorer',
  git: 'Source Control',
  coding: 'Coding',
  mcp: 'MCP Servers',
  connector: 'Connector',
  agents: 'Agents',
  usage: 'Usage',
  store: 'Extensions',
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
  // Set true the instant a UsagePreview req fires (mount, scope/session
  // change, or this button) and cleared when the matching reply is applied
  // (see the 'UsagePreview' push case in the store) — drives the button's
  // spinner for the actual fetch lifecycle, not a fixed timeout.
  const usagePreviewBusy = useKoma((s) => s.usagePreviewBusy)
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
        <span className="opacity-60">{TITLES[view]}</span>
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
            <button
              type="button"
              onClick={refreshUsagePreview}
              disabled={usagePreviewBusy}
              title="Refresh"
              aria-label="Refresh usage"
              className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-60 hover:bg-koma-hover hover:opacity-100 disabled:cursor-wait"
            >
              {usagePreviewBusy ? <BrailleSpinner size={12} /> : <RefreshCw size={12} />}
            </button>
          </div>
        )}
        {view === 'git' && (
          <button
            type="button"
            onClick={handleRefresh}
            disabled={refreshing}
            title="Refresh"
            aria-label="Refresh source control"
            className="flex h-5 w-5 flex-none items-center justify-center rounded normal-case text-koma-fg opacity-60 hover:bg-koma-hover hover:opacity-100 disabled:cursor-wait"
          >
            {refreshing ? <BrailleSpinner size={12} /> : <RefreshCw size={12} />}
          </button>
        )}
      </div>
      <div className="relative min-h-0 flex-1">
        {view === 'explore' && <ExplorePanel />}
        {view === 'git' && <GitPanel />}
        {view === 'coding' && <CodingPanel />}
        {view === 'mcp' && <McpPanel />}
        {view === 'connector' && <ConnectorPanel />}
        {view === 'agents' && <AgentsPanel />}
        {view === 'usage' && <UsagePanel />}
        {view === 'store' && <StorePanel />}
      </div>
    </div>
  )
}
