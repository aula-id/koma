import { createFileRoute, Link } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/docs/overview')({
  component: Overview,
})

function Overview() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Overview</h1>
      <p className="mb-4 text-koma-fg">
        koma is a native AI coding agent that operates in the terminal. Written in
        Rust for speed, it features a TUI for orchestrating AI agents that can read
        code, edit files, run commands, and verify changes.
      </p>
      <h2 className="mb-2 mt-6 text-lg font-semibold text-koma-fg">Features</h2>
      <ul className="list-inside list-disc space-y-1 text-koma-dim">
        <li>Parallel sub-agents</li>
        <li>Background jobs</li>
        <li>Multi-session detachable workflows</li>
        <li>Optional desktop GUI (wry/tao)</li>
        <li>MCP tool integration</li>
        <li>Security toolkit</li>
        <li>Self-updating via <code className="text-koma-accent">koma update</code></li>
      </ul>

      <h2 className="mb-3 mt-8 text-lg font-semibold text-koma-fg">Where to next</h2>
      <ul className="space-y-2 text-koma-fg">
        <li>
          <Link to="/docs/getting-started" className="text-koma-accent hover:underline">
            Quick Start
          </Link>{' '}
          — install koma in one command.
        </li>
        <li>
          <Link to="/docs/tui" className="text-koma-accent hover:underline">
            TUI Commands
          </Link>{' '}
          — every slash command, with live terminal tutorials.
        </li>
        <li>
          <Link to="/docs/keyboard-shortcuts" className="text-koma-accent hover:underline">
            Keyboard Shortcuts
          </Link>{' '}
          — the full key map.
        </li>
        <li>
          <Link to="/docs/settings-oauth" className="text-koma-accent hover:underline">
            OAuth &amp; Providers
          </Link>{' '}
          — connect koma.run, Codex, Claude, and more.
        </li>
        <li>
          <Link to="/docs/gui" className="text-koma-accent hover:underline">
            Desktop GUI
          </Link>{' '}
          — run koma in a native webview window.
        </li>
        <li>
          <Link to="/docs/architecture" className="text-koma-accent hover:underline">
            Architecture
          </Link>{' '}
          — how koma is built.
        </li>
      </ul>
    </article>
  )
}
