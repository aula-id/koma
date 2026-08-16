import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/docs/commands-remote')({
  component: CommandsRemotePage,
})

function CommandsRemotePage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /remote</h1>
      <p className="mb-6 text-koma-fg">
        <code className="text-koma-fg">/remote</code> will manage remote SSH hosts and sessions —
        add, edit, or remove saved hosts, then connect to run a session there. Over SSH the agent
        gets full access to the remote filesystem and tools, with your local chat driving a session
        daemon on the remote machine.
      </p>

      <div className="mb-6 rounded border border-koma-border bg-koma-panel p-6 text-center text-koma-dim">
        /remote — coming soon
      </div>

      <div className="space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">What is planned</h3>
        <p>
          <strong className="text-koma-accent">Hosts</strong> — save remote SSH hosts (user@host,
          port, identity) and pick one from <code className="text-koma-fg">/remote</code> to open a
          session there.
        </p>
        <p>
          <strong className="text-koma-accent">Sessions</strong> — a connected remote runs a session
          daemon so the agent operates on the remote filesystem with the same tools as local chat.
        </p>
        <p>
          <strong className="text-koma-accent">Status</strong> — not yet released. This page documents
          the intended behaviour; the command is not wired into a running build yet.
        </p>
      </div>
    </article>
  )
}
