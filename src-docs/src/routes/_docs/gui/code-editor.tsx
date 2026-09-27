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
            Each open file appears as a tab. Drag tabs between the two editor
            panes, drop a tab on a pane edge while unsplit to create the second
            pane, or right-click a tab and choose Split Right or Split Down.
            Once split, a strip button (or Ctrl+\) flips horizontal ↔ vertical;
            edge drops become move-only. Dividers are resizable. Ctrl+1 / Ctrl+2
            focus a pane. The editor also supports syntax highlighting, minimap,
            find/replace, and bracket matching.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Markdown Preview</h3>
          <p>
            For .md and .markdown files, use the eye button in the editor bar or
            right-click the file or tab and choose Open Markdown Preview. A
            read-only preview tab follows the editor content, including unsaved
            changes. Use Open Source in the preview bar to return to editing.
            Relative images and file links resolve within the workspace.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Language servers (LSP)</h3>
          <p>
            Host-spawned language servers (not Monaco workers) attach when a matching
            server is installed: completion, hover, go-to-definition, references,
            symbols, and diagnostics. Manage installs under Settings → Language
            servers or <code className="text-koma-fg">koma lsp</code>; binaries live
            under <code className="text-koma-fg">~/.koma/lsp/</code> or on PATH. The
            footer Language Servers drawer shows runtime status; Problems lists
            diagnostics. Diff tabs stay syntax-only (no LSP on the diff path).
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Terminal</h3>
          <p>
            Integrated terminal sessions open as main-area tabs (separate from the
            Coding sidebar tree). Use them for local shells alongside the editor.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">
            Saving &amp; Autosave
          </h3>
          <p>
            Files show a dirty indicator (dot on the tab) when modified. Save
            with Ctrl+S. Autosave can be toggled in settings — when enabled,
            changes persist automatically after a short delay. Edits made while
            a save is running stay marked as modified until they are saved.
            Pressing Save again during a write queues the latest buffer; closing
            the editor waits for the pending save.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">
            File Format Preservation
          </h3>
          <p>
            Saving preserves the file’s LF, CRLF, or CR line endings, UTF-8 BOM,
            and BOM-marked UTF-16 encoding and byte order. New files use UTF-8
            and LF. Files with mixed line endings or unsupported encodings are
            not converted automatically; normalize or convert them explicitly
            before editing. For SSH projects, update remote Koma too.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">
            Conflict / Binary / Large-File States
          </h3>
          <p>
            If the file changes on disk before you save, Koma rejects the stale
            save and keeps your local edits. Reload from disk discards those
            edits. Binary files and unsupported encodings are not editable;
            download retains the original bytes. Files larger than 5 MiB cannot
            be edited in this panel.
          </p>
        </div>
      </div>
    </article>
  )
}
