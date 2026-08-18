import { createFileRoute, Link } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/')({
  component: GuiOverview,
})

function GuiOverview() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Desktop GUI</h1>
      <p className="mb-4 text-koma-fg">
        The optional GUI runs as a native desktop window using wry/tao (Rust
        webview). Built with React 19, Tailwind v4, and Zustand, it provides
        a familiar IDE-style interface alongside the full koma agent runtime.
      </p>
      <p className="mb-6 text-koma-dim">
        The GUI shares the same daemon backend as the TUI — sessions, tools,
        providers, and permissions work identically. The difference is the
        interface: panels, tabs, a code editor, and a visual diff viewer.
      </p>

      <h2 className="mb-3 text-lg font-semibold text-koma-fg">Sections</h2>
      <ul className="space-y-2 text-koma-fg">
        <li>
          <Link to="/gui/first-run" className="text-koma-accent hover:underline">
            Tutorial: First Run
          </Link>{' '}
          — step through the onboarding flow.
        </li>
        <li>
          <Link to="/gui/layout" className="text-koma-accent hover:underline">
            GUI Layout
          </Link>{' '}
          — title bar, activity bar, side panel, tab bar, and usage footer.
        </li>
        <li>
          <Link to="/gui/chat-composer" className="text-koma-accent hover:underline">
            Chat &amp; Composer
          </Link>{' '}
          — message input, model picker, file references, and tool approvals.
        </li>
        <li>
          <Link to="/gui/code-editor" className="text-koma-accent hover:underline">
            Code Editor
          </Link>{' '}
          — Monaco-based editor with multi-root file tree.
        </li>
        <li>
          <Link to="/gui/git-diff" className="text-koma-accent hover:underline">
            Git &amp; Diff
          </Link>{' '}
          — source control panel and Monaco diff viewer.
        </li>
        <li>
          <Link to="/gui/import-graph" className="text-koma-accent hover:underline">
            Import Graph
          </Link>{' '}
          — workspace dependency visualization via the linker daemon.
        </li>
        <li>
          <Link to="/gui/extensions" className="text-koma-accent hover:underline">
            Extensions
          </Link>{' '}
          — browsing, installing, and managing extensions.
        </li>
        <li>
          <Link to="/gui/analytics" className="text-koma-accent hover:underline">
            Analytics
          </Link>{' '}
          — usage dashboard, cost tracking, and model breakdown.
        </li>
      </ul>
    </article>
  )
}
