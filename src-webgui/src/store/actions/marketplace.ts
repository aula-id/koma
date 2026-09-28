import type { StoreGet, StoreSet } from '../api'
import type { KomaState } from '../state'
import type { Tab } from '../types/tabs'

export function marketplaceActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'openStoreTab' | 'browseStore' | 'openStoreDetail' | 'closeStoreDetail' | 'openInstalledExtensionTab' | 'installExtension' | 'uninstallExtension' | 'refreshInstalled' | 'clearStoreNotice' | 'openExtensionTab'> {
  return {
  openStoreTab: () => {
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === 'store')
      const tabs: Tab[] = exists ? s.ui.tabs : [...s.ui.tabs, { id: 'store', kind: 'store' }]
      return { ui: { ...s.ui, tabs, activeTabId: 'store' } }
    })
    // No wire fetch here — the StoreTab fires browseStore + refreshInstalled on mount.
  },
  browseStore: (query, category) => {
    // Return to the grid (clear any open detail) and mark it loading.
    set((s) => ({ store: { ...s.store, busy: true, error: null, opResult: null, detail: null } }))
    get().req({ r: 'StoreBrowse', query, category })
  },
  openStoreDetail: (id) => {
    // Grid→detail: null the previous detail so the pane shows a spinner rather
    // than a stale extension while the fetch is in flight.
    set((s) => ({ store: { ...s.store, busy: true, error: null, opResult: null, detail: null } }))
    get().req({ r: 'StoreDetail', id })
  },
  closeStoreDetail: () => {
    set((s) => ({ store: { ...s.store, detail: null, error: null, opResult: null } }))
  },
  openInstalledExtensionTab: (id: string) => { const tabId = `installed-ext:${id}`; set((s) => { const exists = s.ui.tabs.some((t) => t.kind === 'installedExtension' && t.extId === id); const tabs: Tab[] = exists ? s.ui.tabs : [...s.ui.tabs, { id: tabId, kind: 'installedExtension' as const, extId: id, title: id }]; return { ui: { ...s.ui, tabs, activeTabId: tabId }, store: { ...s.store, installedDetail: null, installedDetailRequestId: id, installedDetailLoading: true, installedDetailError: null, opResult: null } } }); get().req({ r: 'GetInstalledExtensionDetail', id }) },
  installExtension: (id, version) => {
    set((s) => ({ store: { ...s.store, pendingOp: id, pendingOpKind: 'install', error: null, opResult: null } }))
    get().req({ r: 'InstallExtension', id, version })
  },
  uninstallExtension: (id) => {
    set((s) => ({ store: { ...s.store, pendingOp: id, pendingOpKind: 'uninstall', error: null, opResult: null } }))
    get().req({ r: 'UninstallExtension', id })
  },
  refreshInstalled: () => {
    get().req({ r: 'ListInstalledExtensions' })
  },
  clearStoreNotice: () => {
    set((s) => ({ store: { ...s.store, error: null, opResult: null } }))
  },
  openExtensionTab: (extId, panelId, title) => {
    const id = `ext:${extId}:${panelId}`
    set((s) => {
      const exists = s.ui.tabs.some((t) => t.id === id)
      const tabs: Tab[] = exists
        ? s.ui.tabs
        : [...s.ui.tabs, { id, kind: 'extension', extId, panelId, title }]
      return { ui: { ...s.ui, tabs, activeTabId: id } }
    })
  },
  }
}
