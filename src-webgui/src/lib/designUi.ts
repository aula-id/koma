/** Live selection for the design sidebar. The canvas owns edits; the panel only mirrors this. */
export type DesignUiState = {
  root: string
  path: string
  selection: string[]
  focusId: string | null
  overrideTargetId: string | null
  enteredContainerId: string | null
}

export type DesignLayerOp =
  /** `toggle` flips the ids in or out of the selection. Otherwise they replace it. */
  | { op: 'select'; ids: string[]; toggle: boolean }
  | { op: 'enter'; id: string }
  | { op: 'rename'; id: string; name: string }
  | { op: 'visible'; id: string; visible: boolean }
  | { op: 'locked'; id: string; locked: boolean }
  /** Ids run top to bottom as the layers panel lists them. */
  | { op: 'move'; ids: string[]; parentId: string | null; targetId: string | null; place: 'before' | 'after' | 'inside' }
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

/** The node under the pointer, shared by the canvas and the layers panel so each can outline what the other hovers. */
let hover: string | null = null
const hoverListeners = new Set<() => void>()

export function getDesignHover(): string | null {
  return hover
}

export function subscribeDesignHover(listener: () => void): () => void {
  hoverListeners.add(listener)
  return () => hoverListeners.delete(listener)
}

export function setDesignHover(id: string | null) {
  if (hover === id) return
  hover = id
  for (const listener of hoverListeners) listener()
}
