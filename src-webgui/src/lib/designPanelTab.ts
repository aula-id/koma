import type { KomaState } from '../store/state'
import type { Tab } from '../store/types/tabs'

/** Design tab shown in the sidebar — not only when the design editor is the active main tab. */
export function resolveDesignPanelTab(s: Pick<KomaState, 'ui' | 'design' | 'coding'>): Extract<Tab, { kind: 'design' }> | null {
  const active = s.ui.tabs.find((item) => item.id === s.ui.activeTabId)
  if (active?.kind === 'design') return active
  const panelId = s.design.panelTabId
  if (panelId) {
    const panel = s.ui.tabs.find((item) => item.id === panelId && item.kind === 'design')
    if (panel && panel.kind === 'design') return panel
  }
  const root = s.coding.activeRoot
  if (root) {
    const fallback = s.ui.tabs.find((item) => item.kind === 'design' && item.root === root)
    if (fallback?.kind === 'design') return fallback
  }
  return null
}
