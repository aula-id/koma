import type { Tab } from '../store/types/tabs'

export type FollowView = 'diagram' | 'design'

/** Which feature's sidebar view a tab belongs to: its editor tab or its Welcome tab. */
export function tabFollowView(tab: Pick<Tab, 'kind'> & { which?: string } | undefined): FollowView | null {
  if (!tab) return null
  if (tab.kind === 'diagram') return 'diagram'
  if (tab.kind === 'design') return 'design'
  if (tab.kind === 'welcome') return tab.which === 'diagram' || tab.which === 'design' ? tab.which : null
  return null
}

/**
 * Reaction to the active tab changing. Fires only on a transition (a different
 * tab id became active); the same tab staying active never re-forces the view,
 * so a manual activity-icon choice sticks until the active tab changes again.
 * The sidebar is always re-opened because it holds the editor's operations.
 */
export function resolveTabFollow(
  prevTabId: string | undefined,
  next: { id: string; kind: Tab['kind']; which?: string } | undefined,
): { view: FollowView; open: true } | null {
  if (!next || next.id === prevTabId) return null
  const view = tabFollowView(next)
  return view ? { view, open: true } : null
}
