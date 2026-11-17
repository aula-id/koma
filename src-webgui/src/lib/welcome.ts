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
