import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/code-editor')({
  component: GuiCodeEditorPage,
})

function GuiCodeEditorPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Code Editor</h1>
      <p className="mb-6 text-koma-fg">
        The Coding panel provides a Monaco-based code editor with a multi-root
        file tree, tabs, and inline save controls.
      </p>

      <div className="space-y-4 text-sm leading-relaxed text-koma-dim">
        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Multi-Root File Tree</h3>
          <p>
            The Explorer panel in the side bar lists all configured workspace
            roots. Expand directories to browse files. Click a file to open
            it in a new editor tab. The tree reflects the live filesystem.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">File Operations</h3>
          <p>
            Right-click files or directories in the tree for context actions:
            rename, delete, copy path, and reveal in system file manager.
            New files and folders can be created from the toolbar.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Monaco Tabs</h3>
          <p>
            Each open file appears as a tab. The editor supports syntax
            highlighting for all languages Monaco provides, plus minimap,
            find/replace, and bracket matching. Multiple split views are
            supported.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Saving &amp; Autosave</h3>
          <p>
            Files show a dirty indicator (dot on the tab) when modified.
            Save with Ctrl+S. Autosave can be toggled in settings — when
            enabled, changes persist automatically after a short delay.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Conflict / Binary / Large-File States</h3>
          <p>
            If the agent modifies a file you have open, a conflict banner
            offers Reload or Diff options. Binary files open in a read-only
            info panel. Files exceeding the size threshold show a warning
            with an option to open anyway.
          </p>
        </div>
      </div>
    </article>
  )
}
