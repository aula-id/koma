import type { Tab } from '../store/types/tabs'

export type WelcomeWhich = 'diagram' | 'design'

export const welcomeTabId = (which: WelcomeWhich): 'welcome-diagram' | 'welcome-design' => `welcome-${which}`

/**
 * Auto-open rule for the activity-icon click: no welcome tab and no editor tab
 * of this kind open, and no files of this kind. `filePaths` is already filtered
 * by the caller; `null` means the listing is unknown, which never auto-opens.
 */
export function shouldAutoOpenWelcome(
  which: WelcomeWhich,
  tabs: readonly Pick<Tab, 'id' | 'kind'>[],
  filePaths: readonly string[] | null,
): boolean {
  if (filePaths === null || filePaths.length > 0) return false
  const welcomeId = welcomeTabId(which)
  return !tabs.some((t) => t.kind === which || (t.kind === 'welcome' && t.id === welcomeId))
}

const baseName = (path: string): string => path.slice(path.lastIndexOf('/') + 1)

export function recentWelcomeFiles(paths: readonly string[], limit = 8): string[] {
  return [...paths].sort((a, b) => baseName(a).localeCompare(baseName(b))).slice(0, limit)
}
