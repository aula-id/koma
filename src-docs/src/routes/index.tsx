import { createFileRoute, Link } from '@tanstack/react-router'

const CARDS = [
  { to: '/docs/getting-started', title: 'Getting Started', desc: 'Install koma and make your first request.' },
  { to: '/docs/tui', title: 'TUI Commands', desc: 'Every slash command, with live terminal tutorials.' },
  { to: '/docs/keyboard-shortcuts', title: 'Keyboard Shortcuts', desc: 'The full key map for chat and panels.' },
  { to: '/docs/settings-oauth', title: 'OAuth & Providers', desc: 'Connect koma.run, Codex, Claude and more.' },
  { to: '/docs/gui', title: 'Desktop GUI', desc: 'Run koma in a native webview window.' },
  { to: '/docs/overview', title: 'Overview', desc: 'What koma is and how it is built.' },
]

function HomePage() {
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-4xl px-6 py-16">
        <section className="mb-16 text-center">
          <h1 className="mb-4 text-5xl font-bold text-koma-accent">koma</h1>
          <p className="mx-auto mb-8 max-w-xl text-lg text-koma-dim">
            an agent that reads your repo, plans, edits, and runs.
          </p>
          <div className="flex flex-wrap items-center justify-center gap-4">
            <Link
              to="/docs/getting-started"
              className="rounded bg-koma-accent px-6 py-2.5 font-semibold text-koma-bg transition hover:opacity-80"
            >
              Get Started
            </Link>
            <a
              href="https://koma.run"
              target="_blank"
              rel="noopener noreferrer"
              className="rounded border border-koma-border px-6 py-2.5 font-semibold text-koma-fg transition hover:bg-koma-panel"
            >
              Visit koma.run &rarr;
            </a>
          </div>
        </section>

        <section>
          <h2 className="mb-4 text-sm font-semibold uppercase tracking-wider text-koma-dim">
            Explore the docs
          </h2>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {CARDS.map((c) => (
              <Link
                key={c.to}
                to={c.to}
                className="rounded-lg border border-koma-border bg-koma-panel p-5 transition hover:border-koma-accent hover:bg-koma-panel2"
              >
                <h3 className="mb-1 font-semibold text-koma-fg">{c.title}</h3>
                <p className="text-sm text-koma-dim">{c.desc}</p>
              </Link>
            ))}
          </div>
        </section>
      </div>
    </div>
  )
}

export const Route = createFileRoute('/')({
  component: HomePage,
})
