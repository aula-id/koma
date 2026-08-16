import { Link, useLocation } from '@tanstack/react-router'
import { useState, useMemo, createContext, useContext } from 'react'

// ── Mode context ────────────────────────────────────────────────────────────

export type DocsMode = 'tui' | 'gui'

interface DocsModeCtx {
  mode: DocsMode
  setMode: (m: DocsMode) => void
}

const DocsModeContext = createContext<DocsModeCtx>({ mode: 'tui', setMode: () => {} })

export function useDocsMode() {
  return useContext(DocsModeContext)
}

export { DocsModeContext }

// ── Helpers ─────────────────────────────────────────────────────────────────

function isActive(pathname: string, to: string): boolean {
  return pathname === to || pathname.startsWith(to + '/')
}

interface SidebarItem {
  label: string
  to: string
  desc?: string
}

interface SidebarSection {
  title: string
  defaultOpen?: boolean
  /** Show in these modes. If omitted, shown in all. */
  modes?: DocsMode[]
  items?: SidebarItem[]
  groups?: { label: string; items: SidebarItem[] }[]
}

// ── Navigation data ─────────────────────────────────────────────────────────

const sections: SidebarSection[] = [
  {
    title: 'Getting Started',
    items: [
      { label: 'Overview', to: '/docs/overview' },
      { label: 'Quick Start', to: '/docs/getting-started' },
      { label: 'Architecture', to: '/docs/architecture' },
    ],
  },
  {
    title: 'Terminal UI',
    modes: ['tui'],
    items: [
      { label: 'TUI Overview', to: '/docs/tui' },
      { label: 'Tutorial: First Run', to: '/docs/tutorial-first-run' },
      { label: 'Tutorial: Provider & Model', to: '/docs/tutorial-provider-model' },
      { label: 'Tutorial: OAuth', to: '/docs/tutorial-oauth' },
    ],
  },
  {
    title: 'Desktop GUI',
    modes: ['gui'],
    items: [
      { label: 'GUI Overview', to: '/docs/gui' },
    ],
  },
  // ── TUI-only ────────────────────────────────────────────────────────────
  {
    title: 'Settings',
    modes: ['tui'],
    items: [
      { label: 'Appearance', to: '/docs/settings-appearance', desc: '/settings' },
      { label: 'General', to: '/docs/settings-general', desc: '/settings' },
      { label: 'Providers', to: '/docs/settings-provider', desc: '/settings' },
      { label: 'OAuth', to: '/docs/settings-oauth', desc: '/settings' },
      { label: 'Add Model', to: '/docs/settings-model', desc: '/settings' },
    ],
  },
  {
    title: 'Commands',
    modes: ['tui'],
    defaultOpen: false,
    groups: [
      {
        label: 'Model & Agents',
        items: [
          { label: '/model', to: '/docs/commands-model', desc: 'switch session model' },
          { label: '/task', to: '/docs/commands-task', desc: 'sub-agents panel' },
          { label: '/agents', to: '/docs/commands-agents', desc: 'agent definitions' },
        ],
      },
      {
        label: 'Session',
        items: [
          { label: '/new', to: '/docs/commands-new', desc: 'spawn session' },
          { label: '/resume', to: '/docs/commands-resume', desc: 'session hub' },
          { label: '/rename', to: '/docs/commands-rename', desc: 'rename session' },
          { label: '/clear', to: '/docs/commands-clear', desc: 'clear history' },
          { label: '/compact', to: '/docs/commands-compact', desc: 'compact context' },
        ],
      },
      {
        label: 'Tools',
        items: [
          { label: '/bash', to: '/docs/commands-bash', desc: 'background jobs' },
          { label: '/todo', to: '/docs/commands-todo', desc: 'task list' },
          { label: '/cd', to: '/docs/commands-cd', desc: 'working directory' },
          { label: '/adddir', to: '/docs/commands-adddir', desc: 'add workspace root' },
          { label: '/attach', to: '/docs/commands-attach', desc: 'attach screenshot' },
        ],
      },
      {
        label: 'Config',
        items: [
          { label: '/mode', to: '/docs/commands-mode', desc: 'cycle or set agent mode' },
          { label: '/effort', to: '/docs/commands-effort', desc: 'reasoning effort' },
          { label: '/free', to: '/docs/commands-free', desc: 'toggle koma-free' },
          { label: '/internet', to: '/docs/commands-internet', desc: 'simple/full mode' },
        ],
      },
      {
        label: 'Extensions',
        items: [
          { label: '/mcp', to: '/docs/commands-mcp', desc: 'MCP servers' },
          { label: '/extension', to: '/docs/commands-extension', desc: 'manage extensions' },
          { label: '/store', to: '/docs/commands-store', desc: 'marketplace' },
          { label: '/skill', to: '/docs/commands-skill', desc: 'agent skills' },
        ],
      },
      {
        label: 'Other',
        items: [
          { label: '/help', to: '/docs/commands-help', desc: 'list commands' },
          { label: '/select', to: '/docs/commands-select', desc: 'dump history' },
          { label: '/security', to: '/docs/commands-security', desc: 'security panel' },
          { label: '/remote', to: '/docs/commands-remote', desc: 'remote SSH hosts' },
          { label: '/usage', to: '/docs/commands-usage', desc: 'cost dashboard' },
          { label: '/quit', to: '/docs/commands-quit', desc: 'quit koma' },
        ],
      },
    ],
  },
  {
    title: 'Reference',
    modes: ['tui'],
    defaultOpen: false,
    items: [
      { label: 'Keyboard Shortcuts', to: '/docs/keyboard-shortcuts' },
      { label: 'All Commands', to: '/docs/commands-all' },
    ],
  },
  // ── GUI-only ────────────────────────────────────────────────────────────
  {
    title: 'GUI Tutorials',
    modes: ['gui'],
    items: [
      { label: 'Overview', to: '/docs/gui' },
    ],
  },
]

// ── Sidebar component ───────────────────────────────────────────────────────

export function Sidebar() {
  const location = useLocation()
  const { mode, setMode } = useDocsMode()
  const [query, setQuery] = useState('')

  const [openMap, setOpenMap] = useState<Record<string, boolean>>(() => {
    const m: Record<string, boolean> = {}
    for (const s of sections) {
      m[s.title] = s.defaultOpen ?? true
    }
    return m
  })

  // Filter sections by current mode
  const visibleSections = useMemo(
    () => sections.filter((s) => !s.modes || s.modes.includes(mode)),
    [mode],
  )

  const activeSection = useMemo(() => {
    for (const s of visibleSections) {
      const allItems = s.items ?? []
      const groupItems = s.groups?.flatMap((g) => g.items) ?? []
      if ([...allItems, ...groupItems].some((i) => isActive(location.pathname, i.to))) {
        return s.title
      }
    }
    return null
  }, [location.pathname, visibleSections])

  const toggle = (title: string) => {
    setOpenMap((m) => ({ ...m, [title]: !m[title] }))
  }

  const q = query.trim().toLowerCase()

  const filterMatch = (item: SidebarItem): boolean => {
    if (!q) return true
    return (
      item.label.toLowerCase().includes(q) ||
      (item.desc?.toLowerCase().includes(q) ?? false)
    )
  }

  return (
    <aside className="flex h-full w-60 flex-none flex-col overflow-hidden border-r border-koma-border">
      {/* Mode selector */}
      <div className="flex-none px-3 pt-4 pb-1">
        <div className="flex rounded-md border border-koma-border bg-koma-panel p-0.5">
          <ModeButton
            active={mode === 'tui'}
            onClick={() => setMode('tui')}
            icon={
              <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <rect x="2" y="3" width="20" height="14" rx="2" />
                <path d="M8 21h8M12 17v4" />
              </svg>
            }
            label="TUI"
          />
          <ModeButton
            active={mode === 'gui'}
            onClick={() => setMode('gui')}
            icon={
              <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <rect x="3" y="3" width="18" height="18" rx="3" />
                <path d="M3 9h18M9 3v18" />
              </svg>
            }
            label="GUI"
          />
        </div>
      </div>

      {/* Search */}
      <div className="flex-none px-3 pt-3 pb-2">
        <div className="flex items-center rounded-md border border-koma-border bg-koma-panel px-2.5 py-1.5 text-xs">
          <svg className="mr-2 h-3.5 w-3.5 flex-none text-koma-dim" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="11" cy="11" r="8"/><path d="M21 21l-4.35-4.35"/></svg>
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search..."
            className="w-full bg-transparent text-koma-fg placeholder:text-koma-dim/50 focus:outline-none"
          />
          {q && (
            <button onClick={() => setQuery('')} className="ml-1 text-koma-dim hover:text-koma-fg">&times;</button>
          )}
        </div>
      </div>

      {/* Nav */}
      <nav className="flex-1 overflow-y-auto px-2 pb-4 pt-1">
        {visibleSections.map((section) => {
          const isOpen = openMap[section.title] || activeSection === section.title
          const allItems = section.items ?? []
          const groupItems = section.groups?.flatMap((g) => g.items) ?? []
          const hasVisibleItems = q
            ? [...allItems, ...groupItems].some(filterMatch)
            : true
          if (q && !hasVisibleItems) return null

          const isCollapsible = !!section.groups || section.defaultOpen === false

          return (
            <div key={section.title} className="mb-4">
              <button
                onClick={() => isCollapsible && toggle(section.title)}
                className={`mb-1 flex w-full items-center gap-1.5 px-2 py-1 text-left text-[11px] font-semibold uppercase tracking-wider transition-colors ${
                  isCollapsible ? 'cursor-pointer hover:text-koma-fg' : 'cursor-default'
                } text-koma-dim`}
              >
                {isCollapsible && (
                  <svg
                    className={`h-3 w-3 flex-none transition-transform ${isOpen ? 'rotate-90' : ''}`}
                    viewBox="0 0 24 24"
                    fill="currentColor"
                  >
                    <path d="M8 5v14l11-7z" />
                  </svg>
                )}
                {section.title}
              </button>

              {isOpen && (
                <div className="flex flex-col gap-0.5">
                  {section.items?.map((item) => (
                    <NavLink key={item.to} item={item} active={isActive(location.pathname, item.to)} filtered={q ? filterMatch(item) : true} />
                  ))}
                  {section.groups?.map((group) => {
                    const visibleItems = q ? group.items.filter(filterMatch) : group.items
                    if (visibleItems.length === 0) return null
                    return (
                      <div key={group.label} className="mt-2 first:mt-0">
                        <div className="px-2 py-0.5 text-[10px] font-medium text-koma-dim/60">{group.label}</div>
                        {visibleItems.map((item) => (
                          <NavLink key={item.to} item={item} active={isActive(location.pathname, item.to)} filtered={true} />
                        ))}
                      </div>
                    )
                  })}
                </div>
              )}
            </div>
          )
        })}
      </nav>
    </aside>
  )
}

// ── Sub-components ──────────────────────────────────────────────────────────

function ModeButton({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean
  onClick: () => void
  icon: React.ReactNode
  label: string
}) {
  return (
    <button
      onClick={onClick}
      className={`flex flex-1 items-center justify-center gap-1.5 rounded-[5px] px-2 py-1.5 text-[11px] font-semibold tracking-wide transition-all ${
        active
          ? 'bg-koma-accent/15 text-koma-accent'
          : 'text-koma-dim hover:text-koma-fg hover:bg-koma-hover'
      }`}
    >
      {icon}
      {label}
    </button>
  )
}

function NavLink({ item, active, filtered }: { item: SidebarItem; active: boolean; filtered: boolean }) {
  if (!filtered) return null
  return (
    <Link
      to={item.to}
      className={`nav-link flex items-baseline gap-2 ${active ? 'nav-link-active' : ''}`}
    >
      <span className="truncate text-[13px]">{item.label}</span>
      {item.desc && (
        <span className="truncate text-[11px] text-koma-dim/50">{item.desc}</span>
      )}
    </Link>
  )
}
