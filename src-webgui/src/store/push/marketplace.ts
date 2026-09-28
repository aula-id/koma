import { postToPanel } from '../../lib/panelBridge'
import type { StoreGet, StoreSet } from '../api'
import type { PushEnvelope } from '../types/envelope'

export function pushMarketplace(set: StoreSet, _get: StoreGet, env: PushEnvelope): boolean {
  switch (env.k) {
      // Store catalogue reply: REPLACE the grid + clear busy. `error` set means
      // the fetch failed (items already empty) — surface it, keep `detail` as-is
      // (a browse is a grid-level action; it doesn't disturb an open detail).
      case 'StoreCatalogue':
        set((s) => ({
          store: { ...s.store, catalogue: env.items, busy: false, error: env.error },
        }))
        break
      // Store detail reply: fill `detail` (null on error) + clear busy. A failed
      // fetch surfaces `error` and leaves the detail null (the tab shows the
      // error), so the grid's own error isn't clobbered by a later detail error
      // only while the detail pane is what's showing.
      case 'StoreItemDetail':
        set((s) => ({
          store: { ...s.store, detail: env.detail, busy: false, error: env.error },
        }))
        break
      // Installed-registry reply (list / post-op re-push): REPLACE `installed`.
      // Also remove any installedExtension tabs whose extId is no longer in the registry,
      // and clear local detail/loading/error if the current detail id was removed.
      case 'InstalledExtensions':
        set((s) => {
          const liveIds = new Set(env.items.map((e) => e.id))
          let tabs = s.ui.tabs
          let activeTabId = s.ui.activeTabId
          const staleTabIds = new Set(
            tabs.filter((t) => t.kind === 'installedExtension' && !liveIds.has(t.extId)).map((t) => t.id)
          )
          if (staleTabIds.size > 0) {
            tabs = tabs.filter((t) => !staleTabIds.has(t.id))
            if (staleTabIds.has(activeTabId)) {
              // Close via existing close-tab fallback: select left neighbor.
              const idx = s.ui.tabs.findIndex((t) => t.id === activeTabId)
              activeTabId = idx > 0 ? s.ui.tabs[idx - 1].id : 'chat'
            }
          }
          const detailRemoved =
            (s.store.installedDetail !== null && !liveIds.has(s.store.installedDetail.id)) ||
            (s.store.installedDetailRequestId !== null && !liveIds.has(s.store.installedDetailRequestId))
          return {
            store: {
              ...s.store,
              installed: env.items,
              installedDetail: detailRemoved ? null : s.store.installedDetail,
              installedDetailRequestId: detailRemoved ? null : s.store.installedDetailRequestId,
              installedDetailLoading: detailRemoved ? false : s.store.installedDetailLoading,
              installedDetailError: detailRemoved ? null : s.store.installedDetailError,
            },
            ui: { ...s.ui, tabs, activeTabId },
          }
        })
        break
      case 'InstalledExtensionDetail':
        set((s) => {
          // Drop stale replies: only apply when the envelope's id matches the current request.
          if (env.id !== s.store.installedDetailRequestId) return s
          const detail = env.detail
          // Enrichment response: detail.storeDetail present means this is the
          // second (online) push. Merge store detail into the existing local
          // detail. The request id is NOT cleared yet — enrichment completes
          // the two-phase lifecycle.
          if (detail?.storeDetail && s.store.installedDetail?.id === env.id) {
            const merged = { ...s.store.installedDetail, storeDetail: detail.storeDetail }
            const tabs = s.ui.tabs.map((t) =>
              t.kind === 'installedExtension' && t.extId === env.id
                ? { ...t, title: merged.name || merged.storeDetail?.name || merged.id }
                : t,
            )
            return {
              store: {
                ...s.store,
                installedDetail: merged,
                installedDetailRequestId: null,
              },
              ui: { ...s.ui, tabs },
            }
          }
          // Initial (local) response: populate detail and clear loading/error,
          // but keep installedDetailRequestId alive so the subsequent
          // enrichment response can pass the stale-reply guard. A new request
          // from a different extension would overwrite the request id first,
          // naturally invalidating a late enrichment for this one.
          const tabs = detail
            ? s.ui.tabs.map((t) =>
                t.kind === 'installedExtension' && t.extId === env.id
                  ? { ...t, title: detail.name || detail.id }
                  : t,
              )
            : s.ui.tabs
          return {
            store: {
              ...s.store,
              installedDetail: env.detail,
              // Keep installedDetailRequestId so enrichment can still match.
              installedDetailLoading: false,
              installedDetailError: env.error,
            },
            ui: { ...s.ui, tabs },
          }
        })
        break
      // Install/uninstall result: clear that card's pendingOp (if it's still the
      // one in flight — guard against a stale reply for a superseded op) and
      // build the `opResult` notice (success confirmation, or the failure
      // message) read by both the grid and detail banners. The authoritative
      // registry refresh is the following InstalledExtensions push.
      case 'ExtensionOpResult':
        set((s) => {
          const stillPending = s.store.pendingOp === env.id
          const label =
            (s.store.detail && s.store.detail.id === env.id && s.store.detail.name) ||
            s.store.catalogue.find((c) => c.id === env.id)?.name ||
            env.id
          const verb = s.store.pendingOpKind === 'uninstall' ? 'Uninstall' : 'Install'
          return {
            store: {
              ...s.store,
              pendingOp: stillPending ? null : s.store.pendingOp,
              pendingOpKind: stillPending ? null : s.store.pendingOpKind,
              opResult: env.ok
                ? { ok: true, message: `${verb}ed ${label}.` }
                : { ok: false, message: env.error ?? `Failed to ${verb.toLowerCase()} ${label}.` },
            },
          }
        })
        break
      // W9 panel bridge: route straight through to the matching panel
      // iframe, never touch store state. An unregistered panel (tab closed,
      // reload racing the reply) makes postToPanel a silent no-op — these
      // are fire-and-forget, never queued.
      case 'ExtPanelReply':
        postToPanel(env.extId, env.panelId, {
          koma: 'host',
          v: 1,
          kind: 'reply',
          reqId: env.reqId,
          ok: env.ok,
          payload: env.payload,
          error: env.error,
        })
        break
      case 'ExtPanelPush':
        postToPanel(env.extId, env.panelId, {
          koma: 'host',
          v: 1,
          kind: 'push',
          payload: env.payload,
        })
        break
    default:
      return false
  }
  return true
}
