import { useEffect } from 'react'
import { useKoma } from '../store/koma'
import { resolveTabFollow, type FollowView } from './sidebarFollow'

/**
 * Store-level reaction to "active tab changed": subscribes once, compares the
 * active tab id against the previous one and applies resolveTabFollow.
 * Nothing is applied at mount, only on later transitions.
 */
export function useSidebarFollowsActiveTab(apply: (view: FollowView) => void): void {
  useEffect(() => {
    let prev = useKoma.getState().ui.activeTabId
    return useKoma.subscribe((s) => {
      const id = s.ui.activeTabId
      if (id === prev) return
      const before = prev
      prev = id
      const r = resolveTabFollow(before, s.ui.tabs.find((t) => t.id === id))
      if (r) apply(r.view)
    })
  }, [apply])
}
