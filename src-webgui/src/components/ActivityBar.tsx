import { Files, GitBranch, Blocks, Plug, Bot, ChartColumn, CircleHelp, Settings } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import type { SidebarView } from './Sidebar'

type ActivityBarProps = {
  activeView: SidebarView
  sidebarOpen: boolean
  onSelect: (view: SidebarView) => void
  onSettings?: () => void
  onHelp?: () => void
}

const iconBtn =
  'relative flex h-10 w-10 items-center justify-center rounded-md text-koma-fg opacity-50 transition hover:bg-koma-hover hover:opacity-85'

const ITEMS: { view: SidebarView; icon: LucideIcon; label: string }[] = [
  { view: 'explore', icon: Files, label: 'Explore' },
  { view: 'git', icon: GitBranch, label: 'Source Control' },
  { view: 'mcp', icon: Blocks, label: 'MCP' },
  { view: 'connector', icon: Plug, label: 'Connector' },
  { view: 'agents', icon: Bot, label: 'Agents' },
  { view: 'usage', icon: ChartColumn, label: 'Usage' },
  { view: 'store', icon: Blocks, label: 'Extensions' },
]

// Thin icon strip. Selecting a view switches the sidebar panel; the active
// view shows the left indicator bar. Help + Settings are pinned to the bottom
// (both inert re: active-state — neither is a `SidebarView`), Help directly
// above Settings.
export function ActivityBar({ activeView, sidebarOpen, onSelect, onSettings, onHelp }: ActivityBarProps) {
  return (
    <div className="flex w-12 flex-none flex-col items-center gap-0.5 border-r border-koma-border bg-koma-panel2 pt-1.5">
      {ITEMS.map(({ view, icon: Icon, label }) => {
        const active = sidebarOpen && activeView === view
        return (
          <button
            key={view}
            onClick={() => onSelect(view)}
            title={label}
            aria-label={label}
            className={`${iconBtn} ${active ? '!opacity-100' : ''}`}
          >
            {active && <span className="absolute left-0 top-2 bottom-2 w-0.5 rounded-sm bg-koma-fg" />}
            <Icon size={22} strokeWidth={1.6} />
          </button>
        )
      })}
      <button onClick={onHelp} className={`${iconBtn} mt-auto`} title="Help" aria-label="Help">
        <CircleHelp size={22} strokeWidth={1.6} />
      </button>
      <button
        onClick={onSettings}
        className={`${iconBtn} mb-1.5`}
        title="Settings"
        aria-label="Settings"
      >
        <Settings size={22} strokeWidth={1.6} />
      </button>
    </div>
  )
}
