import { ExplorePanel } from './panels/ExplorePanel'
import { McpPanel } from './panels/McpPanel'
import { ConnectorPanel } from './panels/ConnectorPanel'
import { UsagePanel } from './panels/UsagePanel'
import { Segmented } from './panels/form'
import { useKoma } from '../store/koma'

export type SidebarView = 'explore' | 'mcp' | 'connector' | 'usage'

type SidebarProps = {
  width: number
  view: SidebarView
}

const TITLES: Record<SidebarView, string> = {
  explore: 'Explorer',
  mcp: 'MCP Servers',
  connector: 'Connector',
  usage: 'Usage',
}

// Sidebar shell: header + the active view's panel. Width from RootLayout state.
export function Sidebar({ width, view }: SidebarProps) {
  const usageScope = useKoma((s) => s.ui.usageScope)
  const setUsageScope = useKoma((s) => s.setUsageScope)
  // No current session (welcome/start screen) — the "session" scope has
  // nothing to filter by, so the toggle is hidden entirely (UsagePanel's own
  // effect forces the scope back to "all" when this goes false).
  const hasSession = useKoma((s) => s.session.id !== null)

  return (
    <div
      style={{ width, flexBasis: width }}
      className="flex min-w-0 flex-none flex-col overflow-hidden border-r border-koma-border bg-koma-panel"
    >
      <div className="flex h-[35px] flex-none items-center justify-between gap-2 px-5 text-[11px] uppercase tracking-wider text-koma-fg opacity-60 whitespace-nowrap">
        <span>{TITLES[view]}</span>
        {view === 'usage' && hasSession && (
          <div className="normal-case tracking-normal">
            <Segmented
              value={usageScope}
              options={[
                { value: 'all', label: 'all' },
                { value: 'session', label: 'session' },
              ]}
              onChange={setUsageScope}
            />
          </div>
        )}
      </div>
      <div className="relative min-h-0 flex-1">
        {view === 'explore' && <ExplorePanel />}
        {view === 'mcp' && <McpPanel />}
        {view === 'connector' && <ConnectorPanel />}
        {view === 'usage' && <UsagePanel />}
      </div>
    </div>
  )
}
