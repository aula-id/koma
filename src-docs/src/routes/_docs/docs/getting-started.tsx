import { createFileRoute } from '@tanstack/react-router'

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
      <p className="text-koma-dim">
        This will download the latest binary for your platform and add it to your PATH.
      </p>
    </article>
  )
}
