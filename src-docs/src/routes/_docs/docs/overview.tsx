import { createFileRoute } from '@tanstack/react-router'

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
    </article>
  )
}
