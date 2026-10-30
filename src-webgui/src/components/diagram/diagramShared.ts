import type { DiagramDoc, DiagramSelection } from '../../lib/diagram'

/** Screen = doc * zoom + offset, in canvas pixels. */
export type DiagramView = { x: number; y: number; zoom: number }

export const DIAGRAM_ZOOM_MIN = 0.1
export const DIAGRAM_ZOOM_MAX = 8
/** Same feel as the designer: a trackpad pinch and a mouse notch both scale smoothly. */
export const DIAGRAM_ZOOM_PER_PIXEL = 0.0015
export const DIAGRAM_ZOOM_STEP = 1.25
/** Screen pixels a press must travel before it becomes a drag. */
export const DIAGRAM_DRAG_THRESHOLD = 3
/** Screen pixels around a line that still count as a hit. */
export const DIAGRAM_HIT_TOLERANCE = 6
export const DIAGRAM_UNDO_CAP = 200

export const DEFAULT_VIEW: DiagramView = { x: 0, y: 0, zoom: 1 }

export function clampZoom(zoom: number): number {
  return Math.min(DIAGRAM_ZOOM_MAX, Math.max(DIAGRAM_ZOOM_MIN, zoom))
}

/** Keeps the doc point under (cx, cy) fixed while the zoom changes. */
export function zoomViewAt(view: DiagramView, cx: number, cy: number, zoom: number): DiagramView {
  const next = clampZoom(zoom)
  const docX = (cx - view.x) / view.zoom
  const docY = (cy - view.y) / view.zoom
  return { zoom: next, x: cx - docX * next, y: cy - docY * next }
}

// Pan and zoom live outside the file, so moving around never dirties it. A remount keeps them.
const views = new Map<string, DiagramView>()

export function savedDiagramView(key: string): DiagramView {
  return views.get(key) ?? DEFAULT_VIEW
}

export function saveDiagramView(key: string, view: DiagramView): void {
  views.set(key, view)
}

export type DiagramHistoryEntry = { doc: DiagramDoc; selection: DiagramSelection }
export type DiagramHistory = { past: DiagramHistoryEntry[]; future: DiagramHistoryEntry[] }

// Undo survives a tab remount (split, move to another group) but not a reload.
const histories = new Map<string, DiagramHistory>()

export function diagramHistory(key: string): DiagramHistory {
  let history = histories.get(key)
  if (!history) {
    history = { past: [], future: [] }
    histories.set(key, history)
  }
  return history
}

export function clearDiagramHistory(key: string): void {
  const history = diagramHistory(key)
  history.past = []
  history.future = []
}
