import type { StoreGet, StoreSet } from '../api'
import type { KomaState } from '../state'

export function remoteActions(set: StoreSet, get: StoreGet): Pick<KomaState, 'requestRemotePath'> {
  return {
  requestRemotePath: () => {
    const s = get()
    // Picker already visible / in-flight — drop the duplicate click.
    if (
      s.remotePath.state === 'listing' ||
      s.remotePath.state === 'ready' ||
      s.remotePath.state === 'error'
    ) {
      return
    }
    // Optimistic open: RemotePathPicker mounts immediately with a braille
    // spinner while the host SSH-lists "~". Host RemotePathPicker push replaces
    // this with the authoritative path/dirs.
    set({
      remotePath: {
        state: 'listing',
        path: '~',
        dirs: [],
        error: null,
      },
    })
    get().req({ r: 'RequestRemotePath' })
  },
  }
}
