import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/docs/architecture')({
  component: Architecture,
})

function Architecture() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Architecture</h1>
      <p className="mb-4 text-koma-fg">
        koma follows a daemon + client architecture. The Rust daemon manages
        sessions, tools, and AI provider connections. The TUI and GUI are
        frontends that communicate with the daemon via IPC.
      </p>
      <h2 className="mb-2 mt-6 text-lg font-semibold text-koma-fg">Components</h2>
      <ul className="list-inside list-disc space-y-1 text-koma-dim">
        <li><strong className="text-koma-fg">Daemon</strong> — core runtime, tool execution, provider management</li>
        <li><strong className="text-koma-fg">TUI</strong> — ratatui terminal frontend</li>
        <li><strong className="text-koma-fg">GUI</strong> — wry/tao desktop frontend (React 19)</li>
        <li><strong className="text-koma-fg">Linker</strong> — session-scoped import graph daemon</li>
        <li><strong className="text-koma-fg">Extensions</strong> — plugin system with broker IPC</li>
      </ul>
    </article>
  )
}
