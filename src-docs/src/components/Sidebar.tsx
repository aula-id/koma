import { Link, useLocation } from '@tanstack/react-router'
import { useState, useMemo } from 'react'

// ── Types ───────────────────────────────────────────────────────────────────

interface SidebarItem {
  label: string
  to: string
  desc?: string
}

interface SidebarSection {
  title: string
  defaultOpen?: boolean
  items?: SidebarItem[]
  groups?: { label: string; items: SidebarItem[] }[]
}

interface ProductDef {
  id: string
  label: string
  sections: SidebarSection[]
}

// ── Single source of truth ──────────────────────────────────────────────────

const PRODUCTS: ProductDef[] = [
  {
    id: 'welcome',
    label: 'Welcome',
    sections: [
      {
        title: 'Welcome',
        items: [
          { label: 'Overview', to: '/welcome' },
          { label: 'Quick Start', to: '/welcome/getting-started' },
          { label: 'Architecture', to: '/welcome/architecture' },
        ],
      },
      {
        title: 'Jump To',
        items: [
          { label: 'TUI Documentation', to: '/tui' },
          { label: 'GUI Documentation', to: '/gui' },
        ],
      },
    ],
  },
  {
    id: 'tui',
    label: 'TUI',
    sections: [
      {
        title: 'TUI',
        items: [{ label: 'Overview', to: '/tui' }],
      },
      {
        title: 'Tutorials',
        items: [
          { label: 'First Run', to: '/tui/first-run' },
          { label: 'Provider & Model', to: '/tui/provider-model' },
          { label: 'OAuth', to: '/tui/oauth' },
        ],
      },
      {
        title: 'Settings',
        items: [
          { label: 'Appearance', to: '/tui/settings-appearance', desc: '/settings' },
          { label: 'General', to: '/tui/settings-general', desc: '/settings' },
          { label: 'Providers', to: '/tui/settings-provider', desc: '/settings' },
          { label: 'OAuth', to: '/tui/settings-oauth', desc: '/settings' },
          { label: 'Add Model', to: '/tui/settings-model', desc: '/settings' },
        ],
      },
      {
        title: 'Commands',
        defaultOpen: false,
        groups: [
          {
            label: 'Model & Agents',
            items: [
              { label: '/model', to: '/tui/commands-model', desc: 'switch session model' },
              { label: '/task', to: '/tui/commands-task', desc: 'sub-agents panel' },
              { label: '/agents', to: '/tui/commands-agents', desc: 'agent definitions' },
            ],
          },
          {
            label: 'Session',
            items: [
              { label: '/new', to: '/tui/commands-new', desc: 'spawn session' },
              { label: '/resume', to: '/tui/commands-resume', desc: 'session hub' },
              { label: '/rename', to: '/tui/commands-rename', desc: 'rename session' },
              { label: '/clear', to: '/tui/commands-clear', desc: 'clear history' },
              { label: '/compact', to: '/tui/commands-compact', desc: 'compact context' },
            ],
          },
          {
            label: 'Tools',
            items: [
              { label: '/bash', to: '/tui/commands-bash', desc: 'background jobs' },
              { label: '/todo', to: '/tui/commands-todo', desc: 'task list' },
              { label: '/cd', to: '/tui/commands-cd', desc: 'working directory' },
              { label: '/adddir', to: '/tui/commands-adddir', desc: 'add workspace root' },
              { label: '/attach', to: '/tui/commands-attach', desc: 'attach screenshot' },
            ],
          },
          {
            label: 'Config',
            items: [
              { label: '/mode', to: '/tui/commands-mode', desc: 'cycle or set agent mode' },
              { label: '/effort', to: '/tui/commands-effort', desc: 'reasoning effort' },
              { label: '/free', to: '/tui/commands-free', desc: 'toggle koma-free' },
              { label: '/internet', to: '/tui/commands-internet', desc: 'simple/full mode' },
            ],
          },
          {
            label: 'Extensions',
            items: [
              { label: '/mcp', to: '/tui/commands-mcp', desc: 'MCP servers' },
              { label: '/extension', to: '/tui/commands-extension', desc: 'manage extensions' },
              { label: '/store', to: '/tui/commands-store', desc: 'marketplace' },
              { label: '/skill', to: '/tui/commands-skill', desc: 'agent skills' },
            ],
          },
          {
            label: 'Other',
            items: [
              { label: '/help', to: '/tui/commands-help', desc: 'list commands' },
              { label: '/select', to: '/tui/commands-select', desc: 'dump history' },
              { label: '/security', to: '/tui/commands-security', desc: 'security panel' },
              { label: '/remote', to: '/tui/commands-remote', desc: 'remote SSH hosts' },
              { label: '/usage', to: '/tui/commands-usage', desc: 'cost dashboard' },
              { label: '/quit', to: '/tui/commands-quit', desc: 'quit koma' },
            ],
          },
        ],
      },
      {
        title: 'Reference',
        defaultOpen: false,
        items: [
          { label: 'Keyboard Shortcuts', to: '/tui/keyboard-shortcuts' },
          { label: 'All Commands', to: '/tui/commands-all' },
        ],
      },
    ],
  },
  {
    id: 'gui',
    label: 'GUI',
    sections: [
      {
        title: 'GUI',
        items: [{ label: 'Overview', to: '/gui' }],
      },
      {
        title: 'Tutorials',
        items: [
          { label: 'First Run', to: '/gui/first-run' },
          { label: 'Provider & Model', to: '/gui/provider-model' },
          { label: 'OAuth', to: '/gui/oauth' },
        ],
      },
      {
        title: 'Interface',
        items: [
          { label: 'GUI Layout', to: '/gui/layout' },
          { label: 'Chat & Composer', to: '/gui/chat-composer' },
          { label: 'Code Editor', to: '/gui/code-editor' },
        ],
      },
      {
        title: 'Features',
        items: [
          { label: 'Git & Diff', to: '/gui/git-diff' },
          { label: 'Import Graph', to: '/gui/import-graph' },
          { label: 'Extensions', to: '/gui/extensions' },
          { label: 'Analytics', to: '/gui/analytics' },
        ],
      },
    ],
  },
]

function detectProduct(pathname: string): ProductDef {
  if (pathname.startsWith('/tui')) return PRODUCTS[1]
  if (pathname.startsWith('/gui')) return PRODUCTS[2]
  return PRODUCTS[0]
}

// ── Helpers ─────────────────────────────────────────────────────────────────

function isActive(pathname: string, to: string): boolean {
  return pathname === to || pathname.startsWith(to + '/')
}

function filterItem(item: SidebarItem, q: string): boolean {
  if (!q) return true
  return (
    item.label.toLowerCase().includes(q) ||
    (item.desc?.toLowerCase().includes(q) ?? false)
  )
}

// ── Sub-components ──────────────────────────────────────────────────────────

function ProductSwitcher({ active }: { active: string }) {
  return (
    <div className="flex-none px-3 pt-4 pb-1">
      <div className="flex rounded-md border border-koma-border bg-koma-panel p-0.5">
        {PRODUCTS.map((p) => (
          <Link
            key={p.id}
            to={p.id === 'welcome' ? '/welcome' : `/${p.id}`}
            className={`flex flex-1 items-center justify-center gap-1.5 rounded-[5px] px-2 py-1.5 text-[11px] font-semibold tracking-wide transition-all ${
              active === p.id
                ? 'bg-koma-accent/15 text-koma-accent'
                : 'text-koma-dim hover:text-koma-fg hover:bg-koma-hover'
            }`}
            activeOptions={{ exact: p.id === 'welcome' }}
          >
            {p.label}
          </Link>
        ))}
      </div>
    </div>
  )
}

function SearchBar({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <div className="flex-none px-3 pt-3 pb-2">
      <div className="flex items-center rounded-md border border-koma-border bg-koma-panel px-2.5 py-1.5 text-xs">
        <svg className="mr-2 h-3.5 w-3.5 flex-none text-koma-dim" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="11" cy="11" r="8"/><path d="M21 21l-4.35-4.35"/></svg>
        <input
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="Search..."
          className="w-full bg-transparent text-koma-fg placeholder:text-koma-dim/50 focus:outline-none"
        />
        {value && (
          <button onClick={() => onChange('')} className="ml-1 text-koma-dim hover:text-koma-fg">&times;</button>
        )}
      </div>
    </div>
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

function NavSections({
  sections,
  pathname,
  query,
  openMap,
  activeSection,
  onToggle,
}: {
  sections: SidebarSection[]
  pathname: string
  query: string
  openMap: Record<string, boolean>
  activeSection: string | null
  onToggle: (title: string) => void
}) {
  return (
    <nav className="flex-1 overflow-y-auto px-2 pb-4 pt-1">
      {sections.map((section) => {
        const isOpen = (openMap[section.title] ?? (section.defaultOpen ?? true)) || activeSection === section.title
        const allItems = section.items ?? []
        const groupItems = section.groups?.flatMap((g) => g.items) ?? []
        const hasVisible = query
          ? [...allItems, ...groupItems].some((i) => filterItem(i, query))
          : true
        if (query && !hasVisible) return null

        const isCollapsible = !!section.groups || section.defaultOpen === false

        return (
          <div key={section.title} className="mb-4">
            <button
              onClick={() => isCollapsible && onToggle(section.title)}
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
                  <NavLink key={item.to} item={item} active={isActive(pathname, item.to)} filtered={query ? filterItem(item, query) : true} />
                ))}
                {section.groups?.map((group) => {
                  const visibleItems = query ? group.items.filter((i) => filterItem(i, query)) : group.items
                  if (visibleItems.length === 0) return null
                  return (
                    <div key={group.label} className="mt-2 first:mt-0">
                      <div className="px-2 py-0.5 text-[10px] font-medium text-koma-dim/60">{group.label}</div>
                      {visibleItems.map((item) => (
                        <NavLink key={item.to} item={item} active={isActive(pathname, item.to)} filtered={true} />
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
  )
}

// ── Main sidebar ────────────────────────────────────────────────────────────

export function Sidebar() {
  const location = useLocation()
  const product = detectProduct(location.pathname)
  const [query, setQuery] = useState('')
  const q = query.trim().toLowerCase()

  const [openMap, setOpenMap] = useState<Record<string, boolean>>(() => ({}))

  const activeSection = useMemo(() => {
    for (const s of product.sections) {
      const allItems = s.items ?? []
      const groupItems = s.groups?.flatMap((g) => g.items) ?? []
      if ([...allItems, ...groupItems].some((i) => isActive(location.pathname, i.to))) {
        return s.title
      }
    }
    return null
  }, [location.pathname, product])

  return (
    <aside className="flex h-full w-60 flex-none flex-col overflow-hidden border-r border-koma-border">
      <ProductSwitcher active={product.id} />
      <SearchBar value={query} onChange={setQuery} />
      <NavSections
        sections={product.sections}
        pathname={location.pathname}
        query={q}
        openMap={openMap}
        activeSection={activeSection}
        onToggle={(title) => setOpenMap((m) => ({ ...m, [title]: !m[title] }))}
      />
    </aside>
  )
}
