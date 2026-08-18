import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/import-graph')({
  component: GuiImportGraphPage,
})

function GuiImportGraphPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Import Graph</h1>
      <p className="mb-6 text-koma-fg">
        The Import Graph panel visualizes dependencies between source files
        in your workspace. It is powered by the linker daemon, which runs
        as a session-scoped background process.
      </p>

      <div className="space-y-4 text-sm leading-relaxed text-koma-dim">
        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Linker Daemon</h3>
          <p>
            The linker daemon is <strong className="text-koma-accent">default-on</strong>. When
            active, it indexes your source files and tracks import
            relationships in real time. If the daemon is unavailable, the
            graph panel shows a fallback state indicating that graph data
            is not accessible.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Workspace &amp; Language Filters</h3>
          <p>
            Filter the graph by workspace root and programming language.
            Supported languages include Rust, Python, Go, Java, TypeScript,
            and JavaScript. Select one or more to narrow the view.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Reindexing</h3>
          <p>
            The graph reindexes automatically when files change. A manual
            reindex button is available if the automatic update lags behind
            rapid edits.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Dependency &amp; Dependent Exploration</h3>
          <p>
            Select a file node to see its dependencies (files it imports)
            and dependents (files that import it). Expand nodes to traverse
            the graph depth.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Impact Analysis</h3>
          <p>
            Right-click a file to run impact analysis: the graph highlights
            all files that would be affected by changes to the selected file,
            up to a configurable depth.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Graph Navigation</h3>
          <p>
            Click a node to center the view on that file. Double-click to
            open the file in the code editor. Pan and zoom with mouse drag
            and scroll wheel.
          </p>
        </div>
      </div>
    </article>
  )
}
