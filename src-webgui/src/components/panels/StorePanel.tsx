import { useEffect } from 'react'
import { Package, Blocks } from 'lucide-react'
import { useKoma } from '../../store/koma'

// Sidebar launcher for the extension STORE. The store itself is a full tab
// (StoreTab) — this panel is a thin entry point: "Browse Extensions" opens the
// tab, and a compact installed list sits below so you can jump straight to the
// store from an installed row. Content lives in the `store` slice; the panel
// fires refreshInstalled on mount so the count is fresh without opening the tab.
export function StorePanel() {
  const installed = useKoma((s) => s.store.installed)
  const openStoreTab = useKoma((s) => s.openStoreTab)
  const refreshInstalled = useKoma((s) => s.refreshInstalled)

  useEffect(() => {
    refreshInstalled()
  }, [refreshInstalled])

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <button
        onClick={openStoreTab}
        className="flex items-center gap-2 px-3 py-2 text-left text-[12px] text-koma-fg hover:bg-koma-hover"
      >
        <Blocks size={14} className="flex-none opacity-80" />
        <span>Browse Extensions</span>
      </button>

      <div className="mt-1 px-3 pb-1 pt-2 text-[10px] uppercase tracking-wider text-koma-dim opacity-60">
        Installed{installed.length > 0 ? ` (${installed.length})` : ''}
      </div>

      {installed.length === 0 ? (
        <div className="px-3 py-2 text-[11px] text-koma-dim opacity-70">
          No extensions installed yet.
        </div>
      ) : (
        <ul className="flex flex-col">
          {installed.map((ext) => (
            <li key={ext.id}>
              <button
                onClick={openStoreTab}
                title={ext.id}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-koma-hover"
              >
                {ext.kind === 'daemon' ? (
                  <Package size={13} className="flex-none opacity-70" />
                ) : (
                  <Blocks size={13} className="flex-none opacity-70" />
                )}
                <span className="min-w-0 flex-1 truncate text-[12px] text-koma-fg">{ext.id}</span>
                <span className="flex-none text-[10px] text-koma-dim opacity-70">{ext.version}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
