/** Live selection for the design sidebar. The canvas owns edits; the panel only mirrors this. */
export type DesignUiState = {
  root: string
  path: string
  selection: string[]
  focusId: string | null
}

export type DesignLayerOp =
  | { op: 'select'; id: string; shift: boolean }
  | { op: 'rename'; id: string; name: string }
  | { op: 'visible'; id: string; visible: boolean }
  | { op: 'locked'; id: string; locked: boolean }
  | { op: 'move'; id: string; parentId: string | null; index: number }
  | { op: 'menu'; id: string; x: number; y: number }

let state: DesignUiState | null = null
const listeners = new Set<() => void>()

export function getDesignUi(): DesignUiState | null {
  return state
}

export function subscribeDesignUi(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function publishDesignUi(next: DesignUiState | null) {
  state = next
  for (const listener of listeners) listener()
}

export function emitDesignLayer(root: string, path: string, action: DesignLayerOp) {
  window.dispatchEvent(new CustomEvent('koma-design-layer', { detail: { root, path, action } }))
}
