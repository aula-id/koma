import type { Tab } from '../store/types/tabs'

export type WelcomeWhich = 'diagram' | 'design'

export const welcomeTabId = (which: WelcomeWhich): 'welcome-diagram' | 'welcome-design' => `welcome-${which}`

/**
 * Icon-click rule: always open (or focus) the welcome tab, except when the
 * active tab is already an editor of this kind, so it never steals focus.
 */
export function shouldOpenWelcome(which: WelcomeWhich, activeKind: Tab['kind'] | undefined): boolean {
  return activeKind !== which
}

const baseName = (path: string): string => path.slice(path.lastIndexOf('/') + 1)

export function recentWelcomeFiles(paths: readonly string[], limit = 8): string[] {
  return [...paths].sort((a, b) => baseName(a).localeCompare(baseName(b))).slice(0, limit)
}

/** Kind of the focused tab in the focused editor group, if any. */
export function activeTabKind(ui: { tabs: readonly Tab[]; activeTabId: string }): Tab['kind'] | undefined {
  return ui.tabs.find((t) => t.id === ui.activeTabId)?.kind
}

/** True only while the focused tab is an editor of this kind (not merely open). */
export function isEditorTabActive(kind: 'diagram' | 'design', ui: { tabs: readonly Tab[]; activeTabId: string }): boolean {
  return activeTabKind(ui) === kind
}
