import { useKoma } from '../store/koma'
import { emptyFileState, fileKey, type FileReadPush } from '../store/coding'
import { codingRequest } from './coding-service'

/** Polling fallback also works over SSH and on filesystems without watch events.
 * Responses are scoped to a host/session and the exact pre-request buffer. */
export function startCodingDiskMonitor() {
  let running = false
  let stopped = false
  const controller = new AbortController()
  const poll = async () => {
    if (running || stopped || document.visibilityState === 'hidden') return
    running = true
    const state = useKoma.getState()
    const hostId = state.remoteState.hostId ?? 'local'
    const generation = state.coding._sessionGen
    const roots = new Map<string, Set<string>>()
    for (const tab of state.ui.tabs) {
      if (tab.kind !== 'codingFile') continue
      const file = state.coding.files[fileKey(tab.root, tab.path)]
      if (!file || file.loading || file.saving || file.content == null || file.binary || file.tooLarge) continue
      if (!roots.has(tab.root)) roots.set(tab.root, new Set())
      roots.get(tab.root)!.add(tab.path)
    }
    const valid = () => !stopped && (useKoma.getState().remoteState.hostId ?? 'local') === hostId && useKoma.getState().coding._sessionGen === generation
    try {
      for (const [root, paths] of roots) {
        const list = [...paths]
        for (let offset = 0; offset < list.length; offset += 32) {
          if (!valid()) return
          const batch = list.slice(offset, offset + 32)
          const before = new Map(batch.map(path => [path, useKoma.getState().coding.files[fileKey(root, path)]]))
          const rows = await codingRequest<Array<{ path: string; fingerprint: string; error: string | null; binary: boolean; tooLarge: boolean }>>({ hostId, root }, { op: 'inspect', paths: batch }, controller.signal)
          for (const row of rows) {
            if (!valid()) return
            const key = fileKey(root, row.path)
            const file = before.get(row.path)
            if (!file || useKoma.getState().coding.files[key] !== file || file.saving || file.loading) continue
            if (!row.error && row.fingerprint === file.fingerprint) continue
            if (file.dirty || file.conflict || row.error || row.binary || row.tooLarge) {
              if (!file.conflict) useKoma.setState(s => ({ coding: { ...s.coding, files: { ...s.coding.files,
                [key]: { ...file, conflict: true, error: row.error ?? 'File changed on disk; local text was kept' },
              } } }))
              continue
            }
            const disk = await codingRequest<Omit<FileReadPush, 'k'>>({ hostId, root }, { op: 'read', path: row.path }, controller.signal)
            if (!valid() || useKoma.getState().coding.files[key] !== file) continue
            if (disk.error || disk.binary || disk.tooLarge || disk.content == null) continue
            useKoma.setState(s => ({ coding: { ...s.coding, files: { ...s.coding.files,
              [key]: emptyFileState({ content: disk.content, savedContent: disk.content, fingerprint: disk.fingerprint }),
            } } }))
            // Keep the already-open language document in sync with an external edit.
            useKoma.getState().req({ r: 'LspDidChange', root, path: row.path, text: disk.content })
          }
        }
      }
    } catch {
      // A disconnected host must not mark every buffer as deleted/conflicted.
      // Reconnect/visibility triggers a new inspect; saves still verify hashes.
    } finally { running = false }
  }
  const focus = () => { void poll() }
  const interval = setInterval(focus, 5000)
  window.addEventListener('focus', focus)
  document.addEventListener('visibilitychange', focus)
  return () => { stopped = true; controller.abort(); clearInterval(interval); window.removeEventListener('focus', focus); document.removeEventListener('visibilitychange', focus) }
}
