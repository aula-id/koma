import { isDesignPath } from '../../lib/design'
import { isDiagramPath } from '../../lib/diagram'
import { shouldAutoOpenWelcome, welcomeTabId, type WelcomeWhich } from '../../lib/welcome'
import type { StoreGet, StoreSet } from '../api'
import { fileKey, isMissingCodingDirError } from '../coding'
import { normalizeGroups } from '../editorGroups'
import type { KomaState } from '../state'
import type { Tab } from '../types/tabs'

const POLL_MS = 100
const POLL_LIMIT = 50
const waiting = new Set<WelcomeWhich>()

export function welcomeActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'openWelcomeTab' | 'maybeOpenWelcomeTab'> {
  // File paths of this kind in the active root's .koma, or null while the listing is unknown or failed.
  const knownFiles = (which: WelcomeWhich): string[] | null => {
    const s = get()
    const root = s.coding.activeRoot
    const dir = root ? s.coding.dirs[fileKey(root, '.koma')] : undefined
    if (!dir || dir.loading) return null
    if (dir.error) return isMissingCodingDirError(dir.error) ? [] : null
    const match = which === 'diagram' ? isDiagramPath : isDesignPath
    return dir.entries.filter((e) => !e.isDir && match(e.path)).map((e) => e.path)
  }

  const decide = (which: WelcomeWhich) => {
    if (shouldAutoOpenWelcome(which, get().ui.tabs, knownFiles(which))) get().openWelcomeTab(which)
  }

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
      if (waiting.has(which)) return
      waiting.add(which)
      let polls = 0
      let refreshed = false
      const poll = () => {
        const { activeRoot: root, dirs } = get().coding
        const dir = root ? dirs[fileKey(root, '.koma')] : undefined
        if (root && !dir && !refreshed) {
          refreshed = true
          get().refreshCodingDir(root, '.koma')
        }
        if ((root && dir && !dir.loading) || ++polls >= POLL_LIMIT) {
          waiting.delete(which)
          decide(which)
          return
        }
        setTimeout(poll, POLL_MS)
      }
      poll()
    },
  }
}
