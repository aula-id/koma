import type { StoreGet, StoreSet } from '../api'
import { useComputerPreview } from '../computerPreview'
import type { PushEnvelope } from '../types/envelope'

export function pushComputer(set: StoreSet, get: StoreGet, env: PushEnvelope): boolean {
  switch (env.k) {
      case 'Computer':
        if (env.status.session === get().session.id) {
          useComputerPreview.getState().syncController(env.status)
          set({ computer: env.status, computerError: null })
        }
        break
      case 'ComputerPreview':
        if (env.frame.request.session === get().session.id) window.dispatchEvent(new CustomEvent('koma-computer-preview', { detail: env.frame }))
        break
      case 'ComputerError':
        if (!get().computer?.enabled) useComputerPreview.getState().hide()
        set({ computerError: env.message }); break
    default:
      return false
  }
  return true
}
