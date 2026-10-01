import {
  snapDesign,
  componentView,
  layoutDesign,
  resolveRef,
  syncLinkedComponents,
  writeComponentView,
  type DesignDoc,
  type DesignHandle,
  type DesignNode,
  type DesignOrder,
  type DesignPenPoint,
  type DesignRect,
  type DesignStyle,
  type DesignWeight,
} from '../../lib/design'

export const DESIGN_PATCH_DEBOUNCE_MS = 200
export const UNDO_CAP = 50
export const ZOOM_MIN = 0.25
export const ZOOM_MAX = 64
export const SELECTION = 'var(--koma-accent)'
export const SHAPE_FILL = '#d9d9d9'
export const HANDLES: { id: DesignHandle; x: string; y: string; cursor: string }[] = [
  { id: 'nw', x: '0%', y: '0%', cursor: 'nwse-resize' },
  { id: 'n', x: '50%', y: '0%', cursor: 'ns-resize' },
  { id: 'ne', x: '100%', y: '0%', cursor: 'nesw-resize' },
  { id: 'e', x: '100%', y: '50%', cursor: 'ew-resize' },
  { id: 'se', x: '100%', y: '100%', cursor: 'nwse-resize' },
  { id: 's', x: '50%', y: '100%', cursor: 'ns-resize' },
  { id: 'sw', x: '0%', y: '100%', cursor: 'nesw-resize' },
  { id: 'w', x: '0%', y: '50%', cursor: 'ew-resize' },
]

export type Tool = 'select' | 'frame' | 'rect' | 'ellipse' | 'line' | 'pen' | 'text' | 'pan' | 'polygon' | 'star'
export type DrawShape = 'frame' | 'rect' | 'ellipse' | 'line' | 'text'
export type View = { panX: number; panY: number; zoom: number }
export type Ghost =
  | { kind: 'marquee'; x: number; y: number; w: number; h: number }
  | { kind: 'shape'; shape: DrawShape; x: number; y: number; w: number; h: number; rotation: number }
export type PenDraft = { id: string; parentId: string | null; points: DesignPenPoint[] }
export type DesignCommands = {
  copy: () => void
  paste: (at?: { x: number; y: number }) => void
  duplicate: () => void
  copyStyle: () => void
  pasteStyle: () => void
  selectAll: () => void
  nudge: (dx: number, dy: number, duplicate: boolean) => void
  zoom: (scope: 'all' | 'selection') => void
  remove: () => void
  wrap: (kind: 'group' | 'frame') => void
  unwrap: () => void
  order: (order: DesignOrder) => void
  flip: (axis: 'x' | 'y') => void
  hide: () => void
  lock: () => void
  auto: () => void
  component: () => void
  select: (id: string) => void
  boolean: (op: 'union' | 'subtract' | 'intersect' | 'exclude' | 'flatten') => void
  outline: () => void
  detach: () => void
}
export type RadiusCorner = 'tl' | 'tr' | 'bl' | 'br'
export type Drag =
  | { kind: 'move'; ids: string[]; startX: number; startY: number; origins: Record<string, { x: number; y: number }>; remembered: boolean; moved: boolean; broke: boolean; alt: boolean; scene: { moving: DesignRect; targets: DesignRect[] } | null }
  | { kind: 'resize'; id: string; handle: DesignHandle; startX: number; startY: number; node: DesignNode; remembered: boolean }
  | { kind: 'radius'; id: string; corner: RadiusCorner; startX: number; startY: number; radius: number; node: DesignNode; remembered: boolean }
  | { kind: 'pan'; lastX: number; lastY: number }
  | { kind: 'marquee'; x0: number; y0: number; x1: number; y1: number }
  | { kind: 'draw'; shape: DrawShape; cx: number; cy: number; x0: number; y0: number; x1: number; y1: number; parentId: string | null }
  | { kind: 'pen'; index: number; space: boolean }
  | { kind: 'vertex'; id: string; index: number; startX: number; startY: number }

let copiedShape: { nodes: DesignNode[]; parentId: string | null } | null = null
let copiedStyle: DesignStyle | null = null

export function getCopiedShape() {
  return copiedShape
}

export function setCopiedShape(next: typeof copiedShape) {
  copiedShape = next
}

export function getCopiedStyle() {
  return copiedStyle
}

export function setCopiedStyle(next: typeof copiedStyle) {
  copiedStyle = next
}
let mintSeq = 0

export function drawnBox(x0: number, y0: number, x1: number, y1: number, grid: number, snap: boolean) {
  const x = snapDesign(Math.min(x0, x1), grid, snap)
  const y = snapDesign(Math.min(y0, y1), grid, snap)
  const right = snapDesign(Math.max(x0, x1), grid, snap)
  const bottom = snapDesign(Math.max(y0, y1), grid, snap)
  return { x, y, w: Math.max(0, right - x), h: Math.max(0, bottom - y) }
}

export function lineBox(x0: number, y0: number, x1: number, y1: number) {
  const dx = x1 - x0
  const dy = y1 - y0
  const length = Math.max(1, Math.hypot(dx, dy))
  const rotation = (Math.atan2(dy, dx) * 180) / Math.PI
  return { x: (x0 + x1) / 2 - length / 2, y: (y0 + y1) / 2 - 1, w: length, h: 2, rotation }
}

/** Shift keeps a square. Alt draws the opposite corner around the press point. */
export function shapeCorners(cx: number, cy: number, x: number, y: number, shift: boolean, alt: boolean) {
  let dx = x - cx
  let dy = y - cy
  if (shift) {
    const side = Math.max(Math.abs(dx), Math.abs(dy))
    dx = Math.sign(dx || 1) * side
    dy = Math.sign(dy || 1) * side
  }
  if (alt) return { x0: cx - dx, y0: cy - dy, x1: cx + dx, y1: cy + dy }
  return { x0: cx, y0: cy, x1: cx + dx, y1: cy + dy }
}

/** Shift snaps a line to 45 degrees. Alt extends it through the press point. */
export function lineEnds(cx: number, cy: number, x: number, y: number, shift: boolean, alt: boolean) {
  let dx = x - cx
  let dy = y - cy
  if (shift) {
    const length = Math.hypot(dx, dy)
    const angle = Math.round(Math.atan2(dy, dx) / (Math.PI / 4)) * (Math.PI / 4)
    dx = Math.cos(angle) * length
    dy = Math.sin(angle) * length
  }
  if (alt) return { x0: cx - dx, y0: cy - dy, x1: cx + dx, y1: cy + dy }
  return { x0: cx, y0: cy, x1: cx + dx, y1: cy + dy }
}

export function mintId(prefix: string): string {
  mintSeq += 1
  return `${prefix}${Date.now().toString(36)}${mintSeq}`
}

export function instanceAxes(doc: DesignDoc, node: DesignNode): { name: string; values: string[]; current: string }[] | null {
  if (node.kind !== 'instance' || !node.component) return null
  const component = doc.components.find((item) => item.id === node.component)
  if (!component?.axes) return []
  return Object.entries(component.axes).map(([name, values]) => ({
    name,
    values,
    current: node.variant?.[name] ?? values[0] ?? '',
  }))
}

export function editingDoc(stored: DesignDoc, focusId: string | null): DesignDoc {
  if (!focusId) return stored
  return componentView(stored, focusId) ?? stored
}

export function projectDoc(stored: DesignDoc, focusId: string | null, view: DesignDoc): DesignDoc {
  if (!focusId) return layoutDesign(syncLinkedComponents(view))
  const written = writeComponentView(stored, focusId, view)
  return written ? layoutDesign(written) : stored
}

export function paintCss(doc: DesignDoc, ref: string, fallback: string): string {
  if (ref === 'none') return 'transparent'
  if (!ref) return fallback
  return resolveRef(doc, ref) || fallback
}

export function weightCss(weight: DesignWeight): number {
  if (weight === 'bold') return 700
  if (weight === 'medium') return 500
  return 400
}
