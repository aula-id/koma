import { createFileRoute, Link } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/docs/getting-started')({
  component: GettingStarted,
})

function GettingStarted() {
  return (
    <article className="prose-koma">
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Getting Started</h1>
      <p className="mb-4 text-koma-fg">
        Install koma with a single command:
      </p>
      <pre className="mb-6 rounded border border-koma-border bg-koma-panel p-4 text-sm text-koma-fg">
        <code>curl -fsSL https://koma.run/install.sh | bash</code>
      </pre>
      <p className="mb-8 text-koma-dim">
        This will download the latest binary for your platform and add it to your PATH.
      </p>

      <h2 className="mb-3 text-lg font-semibold text-koma-fg">Next steps</h2>
      <ul className="space-y-2 text-koma-fg">
        <li>
          Run <code className="text-koma-accent">koma</code> to open the terminal UI.
        </li>
        <li>
          <Link to="/docs/tui" className="text-koma-accent hover:underline">
            Explore the TUI
          </Link>{' '}
          — tour the interface and every slash command.
        </li>
        <li>
          <Link to="/docs/keyboard-shortcuts" className="text-koma-accent hover:underline">
            Keyboard Shortcuts
          </Link>{' '}
          — learn the full key map.
        </li>
        <li>
          <Link to="/docs/settings-oauth" className="text-koma-accent hover:underline">
            Connect a provider
          </Link>{' '}
          in Settings → OAuth to start coding.
        </li>
        <li>
          <Link to="/docs/gui" className="text-koma-accent hover:underline">
            Desktop GUI
          </Link>{' '}
          — run koma in a native window instead of the terminal.
        </li>
      </ul>
    </article>
  )
}
