import type { HubCookingEntry, HubHistoryEntry } from '../store/types/session'

export type SessionFolderGroup = {
  key: string
  label: string
  workdir?: string
  currentDir: boolean
  live: HubCookingEntry[]
  history: HubHistoryEntry[]
}

export function folderGroupKey(dirLabel: string | undefined | null): string {
  return dirLabel?.trim() ?? ''
}

/** Client-only grouping of hub rows by local folder path when known, or by folder label for older rows. Other last. */
export function groupRecentByFolder(
  live: HubCookingEntry[],
  history: HubHistoryEntry[],
): SessionFolderGroup[] {
  const map = new Map<string, SessionFolderGroup>()
  const ensure = (dirLabel: string | undefined | null, currentDir?: boolean, workdir?: string) => {
    const label = folderGroupKey(dirLabel)
    const key = workdir || label
    let group = map.get(key)
    if (!group) {
      group = { key, label: label || workdir || 'Other', workdir, currentDir: false, live: [], history: [] }
      map.set(key, group)
    }
    if (currentDir) group.currentDir = true
    return group
  }
  for (const row of live) ensure(row.dirLabel, row.currentDir, row.workdir).live.push(row)
  for (const row of history) ensure(row.dirLabel, row.currentDir, row.workdir).history.push(row)
  return [...map.values()].sort((a, b) => {
    if (a.currentDir !== b.currentDir) return a.currentDir ? -1 : 1
    if (!a.key && b.key) return 1
    if (a.key && !b.key) return -1
    return a.label.localeCompare(b.label)
  })
}
