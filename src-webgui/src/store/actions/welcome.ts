import { activeTabKind, shouldOpenWelcome, welcomeTabId, type WelcomeWhich } from '../../lib/welcome'
import type { StoreGet, StoreSet } from '../api'
import { normalizeGroups } from '../editorGroups'
import type { KomaState } from '../state'
import type { Tab } from '../types/tabs'

export function welcomeActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'openWelcomeTab' | 'maybeOpenWelcomeTab'> {
  return {
    openWelcomeTab: (which) => {
      const id = welcomeTabId(which)
      set((s) => {
        const base = normalizeGroups(s.ui)
        const tabs: Tab[] = base.tabs.some((t) => t.id === id) ? base.tabs : [...base.tabs, { id, kind: 'welcome', which }]
        return { ui: normalizeGroups({ ...base, tabs, activeTabId: id }) }
      })
    },
    maybeOpenWelcomeTab: (which) => {
      if (shouldOpenWelcome(which, activeTabKind(get().ui))) get().openWelcomeTab(which)
    },
  }
}
