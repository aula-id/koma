import { createFileRoute, Link } from '@tanstack/react-router'

const CARDS = [
  { to: '/welcome/getting-started', title: 'Getting Started', desc: 'Install koma and make your first request.' },
  { to: '/tui', title: 'TUI Documentation', desc: 'Terminal commands, tutorials, and keyboard shortcuts.' },
  { to: '/gui', title: 'GUI Documentation', desc: 'Desktop interface, code editor, and visual tools.' },
  { to: '/tui/keyboard-shortcuts', title: 'Keyboard Shortcuts', desc: 'The full key map for chat and panels.' },
  { to: '/tui/settings-oauth', title: 'OAuth & Providers', desc: 'Connect koma.run, Codex, Claude and more.' },
  { to: '/welcome/architecture', title: 'Architecture', desc: 'How koma is built internally.' },
]

function HomePage() {
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-[1100px] px-6 py-16 sm:py-20">
        <section className="mb-16 text-center">
          <h1 className="mb-4 text-4xl font-normal tracking-wide text-koma-fg sm:text-5xl">
            koma
          </h1>
          <p className="mx-auto mb-8 max-w-xl text-[0.9375rem] leading-relaxed text-koma-dim sm:text-lg">
            an agent that reads your repo, plans, edits, and runs.
          </p>
          <div className="flex flex-wrap items-center justify-center gap-3">
            <Link
              to="/welcome/getting-started"
              className="rounded-lg bg-koma-accent px-5 py-2.5 text-[0.8125rem] font-medium text-koma-bg transition hover:opacity-90"
            >
              Get Started
            </Link>
            <a
              href="https://koma.run"
              target="_blank"
              rel="noopener noreferrer"
              className="rounded-lg border border-koma-border px-5 py-2.5 text-[0.8125rem] font-medium text-koma-fg transition hover:bg-koma-hover"
            >
              Visit koma.run &rarr;
            </a>
          </div>
        </section>

        <section>
          <h2 className="mb-4 text-[0.6875rem] font-medium uppercase tracking-widest text-koma-dim">
            Explore the docs
          </h2>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {CARDS.map((c) => (
              <Link
                key={c.to}
                to={c.to}
                className="rounded-xl border border-koma-border bg-koma-panel p-5 transition hover:border-koma-accent/40 hover:bg-koma-panel2"
              >
                <h3 className="mb-1.5 text-[0.9375rem] font-medium tracking-wide text-koma-fg">
                  {c.title}
                </h3>
                <p className="text-[0.8125rem] leading-relaxed text-koma-dim">{c.desc}</p>
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
