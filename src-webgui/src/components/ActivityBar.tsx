import { Files, Blocks, Plug, Settings } from 'lucide-react'

type ActivityBarProps = {
  sidebarOpen: boolean
  onToggle: () => void
}

const iconBtn =
  'relative flex h-10 w-10 items-center justify-center rounded-md text-koma-fg opacity-50 transition hover:bg-koma-hover hover:opacity-85'

// VSCode-style thin icon strip. Only the top (Explore) button is wired — it
// toggles the sidebar. MCP / Connector / Settings are inert placeholders for
// now; Settings is pinned to the bottom (mt-auto).
export function ActivityBar({ sidebarOpen, onToggle }: ActivityBarProps) {
  return (
    <div className="flex w-12 flex-none flex-col items-center gap-0.5 border-r border-koma-border bg-koma-panel2 pt-1.5">
      <button
        onClick={onToggle}
        title="Explore"
        aria-label="Explore"
        className={`${iconBtn} ${sidebarOpen ? '!opacity-100' : ''}`}
      >
        {sidebarOpen && (
          <span className="absolute left-0 top-2 bottom-2 w-0.5 rounded-sm bg-koma-fg" />
        )}
        <Files size={22} strokeWidth={1.6} />
      </button>
      <button className={iconBtn} title="MCP" aria-label="MCP">
        <Blocks size={22} strokeWidth={1.6} />
      </button>
      <button className={iconBtn} title="Connector" aria-label="Connector">
        <Plug size={22} strokeWidth={1.6} />
      </button>
      <button className={`${iconBtn} mt-auto mb-1.5`} title="Settings" aria-label="Settings">
        <Settings size={22} strokeWidth={1.6} />
      </button>
    </div>
  )
}
