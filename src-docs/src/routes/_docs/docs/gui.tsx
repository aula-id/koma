import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/docs/gui')({
  component: GuiPage,
})

function GuiPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Desktop GUI</h1>
      <p className="mb-4 text-koma-fg">
        The optional GUI runs as a native desktop window using wry/tao (Rust
        webview). Built with React 19, Tailwind v4, and Zustand.
      </p>
      <div className="mb-6 rounded border border-koma-border bg-koma-panel p-6 text-center text-koma-dim">
        GUI Demo — coming soon
      </div>
      <p className="text-koma-dim">
        The demo above replays a scripted session in a faithful recreation of
        the desktop GUI, rendered in your browser.
      </p>
    </article>
  )
}
