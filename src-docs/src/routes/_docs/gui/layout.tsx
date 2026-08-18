import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/layout')({
  component: GuiLayoutPage,
})

function GuiLayoutPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">GUI Layout</h1>
      <p className="mb-6 text-koma-fg">
        The desktop GUI window is divided into a title bar, activity bar,
        resizable side panel, main content area with a tab bar, and a usage
        footer.
      </p>

      <div className="space-y-4 text-sm leading-relaxed text-koma-dim">
        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Title Bar</h3>
          <p>
            Shows the workspace name centered in the title bar. Window controls
            (minimize, maximize, close) sit on the right.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Activity Bar</h3>
          <p>
            The narrow vertical bar on the left provides icons for switching
            between panels: Chat, Explorer, Source Control, Import Graph,
            Extensions, and Analytics. Each icon toggles the side panel content.
            The active panel shows a left-edge accent indicator.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Side Panel</h3>
          <p>
            The resizable panel next to the activity bar displays context for
            the selected activity: session list, file explorer, git changes,
            extension details, etc. Drag the panel edge to resize. The panel
            header shows the section title.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Tab Bar</h3>
          <p>
            Open files and diffs appear as tabs above the main content area.
            Click a tab to switch. Dirty (unsaved) tabs show a dot indicator.
            Close tabs with the × button or middle-click.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Main Content</h3>
          <p>
            The central area renders the active tab: chat transcript, code
            editor (Monaco), diff viewer, or the start screen when no session
            is attached.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Usage Footer</h3>
          <p>
            A thin bar at the bottom shows the current session's token usage,
            cost, and model name. Updates in real time during streaming.
          </p>
        </div>
      </div>
    </article>
  )
}
