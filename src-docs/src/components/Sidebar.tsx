import { Link, useLocation } from '@tanstack/react-router'

const sections = [
  {
    title: 'Introduction',
    items: [
      { label: 'Overview', to: '/docs/overview' },
      { label: 'Getting Started', to: '/docs/getting-started' },
      { label: 'Architecture', to: '/docs/architecture' },
    ],
  },
  {
    title: 'Interfaces',
    items: [
      { label: 'Terminal UI', to: '/docs/tui' },
      { label: 'Desktop GUI', to: '/docs/gui' },
    ],
  },
]

export function Sidebar() {
  const location = useLocation()

  return (
    <aside className="w-56 flex-none overflow-y-auto border-r border-koma-border py-6 pr-2">
      {sections.map((section) => (
        <div key={section.title} className="mb-6">
          <h3 className="mb-2 px-3 text-[11px] font-semibold uppercase tracking-wider text-koma-dim">
            {section.title}
          </h3>
          <nav className="flex flex-col gap-0.5">
            {section.items.map((item) => (
              <Link
                key={item.to}
                to={item.to}
                className={`nav-link ${location.pathname === item.to ? 'nav-link-active' : ''}`}
              >
                {item.label}
              </Link>
            ))}
          </nav>
        </div>
      ))}
    </aside>
  )
}
