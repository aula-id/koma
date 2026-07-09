import { MessageSquare, FileDiff, Settings, CircleHelp, Bot, Terminal, X } from 'lucide-react'
import { useKoma } from '../store/koma'

// Parent directory of a path — used to disambiguate two open tabs that share a
// basename (VSCode-style dim suffix).
function parentDir(path: string): string {
  const parts = path.split('/').filter(Boolean)
  return parts.length > 1 ? parts[parts.length - 2] : ''
}

// VSCode-style tab strip over the main content column. tabs[0] is the permanent,
// uncloseable chat tab; diff tabs open from the Explorer's File-changed rows.
// Hidden entirely until at least one diff tab exists (zero chrome cost until the
// feature is used). Styling matches the app chrome idiom (ActivityBar): panel2
// strip, active row raised onto the canvas bg with a top accent line in fg.
export function TabBar() {
  const tabs = useKoma((s) => s.ui.tabs)
  const activeTabId = useKoma((s) => s.ui.activeTabId)
  const activateTab = useKoma((s) => s.activateTab)
  const closeTab = useKoma((s) => s.closeTab)

  if (tabs.length <= 1) return null

  // Count basenames so a colliding title can show its parent dir.
  const counts = new Map<string, number>()
  for (const t of tabs) {
    if (t.kind === 'diff') counts.set(t.title, (counts.get(t.title) ?? 0) + 1)
  }

  return (
    <div className="flex h-8 flex-none items-stretch overflow-x-auto border-b border-koma-border bg-koma-panel2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
      {tabs.map((t) => {
        const active = t.id === activeTabId
        const base =
          'group relative flex h-full flex-none select-none items-center gap-1.5 border-r border-koma-border text-[12px] transition-colors'
        const tone = active
          ? 'bg-koma-bg text-koma-fg'
          : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
        // Active indicator — a top accent line in fg, matching the ActivityBar's
        // active-view bar.
        const accent = active ? (
          <span className="absolute inset-x-0 top-0 h-0.5 bg-koma-fg" />
        ) : null

        if (t.kind === 'chat') {
          return (
            <button
              key={t.id}
              onClick={() => activateTab(t.id)}
              title="Chat"
              className={`${base} ${tone} px-3`}
            >
              {accent}
              <MessageSquare size={13} className="flex-none" />
              <span>chat</span>
            </button>
          )
        }

        // Settings tab: closeable like a diff tab, with the gear icon + fixed title.
        if (t.kind === 'settings') {
          return (
            <div
              key={t.id}
              role="button"
              tabIndex={0}
              onClick={() => activateTab(t.id)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
              }}
              title="Settings"
              className={`${base} ${tone} cursor-pointer pl-3 pr-1.5`}
            >
              {accent}
              <Settings size={13} className="flex-none opacity-80" />
              <span className="truncate">Settings</span>
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  closeTab(t.id)
                }}
                aria-label="Close tab"
                title="Close"
                className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                  active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                }`}
              >
                <X size={12} />
              </button>
            </div>
          )
        }

        // Help tab: closeable like a diff tab, with a help icon + fixed title.
        // Mirrors the Settings tab block exactly.
        if (t.kind === 'help') {
          return (
            <div
              key={t.id}
              role="button"
              tabIndex={0}
              onClick={() => activateTab(t.id)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
              }}
              title="Help"
              className={`${base} ${tone} cursor-pointer pl-3 pr-1.5`}
            >
              {accent}
              <CircleHelp size={13} className="flex-none opacity-80" />
              <span className="truncate">Help</span>
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  closeTab(t.id)
                }}
                aria-label="Close tab"
                title="Close"
                className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                  active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                }`}
              >
                <X size={12} />
              </button>
            </div>
          )
        }

        // Stream tabs (read-only sub-agent transcript / bash output): closeable like a
        // diff tab, with a Bot / Terminal icon + the title (agent name / truncated cmd).
        if (t.kind === 'subagent' || t.kind === 'bash') {
          const Icon = t.kind === 'subagent' ? Bot : Terminal
          return (
            <div
              key={t.id}
              role="button"
              tabIndex={0}
              onClick={() => activateTab(t.id)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
              }}
              title={t.title}
              className={`${base} ${tone} max-w-[220px] cursor-pointer pl-3 pr-1.5`}
            >
              {accent}
              <Icon size={13} className="flex-none opacity-80" />
              <span className="truncate">{t.title}</span>
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  closeTab(t.id)
                }}
                aria-label="Close tab"
                title="Close"
                className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                  active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                }`}
              >
                <X size={12} />
              </button>
            </div>
          )
        }

        const dir = (counts.get(t.title) ?? 0) > 1 ? parentDir(t.path) : ''
        // A div (not a button) so the close × can nest without invalid
        // button-in-button markup; keyboard-activatable via role/tabIndex.
        return (
          <div
            key={t.id}
            role="button"
            tabIndex={0}
            onClick={() => activateTab(t.id)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
            }}
            title={t.path}
            className={`${base} ${tone} max-w-[220px] cursor-pointer pl-3 pr-1.5`}
          >
            {accent}
            <FileDiff size={13} className="flex-none opacity-80" />
            <span className="truncate">{t.title}</span>
            {dir && <span className="flex-none truncate text-koma-dim opacity-60">{dir}</span>}
            <button
              onClick={(e) => {
                e.stopPropagation()
                closeTab(t.id)
              }}
              aria-label="Close tab"
              title="Close"
              className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
              }`}
            >
              <X size={12} />
            </button>
          </div>
        )
      })}
    </div>
  )
}
