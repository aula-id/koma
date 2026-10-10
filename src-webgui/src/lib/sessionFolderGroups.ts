import type { HubCookingEntry, HubHistoryEntry } from '../store/types/session'

export type SessionFolderGroup = {
  key: string
  label: string
  currentDir: boolean
  live: HubCookingEntry[]
  history: HubHistoryEntry[]
}

export function folderGroupKey(dirLabel: string | undefined | null): string {
  return dirLabel?.trim() ?? ''
}

/** Client-only grouping of hub rows by folder basename (`dirLabel`). Other last. */
export function groupRecentByFolder(
  live: HubCookingEntry[],
  history: HubHistoryEntry[],
): SessionFolderGroup[] {
  const map = new Map<string, SessionFolderGroup>()
  const ensure = (dirLabel: string | undefined | null, currentDir?: boolean) => {
    const key = folderGroupKey(dirLabel)
    let group = map.get(key)
    if (!group) {
      group = { key, label: key || 'Other', currentDir: false, live: [], history: [] }
      map.set(key, group)
    }
    if (currentDir) group.currentDir = true
    return group
  }
  for (const row of live) ensure(row.dirLabel, row.currentDir).live.push(row)
  for (const row of history) ensure(row.dirLabel, row.currentDir).history.push(row)
  return [...map.values()].sort((a, b) => {
    if (a.currentDir !== b.currentDir) return a.currentDir ? -1 : 1
    if (!a.key && b.key) return 1
    if (a.key && !b.key) return -1
    return a.label.localeCompare(b.label)
  })
}
