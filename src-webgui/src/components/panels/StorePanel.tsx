import { useEffect } from 'react'
import { Package, Blocks } from 'lucide-react'
import { useKoma } from '../../store/koma'

// Sidebar launcher for the extension STORE. The store itself is a full tab
// (StoreTab) — this panel is a thin entry point: "Browse extensions" opens the
// tab, and a compact installed list sits below so you can jump straight to the
// store from an installed row. Content lives in the `store` slice; the panel
// fires refreshInstalled on mount so the count is fresh without opening the tab.
export function StorePanel() {
  const installed = useKoma((s) => s.store.installed)
  const openStoreTab = useKoma((s) => s.openStoreTab)
  const openInstalledExtensionTab = useKoma((s) => s.openInstalledExtensionTab)
  const refreshInstalled = useKoma((s) => s.refreshInstalled)

  useEffect(() => {
    refreshInstalled()
  }, [refreshInstalled])

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col overflow-hidden bg-koma-panel">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="p-2">
          <button
            type="button"
            onClick={openStoreTab}
            className="flex w-full items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[12px] text-koma-fg opacity-90 hover:bg-koma-hover hover:opacity-100"
          >
            <Blocks size={14} className="flex-none" />
            <span>Browse extensions</span>
          </button>
        </div>

        <div className="mt-1 px-3 pb-1 pt-2 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
          Installed{installed.length > 0 ? ` (${installed.length})` : ''}
        </div>

        {installed.length === 0 ? (
          <div className="px-3 py-2 text-[11px] leading-snug text-koma-fg opacity-75">
            No extensions installed yet. Extensions add new tools and panels to Koma; use Browse extensions above.
          </div>
        ) : (
          <ul className="flex flex-col">
            {installed.map((ext) => (
              <li key={ext.id}>
                <button
                  onClick={() => openInstalledExtensionTab(ext.id)}
                  title={ext.id}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-koma-hover"
                >
                  {ext.kind === 'daemon' ? (
                    <Package size={13} className="flex-none opacity-70" />
                  ) : (
                    <Blocks size={13} className="flex-none opacity-70" />
                  )}
                  <span className="min-w-0 flex-1 truncate text-[12px] text-koma-fg">{ext.name || ext.id}</span>
                  <span className="flex-none text-[10px] text-koma-dim opacity-70">{ext.version}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}
