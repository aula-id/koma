import { useKoma } from '../store/koma'
import { pathToUri } from './lsp-bridge'
type Location = { root: string; path: string; line: number; column: number }
const histories = new Map<string, { entries: Location[]; index: number; suppressUntil: number }>()
export function recordCodingLocation(hostId: string, location: Location) {
  if (hostId !== (useKoma.getState().remoteState.hostId ?? 'local')) return
  const history = histories.get(hostId) ?? { entries: [], index: -1, suppressUntil: 0 }
  histories.set(hostId, history)
  if (Date.now() < history.suppressUntil) return
  const previous = history.entries[history.index]
  if (previous?.root === location.root && previous.path === location.path && Math.abs(previous.line - location.line) < 5) {
    history.entries[history.index] = location
  } else {
    history.entries.splice(history.index + 1)
    history.entries.push(location)
    if (history.entries.length > 100) history.entries.shift()
    history.index = history.entries.length - 1
  }
}
export function navigateCodingHistory(direction: -1 | 1) {
  const state = useKoma.getState()
  const history = histories.get(state.remoteState.hostId ?? 'local')
  if (!history) return
  const next = history.index + direction
  if (next < 0 || next >= history.entries.length) return
  history.index = next
  history.suppressUntil = Date.now() + 500
  const location = history.entries[next]
  state.openDiagnostic(pathToUri(location.root, location.path), location.line - 1, location.column - 1)
}
