// Diagram documents stored as `<workspace>/.koma/<name>.diag`.
// The canvas and the sidebar both speak this shape. Snap is per file.

export const DIAGRAM_GRID = 16
export const SHAPE_MIME = 'application/x-koma-shape'

export type DiagramKind = 'rect' | 'ellipse' | 'diamond' | 'text'
export type DiagramSide = 'n' | 'e' | 's' | 'w'
export type DiagramHandle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w'
export type DiagramDash = 'solid' | 'dashed' | 'dotted'
export type DiagramArrow = 'none' | 'arrow' | 'open'
export type DiagramCorner = 'sharp' | 'rounded'
export type DiagramRoute = 'orthogonal' | 'straight'
export type DiagramPoint = { x: number; y: number }

export type DiagramNode = {
  id: string
  kind: DiagramKind
  x: number
  y: number
  w: number
  h: number
  text: string
  /** Degrees clockwise. Omitted when the shape is unrotated. */
  rotation?: number
}

export type DiagramEdge = {
  id: string
  from: string
  to: string
  /** False hides the stroke. Omitted means on. */
  stroke?: boolean
  color?: string
  width?: number
  dash?: DiagramDash
  start?: DiagramArrow
  /** Omitted means an end arrow. */
  end?: DiagramArrow
  /** Omitted means sharp corners. */
  corner?: DiagramCorner
  /** Omitted means an orthogonal route. */
  route?: DiagramRoute
  fromSide?: DiagramSide
  toSide?: DiagramSide
  /** Interior waypoints in document coordinates. Endpoints stay on the shapes. */
  bends?: DiagramPoint[]
}

const SIDES: readonly DiagramSide[] = ['n', 'e', 's', 'w']
const DASHES: readonly DiagramDash[] = ['solid', 'dashed', 'dotted']
const ARROWS: readonly DiagramArrow[] = ['none', 'arrow', 'open']
const CORNERS: readonly DiagramCorner[] = ['sharp', 'rounded']
const ROUTES: readonly DiagramRoute[] = ['orthogonal', 'straight']
const MIN_W = 32
const MIN_H = 24

export type DiagramDoc = {
  snap: boolean
  grid: number
  nodes: DiagramNode[]
  edges: DiagramEdge[]
}

const KINDS: readonly DiagramKind[] = ['rect', 'ellipse', 'diamond', 'text']

export function emptyDiagram(): DiagramDoc {
  return { snap: true, grid: DIAGRAM_GRID, nodes: [], edges: [] }
}

export function snap(n: number, grid: number, enabled: boolean): number {
  if (!enabled || !Number.isFinite(grid) || grid <= 0) return n
  return Math.round(n / grid) * grid
}

export function diagramTabId(root: string, path: string): string {
  return `diagram:${root}:${path}`
}

export function isDiagramPath(path: string): boolean {
  return path.startsWith('.koma/') && path.toLowerCase().endsWith('.diag') && !path.slice('.koma/'.length).includes('/')
}

/** Accept a typed file name and force a single `.diag` suffix. */
export function diagramFileName(raw: string): string | null {
  const cleaned = raw.trim().replace(/^\/+|\/+$/g, '')
  if (!cleaned || cleaned.includes('..') || cleaned.includes('/') || cleaned.includes('\\')) return null
  const stem = cleaned.replace(/\.diag$/i, '')
  if (!stem) return null
  return `${stem}.diag`
}

export function defaultNodeSize(kind: DiagramKind): { w: number; h: number; text: string } {
  if (kind === 'text') return { w: 120, h: 36, text: 'text' }
  return { w: 160, h: 64, text: 'label' }
}

function isKind(value: unknown): value is DiagramKind {
  return typeof value === 'string' && (KINDS as readonly string[]).includes(value)
}

function num(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function parseNodes(value: unknown): DiagramNode[] | null {
  if (!Array.isArray(value)) return null
  const nodes: DiagramNode[] = []
  for (const item of value) {
    if (!item || typeof item !== 'object') return null
    const row = item as Record<string, unknown>
    const x = num(row.x)
    const y = num(row.y)
    const w = num(row.w)
    const h = num(row.h)
    if (typeof row.id !== 'string' || !row.id || !isKind(row.kind) || x == null || y == null || w == null || h == null) return null
    if (typeof row.text !== 'string') return null
    const node: DiagramNode = { id: row.id, kind: row.kind, x, y, w, h, text: row.text }
    const rotation = num(row.rotation)
    if (rotation != null && rotation !== 0) node.rotation = ((rotation % 360) + 360) % 360
    nodes.push(node)
  }
  return nodes
}

function oneOf<T extends string>(value: unknown, allowed: readonly T[]): T | undefined {
  return typeof value === 'string' && (allowed as readonly string[]).includes(value) ? (value as T) : undefined
}

function parseColor(value: unknown): string | undefined {
  return typeof value === 'string' && /^#[0-9a-fA-F]{6}$/.test(value) ? value.toLowerCase() : undefined
}

function parseBends(value: unknown): DiagramPoint[] | undefined {
  if (!Array.isArray(value)) return undefined
  const bends: DiagramPoint[] = []
  for (const item of value) {
    if (!item || typeof item !== 'object') return undefined
    const row = item as Record<string, unknown>
    const x = num(row.x)
    const y = num(row.y)
    if (x == null || y == null) return undefined
    bends.push({ x, y })
    if (bends.length >= 32) break
  }
  return bends.length ? bends : undefined
}

function parseEdges(value: unknown): DiagramEdge[] | null {
  if (!Array.isArray(value)) return null
  const edges: DiagramEdge[] = []
  for (const item of value) {
    if (!item || typeof item !== 'object') return null
    const row = item as Record<string, unknown>
    if (typeof row.id !== 'string' || !row.id || typeof row.from !== 'string' || typeof row.to !== 'string') return null
    const edge: DiagramEdge = { id: row.id, from: row.from, to: row.to }
    if (row.stroke === false) edge.stroke = false
    const color = parseColor(row.color)
    const width = num(row.width)
    const dash = oneOf(row.dash, DASHES)
    const start = oneOf(row.start, ARROWS)
    const end = oneOf(row.end, ARROWS)
    const corner = oneOf(row.corner, CORNERS)
    const route = oneOf(row.route, ROUTES)
    const fromSide = oneOf(row.fromSide, SIDES)
    const toSide = oneOf(row.toSide, SIDES)
    const bends = parseBends(row.bends)
    if (color) edge.color = color
    if (width != null && width > 0 && width !== 1.5) edge.width = width
    if (dash && dash !== 'solid') edge.dash = dash
    if (start && start !== 'none') edge.start = start
    if (end && end !== 'arrow') edge.end = end
    if (corner && corner !== 'sharp') edge.corner = corner
    if (route && route !== 'orthogonal') edge.route = route
    if (fromSide) edge.fromSide = fromSide
    if (toSide) edge.toSide = toSide
    if (bends) edge.bends = bends
    edges.push(edge)
  }
  return edges
}

export function parseDiagram(text: string): { doc: DiagramDoc; error: string | null } {
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    return { doc: emptyDiagram(), error: 'This file is not a diagram' }
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return { doc: emptyDiagram(), error: 'This file is not a diagram' }
  }
  const raw = value as Record<string, unknown>
  const nodes = parseNodes(raw.nodes)
  const edges = parseEdges(raw.edges)
  if (!nodes || !edges) return { doc: emptyDiagram(), error: 'This file is not a diagram' }
  const grid = num(raw.grid)
  return {
    doc: {
      snap: typeof raw.snap === 'boolean' ? raw.snap : true,
      grid: grid != null && grid > 0 ? grid : DIAGRAM_GRID,
      nodes,
      edges,
    },
    error: null,
  }
}

export function serializeDiagram(doc: DiagramDoc): string {
  return JSON.stringify({
    snap: doc.snap,
    grid: doc.grid,
    nodes: doc.nodes.map((n) => {
      const row: DiagramNode = { id: n.id, kind: n.kind, x: n.x, y: n.y, w: n.w, h: n.h, text: n.text }
      if (n.rotation) row.rotation = n.rotation
      return row
    }),
    edges: doc.edges.map((e) => {
      const row: DiagramEdge = { id: e.id, from: e.from, to: e.to }
      if (e.stroke === false) row.stroke = false
      if (e.color) row.color = e.color
      if (e.width != null && e.width !== 1.5) row.width = e.width
      if (e.dash && e.dash !== 'solid') row.dash = e.dash
      if (e.start && e.start !== 'none') row.start = e.start
      if (e.end && e.end !== 'arrow') row.end = e.end
      if (e.corner && e.corner !== 'sharp') row.corner = e.corner
      if (e.route && e.route !== 'orthogonal') row.route = e.route
      if (e.fromSide) row.fromSide = e.fromSide
      if (e.toSide) row.toSide = e.toSide
      if (e.bends?.length) row.bends = e.bends.map((p) => ({ x: p.x, y: p.y }))
      return row
    }),
  })
}

export type DiagramEdgeStyle = {
  stroke: boolean
  color: string
  width: number
  dash: DiagramDash
  start: DiagramArrow
  end: DiagramArrow
  corner: DiagramCorner
  route: DiagramRoute
}

export function edgeStyle(edge: DiagramEdge): DiagramEdgeStyle {
  return {
    stroke: edge.stroke !== false,
    color: edge.color ?? '',
    width: edge.width != null && edge.width > 0 ? edge.width : 1.5,
    dash: edge.dash ?? 'solid',
    start: edge.start ?? 'none',
    end: edge.end ?? 'arrow',
    corner: edge.corner ?? 'sharp',
    route: edge.route ?? 'orthogonal',
  }
}

export function nodeCenter(node: DiagramNode): DiagramPoint {
  return { x: node.x + node.w / 2, y: node.y + node.h / 2 }
}

export function rotatePoint(point: DiagramPoint, center: DiagramPoint, degrees: number): DiagramPoint {
  if (!degrees) return point
  const rad = (degrees * Math.PI) / 180
  const dx = point.x - center.x
  const dy = point.y - center.y
  const cos = Math.cos(rad)
  const sin = Math.sin(rad)
  return { x: center.x + dx * cos - dy * sin, y: center.y + dx * sin + dy * cos }
}

export function sideAnchor(node: DiagramNode, side: DiagramSide): DiagramPoint {
  const local =
    side === 'n'
      ? { x: node.x + node.w / 2, y: node.y }
      : side === 's'
        ? { x: node.x + node.w / 2, y: node.y + node.h }
        : side === 'w'
          ? { x: node.x, y: node.y + node.h / 2 }
          : { x: node.x + node.w, y: node.y + node.h / 2 }
  return rotatePoint(local, nodeCenter(node), node.rotation ?? 0)
}

export function nearestSide(node: DiagramNode, point: DiagramPoint): DiagramSide {
  let best: DiagramSide = 'e'
  let dist = Infinity
  for (const side of SIDES) {
    const anchor = sideAnchor(node, side)
    const d = (anchor.x - point.x) ** 2 + (anchor.y - point.y) ** 2
    if (d < dist) {
      dist = d
      best = side
    }
  }
  return best
}

export function resizeNode(
  node: DiagramNode,
  handle: DiagramHandle,
  dx: number,
  dy: number,
  grid: number,
  enabled: boolean,
): DiagramNode {
  const rad = (-(node.rotation ?? 0) * Math.PI) / 180
  const lx = dx * Math.cos(rad) - dy * Math.sin(rad)
  const ly = dx * Math.sin(rad) + dy * Math.cos(rad)
  const right = node.x + node.w
  const bottom = node.y + node.h
  let x = node.x
  let y = node.y
  let w = node.w
  let h = node.h
  if (handle.includes('w')) {
    x = snap(node.x + lx, grid, enabled)
    w = right - x
  } else if (handle.includes('e')) {
    w = snap(right + lx, grid, enabled) - x
  }
  if (handle.includes('n')) {
    y = snap(node.y + ly, grid, enabled)
    h = bottom - y
  } else if (handle.includes('s')) {
    h = snap(bottom + ly, grid, enabled) - y
  }
  if (w < MIN_W) {
    if (handle.includes('w')) x = right - MIN_W
    w = MIN_W
  }
  if (h < MIN_H) {
    if (handle.includes('n')) y = bottom - MIN_H
    h = MIN_H
  }
  return { ...node, x, y, w, h }
}

export function pointerAngle(node: DiagramNode, point: DiagramPoint): number {
  const center = nodeCenter(node)
  return (Math.atan2(point.y - center.y, point.x - center.x) * 180) / Math.PI
}

export function nextRotation(start: number, startAngle: number, angle: number, snapOn: boolean): number {
  let degrees = start + (angle - startAngle)
  degrees = ((degrees % 360) + 360) % 360
  if (snapOn) degrees = Math.round(degrees / 15) * 15 % 360
  return degrees === 360 ? 0 : degrees
}

function outgoing(point: DiagramPoint, side: DiagramSide, distance: number): DiagramPoint {
  if (side === 'n') return { x: point.x, y: point.y - distance }
  if (side === 's') return { x: point.x, y: point.y + distance }
  if (side === 'w') return { x: point.x - distance, y: point.y }
  return { x: point.x + distance, y: point.y }
}

function dedupe(points: DiagramPoint[]): DiagramPoint[] {
  const kept: DiagramPoint[] = []
  for (const point of points) {
    const prev = kept[kept.length - 1]
    if (prev && Math.abs(prev.x - point.x) < 0.5 && Math.abs(prev.y - point.y) < 0.5) continue
    const before = kept[kept.length - 2]
    if (
      prev &&
      before &&
      ((Math.abs(before.x - prev.x) < 0.5 && Math.abs(prev.x - point.x) < 0.5) ||
        (Math.abs(before.y - prev.y) < 0.5 && Math.abs(prev.y - point.y) < 0.5))
    ) {
      kept[kept.length - 1] = point
      continue
    }
    kept.push(point)
  }
  return kept
}

export function edgeRoute(from: DiagramNode, to: DiagramNode, edge: DiagramEdge): DiagramPoint[] {
  const style = edgeStyle(edge)
  const fromSide = edge.fromSide ?? nearestSide(from, nodeCenter(to))
  const toSide = edge.toSide ?? nearestSide(to, nodeCenter(from))
  const start = sideAnchor(from, fromSide)
  const end = sideAnchor(to, toSide)
  if (style.route === 'straight') return [start, end]
  if (edge.bends?.length) return dedupe([start, ...edge.bends, end])
  const stub = 24
  const leave = outgoing(start, fromSide, stub)
  const enter = outgoing(end, toSide, stub)
  const horizontal = (side: DiagramSide) => side === 'e' || side === 'w'
  let mid: DiagramPoint[]
  if (horizontal(fromSide) && horizontal(toSide)) {
    const x = (leave.x + enter.x) / 2
    mid = [leave, { x, y: leave.y }, { x, y: enter.y }, enter]
  } else if (!horizontal(fromSide) && !horizontal(toSide)) {
    const y = (leave.y + enter.y) / 2
    mid = [leave, { x: leave.x, y }, { x: enter.x, y }, enter]
  } else if (horizontal(fromSide)) {
    mid = [leave, { x: enter.x, y: leave.y }, enter]
  } else {
    mid = [leave, { x: leave.x, y: enter.y }, enter]
  }
  return dedupe([start, ...mid, end])
}

/** Move one segment of an orthogonal route and keep both anchors fixed. */
export function slideSegment(points: DiagramPoint[], index: number, value: number): DiagramPoint[] {
  if (index < 0 || index >= points.length - 1 || points.length < 2) return points
  const start = points[index]
  const end = points[index + 1]
  const horizontal = Math.abs(end.x - start.x) >= Math.abs(end.y - start.y)
  const shift = (point: DiagramPoint): DiagramPoint => (horizontal ? { x: point.x, y: value } : { x: value, y: point.y })
  const next = points.map((point) => ({ ...point }))
  next[index] = shift(next[index])
  next[index + 1] = shift(next[index + 1])
  if (index === 0) {
    next[0] = { ...points[0] }
    next.splice(1, 0, shift(points[0]))
  }
  const last = next.length - 1
  const endIndex = index === 0 ? index + 2 : index + 1
  if (endIndex === last) {
    next[last] = { ...points[points.length - 1] }
    next.splice(last, 0, shift(points[points.length - 1]))
  }
  return dedupe(next)
}

export function routePath(points: DiagramPoint[], rounded: boolean): string {
  if (points.length < 2) return ''
  if (!rounded || points.length < 3) {
    return points.map((point, index) => `${index ? 'L' : 'M'}${point.x} ${point.y}`).join(' ')
  }
  const radius = 8
  let path = `M${points[0].x} ${points[0].y}`
  for (let i = 1; i < points.length - 1; i++) {
    const prev = points[i - 1]
    const current = points[i]
    const next = points[i + 1]
    const inX = current.x - prev.x
    const inY = current.y - prev.y
    const outX = next.x - current.x
    const outY = next.y - current.y
    const inLen = Math.hypot(inX, inY) || 1
    const outLen = Math.hypot(outX, outY) || 1
    const rad = Math.min(radius, inLen / 2, outLen / 2)
    const enter = { x: current.x - (inX / inLen) * rad, y: current.y - (inY / inLen) * rad }
    const leave = { x: current.x + (outX / outLen) * rad, y: current.y + (outY / outLen) * rad }
    path += ` L${enter.x} ${enter.y} Q${current.x} ${current.y} ${leave.x} ${leave.y}`
  }
  const last = points[points.length - 1]
  return `${path} L${last.x} ${last.y}`
}
