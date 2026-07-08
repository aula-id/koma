import { ExplorePanel } from './panels/ExplorePanel'
import { McpPanel } from './panels/McpPanel'
import { ConnectorPanel } from './panels/ConnectorPanel'
import { UsagePanel } from './panels/UsagePanel'

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
  return (
    <div
      style={{ width, flexBasis: width }}
      className="flex min-w-0 flex-none flex-col overflow-hidden border-r border-koma-border bg-koma-panel"
    >
      <div className="flex h-[35px] flex-none items-center px-5 text-[11px] uppercase tracking-wider text-koma-fg opacity-60 whitespace-nowrap">
        {TITLES[view]}
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
