import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/extensions')({
  component: GuiExtensionsPage,
})

function GuiExtensionsPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Extensions</h1>
      <p className="mb-6 text-koma-fg">
        The Extensions panel lets you browse, install, and manage extensions
        that add capabilities to koma.
      </p>

      <div className="space-y-4 text-sm leading-relaxed text-koma-dim">
        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Browsing &amp; Installing</h3>
          <p>
            The extension marketplace lists available extensions with name,
            description, author, and version. Click Install to add an
            extension. Installed extensions show an Installed badge.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Removing Extensions</h3>
          <p>
            On an installed extension's detail page, click Remove to
            uninstall. Removed extensions are immediately deactivated.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Extension Details</h3>
          <p>
            Clicking an extension opens its detail page showing the full
            description, changelog, and contributed capabilities: activity-bar
            entries, panels, and tools.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Contributed Entries</h3>
          <p>
            Extensions can add activity-bar icons and side-panel views. These
            appear alongside the built-in panels (Chat, Explorer, Source
            Control, etc.) and follow the same interaction patterns.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">MCP &amp; Connector Panels</h3>
          <p>
            MCP servers and provider connectors are managed through their
            own dedicated sections in the activity bar, not through the
            extension store. This keeps provider configuration separate
            from general extensions.
          </p>
        </div>
      </div>
    </article>
  )
}
