import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/tui/commands-remote')({
  component: CommandsRemotePage,
})

function CommandsRemotePage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /remote</h1>
      <p className="mb-6 text-koma-fg">
        <code className="text-koma-fg">/remote</code> opens the remote host manager —
        save SSH hosts, then connect so a session runs on the remote machine while
        your local chat drives it. The agent gets the remote filesystem and tools
        (same tool surface as a local session, bound to the remote workspace).
      </p>

      <div className="mt-2 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key details</h3>
        <p>
          <strong className="text-koma-accent">Hosts</strong> — store user@host, port,
          and identity; pick a host from the panel to open or resume a remote session
          (<code className="text-koma-fg">/new remote</code>,{' '}
          <code className="text-koma-fg">/resume remote</code>).
        </p>
        <p>
          <strong className="text-koma-accent">Sessions</strong> — a connected remote
          runs session work over SSH so tools operate on the remote tree; local TUI/GUI
          remain the front-end.
        </p>
        <p>
          <strong className="text-koma-accent">GUI</strong> — the Activity Bar{' '}
          <em>Remote</em> view mirrors host management in the web client.
        </p>
        <p>
          Implementation: <code className="text-koma-fg">Mode::Remote</code>,{' '}
          <code className="text-koma-fg">view/remote</code>,{' '}
          <code className="text-koma-fg">app/mode/remote</code>, and the{' '}
          <code className="text-koma-fg">/remote</code> command in{' '}
          <code className="text-koma-fg">controller/command.rs</code>.
        </p>
      </div>
    </article>
  )
}
