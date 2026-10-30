import type { DesignBooleanOp, DesignDoc, DesignNode, DesignPenPoint } from './types'
import { booleanEligible, booleanPaintNode, booleanResult, contoursToVector, isBooleanNode } from './boolean'
import { createNode, findDesignNode, fitVectorNode, nodeFromPen, spinToParent, updateDesignNode, wrapDesignNodes } from './model'
import { setNodeSolid } from './paint'

function regularPoints(count: number, turn = -Math.PI / 2): DesignPenPoint[] {
  const n = Math.max(3, Math.round(count))
  const zero = { x: 0, y: 0 }
  const points: DesignPenPoint[] = []
  for (let i = 0; i < n; i++) {
    const angle = turn + (i * Math.PI * 2) / n
    points.push({ x: Math.cos(angle), y: Math.sin(angle), incoming: zero, outgoing: zero })
  }
  return points
}

function fillBox(points: DesignPenPoint[], w: number, h: number): DesignPenPoint[] {
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  for (const point of points) {
    minX = Math.min(minX, point.x)
    minY = Math.min(minY, point.y)
    maxX = Math.max(maxX, point.x)
    maxY = Math.max(maxY, point.y)
  }
  const spanX = maxX - minX || 1
  const spanY = maxY - minY || 1
  return points.map((point) => ({
    ...point,
    x: ((point.x - minX) / spanX) * w,
    y: ((point.y - minY) / spanY) * h,
  }))
}

export function createPolygonNode(id: string, x: number, y: number, w = 100, sides = 6, h = w): DesignNode {
  const width = Math.max(1, w)
  const height = Math.max(1, h)
  const node = nodeFromPen(id, fillBox(regularPoints(sides), width, height), true)
  if (!node) return { ...createNode('rect', id, x, y), name: 'Polygon', w: width, h: height, pointCount: sides }
  return { ...node, name: 'Polygon', pointCount: sides, x, y, w: width, h: height }
}

const SHAPE_KINDS = ['rect', 'ellipse', 'line', 'polygon', 'star'] as const
export type DesignReshapeKind = (typeof SHAPE_KINDS)[number]

export function shapeKindOf(node: DesignNode): DesignReshapeKind | null {
  if (node.kind === 'rect' || node.kind === 'ellipse' || node.kind === 'line') return node.kind
  if (node.kind === 'vector' && node.innerRadius != null) return 'star'
  if (node.kind === 'vector' && node.pointCount != null) return 'polygon'
  return null
}

/** Keep box and paints; switch rect / ellipse / line / polygon / star. */
export function reshapeDesignNode(node: DesignNode, kind: DesignReshapeKind): DesignNode {
  if (shapeKindOf(node) === kind) return node
  const shared: Partial<DesignNode> = {
    id: node.id,
    x: node.x,
    y: node.y,
    w: kind === 'line' ? Math.max(node.w, 8) : node.w,
    h: kind === 'line' ? Math.max(node.strokeWidth ?? 2, 1) : node.h,
    fill: node.fill,
    fills: node.fills,
    stroke: node.stroke,
    strokes: node.strokes,
    strokeWidth: node.strokeWidth,
    opacity: node.opacity,
    rotation: node.rotation,
    visible: node.visible,
    locked: node.locked,
    constraintH: node.constraintH,
    constraintV: node.constraintV,
    name: node.name,
  }
  if (kind === 'polygon') {
    const made = createPolygonNode(node.id, node.x, node.y, Math.max(8, node.w), node.pointCount && node.pointCount >= 3 ? node.pointCount : 6, Math.max(8, node.h))
    return { ...made, ...shared, kind: 'vector', name: node.name || 'Polygon' }
  }
  if (kind === 'star') {
    const made = createStarNode(node.id, node.x, node.y, Math.max(8, node.w), node.pointCount && node.pointCount >= 3 ? node.pointCount : 5, node.innerRadius ?? 0.38, Math.max(8, node.h))
    return { ...made, ...shared, kind: 'vector', name: node.name || 'Star' }
  }
  const next: DesignNode = {
    ...node,
    kind,
    name: node.name || (kind === 'ellipse' ? 'Ellipse' : kind === 'line' ? 'Line' : 'Rectangle'),
    w: shared.w ?? node.w,
    h: shared.h ?? node.h,
  }
  delete next.vector
  delete next.pointCount
  delete next.innerRadius
  if (kind === 'line') {
    next.stroke = next.stroke && next.stroke !== 'none' ? next.stroke : '#1c1c1c'
    next.strokeWidth = next.strokeWidth ?? 2
  }
  return next
}

/** Rebuild a polygon or star after changing sides / inner radius. */
export function retuneDesignShape(node: DesignNode, patch: { pointCount?: number; innerRadius?: number }): DesignNode {
  const kind = shapeKindOf(node)
  if (kind !== 'polygon' && kind !== 'star') return node
  const count = Math.max(3, Math.round(patch.pointCount ?? node.pointCount ?? (kind === 'star' ? 5 : 6)))
  const inner = Math.max(0.05, Math.min(0.95, patch.innerRadius ?? node.innerRadius ?? 0.38))
  const made = kind === 'star'
    ? createStarNode(node.id, node.x, node.y, Math.max(8, node.w), count, inner, Math.max(8, node.h))
    : createPolygonNode(node.id, node.x, node.y, Math.max(8, node.w), count, Math.max(8, node.h))
  return {
    ...made,
    id: node.id,
    x: node.x,
    y: node.y,
    w: node.w,
    h: node.h,
    name: node.name,
    fill: node.fill,
    fills: node.fills,
    stroke: node.stroke,
    strokes: node.strokes,
    strokeWidth: node.strokeWidth,
    opacity: node.opacity,
    rotation: node.rotation,
    visible: node.visible,
    locked: node.locked,
    constraintH: node.constraintH,
    constraintV: node.constraintV,
  }
}

export function createStarNode(id: string, x: number, y: number, w = 100, points = 5, inner = 0.38, h = w): DesignNode {
  const n = Math.max(3, Math.round(points))
  const width = Math.max(1, w)
  const height = Math.max(1, h)
  const ratio = Math.max(0.05, Math.min(0.95, inner))
  const zero = { x: 0, y: 0 }
  const verts: DesignPenPoint[] = []
  for (let i = 0; i < n * 2; i++) {
    const radius = i % 2 === 0 ? 1 : ratio
    const angle = -Math.PI / 2 + (i * Math.PI) / n
    verts.push({ x: Math.cos(angle) * radius, y: Math.sin(angle) * radius, incoming: zero, outgoing: zero })
  }
  const node = nodeFromPen(id, fillBox(verts, width, height), true)
  if (!node) return { ...createNode('rect', id, x, y), name: 'Star', w: width, h: height, pointCount: n, innerRadius: inner }
  return { ...node, name: 'Star', pointCount: n, innerRadius: inner, x, y, w: width, h: height }
}

function opTitle(op: string): string {
  return op[0].toUpperCase() + op.slice(1)
}

export function booleanDesignNodes(doc: DesignDoc, ids: string[], op: DesignBooleanOp, mint: () => string): DesignDoc | null {
  if (ids.length < 2) return null
  const id = mint()
  const wrapped = wrapDesignNodes(doc, ids, 'group', id)
  if (!wrapped) return null
  return updateDesignNode(wrapped, id, (node) => booleanPaintNode({ ...node, name: opTitle(op), booleanOp: op }))
}

/** A boolean back to a plain group; its shapes stay as they are. */
export function booleanToGroup(node: DesignNode): DesignNode {
  if (!node.booleanOp) return node
  const next = { ...node }
  delete next.booleanOp
  if (next.name === opTitle(node.booleanOp)) next.name = 'Group'
  return next
}

/** A plain group takes a boolean operation in place (Penpot's group-to-bool). */
export function groupToBoolean(node: DesignNode, op: DesignBooleanOp): DesignNode {
  if (node.kind !== 'group' || !node.children?.length) return node
  return booleanPaintNode({ ...node, name: node.name === 'Group' || !node.name ? opTitle(op) : node.name, booleanOp: op })
}

export function retargetBooleanOp(node: DesignNode, op: DesignBooleanOp): DesignNode {
  if (!node.booleanOp) return node
  const name = node.name === opTitle(node.booleanOp) ? opTitle(op) : node.name
  if (node.booleanOp === op && name === node.name) return node
  return { ...node, booleanOp: op, name }
}

/**
 * Union, subtract, intersect, exclude, or flatten. One boolean changes op instead of nesting,
 * and picking the operation it already has turns it back into a group. One plain group takes
 * the operation in place.
 */
export function applyDesignBoolean(doc: DesignDoc, ids: string[], op: DesignBooleanOp | 'flatten', mint: () => string): DesignDoc | null {
  if (!ids.length) return null
  if (!booleanEligible(doc, ids).ok) return null
  if (ids.length === 1) {
    const node = findDesignNode(doc, ids[0])
    if (!node) return null
    if (!isBooleanNode(node)) {
      if (op === 'flatten') return null
      return updateDesignNode(doc, ids[0], (item) => groupToBoolean(item, op))
    }
    if (op === 'flatten') return updateDesignNode(doc, ids[0], (item) => flattenBooleanNode(doc, item))
    if (node.booleanOp === op) return updateDesignNode(doc, ids[0], booleanToGroup)
    return updateDesignNode(doc, ids[0], (item) => retargetBooleanOp(item, op))
  }
  if (op === 'flatten') {
    const id = mint()
    const union = booleanDesignNodes(doc, ids, 'union', () => id)
    if (!union) return null
    return updateDesignNode(union, id, (item) => flattenBooleanNode(union, item))
  }
  return booleanDesignNodes(doc, ids, op, mint)
}

export function outlineStrokeNode(node: DesignNode): DesignNode {
  if (node.kind !== 'vector' || !node.vector) return node
  const fill = node.stroke && node.stroke !== 'none' ? node.stroke : '#1c1c1c'
  const next = setNodeSolid({ ...node, name: node.name || 'Outline' }, 'fill', fill)
  return setNodeSolid(next, 'stroke', 'none')
}

export function flattenBooleanNode(doc: DesignDoc, node: DesignNode): DesignNode {
  if (!isBooleanNode(node)) return node
  const painted = booleanPaintNode(node)
  const geometry = booleanResult(doc, node)
  const vector = contoursToVector(geometry.contours, geometry.rule)
  const base: DesignNode = {
    id: node.id,
    kind: 'vector',
    name: node.name || 'Flatten',
    x: node.x,
    y: node.y,
    w: Math.max(1, node.w),
    h: Math.max(1, node.h),
    rotation: node.rotation,
    flipX: node.flipX,
    flipY: node.flipY,
    fill: painted.fill,
    fills: painted.fills,
    stroke: painted.stroke,
    strokes: painted.strokes,
    strokeWidth: painted.strokeWidth,
    opacity: painted.opacity,
    effects: painted.effects,
    blend: painted.blend,
    vector,
  }
  if (!vector.vertices.length) return { ...base, vector: { vertices: [], segments: [], regions: [] } }
  const fitted = fitVectorNode({ ...base, x: 0, y: 0 })
  const probe: DesignNode = { ...node, x: 0, y: 0, w: fitted.w, h: fitted.h }
  const origin = spinToParent(probe, 0, 0)
  const parent = spinToParent(node, fitted.x, fitted.y)
  return { ...fitted, x: parent.x - origin.x, y: parent.y - origin.y, rotation: node.rotation, flipX: node.flipX, flipY: node.flipY }
}
