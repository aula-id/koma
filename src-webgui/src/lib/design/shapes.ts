import type { DesignBooleanOp, DesignDoc, DesignNode, DesignPenPoint } from './types'
import { createNode, nodeFromPen, updateDesignNode, wrapDesignNodes } from './model'
import { setNodeSolid } from './paint'

function regularPoints(cx: number, cy: number, radius: number, count: number, turn = -Math.PI / 2): DesignPenPoint[] {
  const n = Math.max(3, Math.round(count))
  const zero = { x: 0, y: 0 }
  const points: DesignPenPoint[] = []
  for (let i = 0; i < n; i++) {
    const angle = turn + (i * Math.PI * 2) / n
    points.push({ x: cx + Math.cos(angle) * radius, y: cy + Math.sin(angle) * radius, incoming: zero, outgoing: zero })
  }
  return points
}

export function createPolygonNode(id: string, x: number, y: number, size = 100, sides = 3): DesignNode {
  const radius = size / 2
  const node = nodeFromPen(id, regularPoints(x + radius, y + radius, radius, sides), true)
  if (!node) return { ...createNode('rect', id, x, y), name: 'Polygon', pointCount: sides }
  return { ...node, name: 'Polygon', pointCount: sides }
}

export function createStarNode(id: string, x: number, y: number, size = 100, points = 5, inner = 0.38): DesignNode {
  const n = Math.max(3, Math.round(points))
  const outer = size / 2
  const innerR = outer * Math.max(0.05, Math.min(0.95, inner))
  const zero = { x: 0, y: 0 }
  const verts: DesignPenPoint[] = []
  for (let i = 0; i < n * 2; i++) {
    const radius = i % 2 === 0 ? outer : innerR
    const angle = -Math.PI / 2 + (i * Math.PI) / n
    verts.push({ x: x + outer + Math.cos(angle) * radius, y: y + outer + Math.sin(angle) * radius, incoming: zero, outgoing: zero })
  }
  const node = nodeFromPen(id, verts, true)
  if (!node) return { ...createNode('rect', id, x, y), name: 'Star', pointCount: n, innerRadius: inner }
  return { ...node, name: 'Star', pointCount: n, innerRadius: inner }
}

export function booleanDesignNodes(doc: DesignDoc, ids: string[], op: DesignBooleanOp, mint: () => string): DesignDoc | null {
  if (ids.length < 2) return null
  const id = mint()
  const wrapped = wrapDesignNodes(doc, ids, 'group', id)
  if (!wrapped) return null
  return updateDesignNode(wrapped, id, (node) => ({ ...node, name: op[0].toUpperCase() + op.slice(1), booleanOp: op }))
}

export function outlineStrokeNode(node: DesignNode): DesignNode {
  if (node.kind !== 'vector' || !node.vector) return node
  const fill = node.stroke && node.stroke !== 'none' ? node.stroke : '#1c1c1c'
  const next = setNodeSolid({ ...node, name: node.name || 'Outline' }, 'fill', fill)
  return setNodeSolid(next, 'stroke', 'none')
}

export function flattenBooleanNode(node: DesignNode): DesignNode {
  if (!node.booleanOp || !node.children?.length) return node
  const first = node.children.find((child) => child.kind === 'vector' && child.vector)
  if (!first?.vector) return node
  const vertices = first.vector.vertices.map((point) => ({ x: point.x + first.x, y: point.y + first.y }))
  const segments = first.vector.segments.map((segment) => ({ ...segment, tangentStart: { ...segment.tangentStart }, tangentEnd: { ...segment.tangentEnd } }))
  for (const child of node.children) {
    if (child.id === first.id || child.kind !== 'vector' || !child.vector) continue
    const offset = vertices.length
    for (const point of child.vector.vertices) vertices.push({ x: point.x + child.x, y: point.y + child.y })
    for (const segment of child.vector.segments) {
      segments.push({
        start: segment.start + offset,
        end: segment.end + offset,
        tangentStart: { ...segment.tangentStart },
        tangentEnd: { ...segment.tangentEnd },
      })
    }
  }
  return {
    id: node.id,
    kind: 'vector',
    name: node.name || 'Flatten',
    x: node.x,
    y: node.y,
    w: node.w,
    h: node.h,
    fill: first.fill ?? '#d9d9d9',
    stroke: first.stroke ?? 'none',
    vector: { vertices, segments, regions: first.vector.regions.map((region) => ({ ...region, loops: region.loops.map((loop) => loop.slice()) })) },
  }
}
