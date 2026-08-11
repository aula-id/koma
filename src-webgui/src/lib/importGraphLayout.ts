import type { ImportGraphNode, ImportGraphEdge } from '../store/koma'

// ─── Layout constants ────────────────────────────────────────────────────
const NODE_W = 180
const NODE_H = 32
const LAYER_GAP = 80
const NODE_GAP = 12
const PADDING = 40
const GRID_COLS_DEFAULT = 4

export type LayoutNode = {
  id: string // canonical path
  x: number
  y: number
  width: number
  height: number
  label: string // workspace-relative path for display
  role: string
  depth: number | null
  language: string
  outDegree: number
  inDegree: number
}

export type LayoutEdge = {
  from: string
  to: string
  points: { x: number; y: number }[]
}

export type ImportGraphLayout = {
  nodes: LayoutNode[]
  edges: LayoutEdge[]
  width: number
  height: number
}

// Extract a short display label from a full path — the last 1–2 segments.
function shortLabel(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/')
  if (parts.length <= 2) return path
  return parts.slice(-2).join('/')
}

// ─── Layered DAG layout ─────────────────────────────────────────────────
//
// 1. Layer assignment: nodes grouped by `depthFromFocus`. Overview nodes
//    (null depth) go into a single flat grid layer.
// 2. Node positioning: within each layer, sorted by inDegree desc then
//    lex path. Fixed row height, computed column width.
// 3. Edge routing: right-angle S-routes from source bottom to target top
//    via a midpoint waypoint.
export function computeImportGraphLayout(
  nodes: ImportGraphNode[],
  edges: ImportGraphEdge[],
  _focus: string | null,
): ImportGraphLayout {
  if (nodes.length === 0) {
    return { nodes: [], edges: [], width: PADDING * 2, height: PADDING * 2 }
  }

  // Build lookup maps
  const nodeMap = new Map<string, ImportGraphNode>()
  for (const n of nodes) nodeMap.set(n.path, n)

  // ── Step 1: Layer assignment ─────────────────────────────────────────
  const hasFocus = nodes.some((n) => n.role === 'Focus')
  const layers: Map<number, ImportGraphNode[]> = new Map()

  if (hasFocus) {
    // Group by depthFromFocus; null-depth nodes get a high layer (below everything)
    for (const n of nodes) {
      const layer = n.depthFromFocus ?? 999
      const arr = layers.get(layer) ?? []
      arr.push(n)
      layers.set(layer, arr)
    }
  } else {
    // Overview mode: everything in layer 0
    layers.set(0, [...nodes])
  }

  const sortedLayers = [...layers.entries()].sort((a, b) => a[0] - b[0])

  // ── Step 2: Position nodes ──────────────────────────────────────────
  const layoutNodes: LayoutNode[] = []
  const posMap = new Map<string, { x: number; y: number }>()
  let yOffset = PADDING

  if (!hasFocus && nodes.length > 0) {
    // Overview grid layout
    const cols = Math.min(GRID_COLS_DEFAULT, nodes.length)
    const sorted = [...nodes].sort((a, b) => a.path.localeCompare(b.path))
    for (let i = 0; i < sorted.length; i++) {
      const col = i % cols
      const row = Math.floor(i / cols)
      const x = PADDING + col * (NODE_W + NODE_GAP)
      const y = PADDING + row * (NODE_H + NODE_GAP)
      const n = sorted[i]
      layoutNodes.push({
        id: n.path,
        x,
        y,
        width: NODE_W,
        height: NODE_H,
        label: shortLabel(n.path),
        role: n.role,
        depth: n.depthFromFocus,
        language: n.language,
        outDegree: n.outDegree,
        inDegree: n.inDegree,
      })
      posMap.set(n.path, { x: x + NODE_W / 2, y: y + NODE_H / 2 })
    }
    const totalRows = Math.ceil(sorted.length / cols)
    const graphW = PADDING * 2 + cols * (NODE_W + NODE_GAP) - NODE_GAP
    const graphH = PADDING * 2 + totalRows * (NODE_H + NODE_GAP) - NODE_GAP
    return { nodes: layoutNodes, edges: [], width: graphW, height: graphH }
  }

  // Focused layered layout
  for (const [, layerNodes] of sortedLayers) {
    // Sort within layer: inDegree desc, then lex path
    layerNodes.sort((a, b) => {
      if (b.inDegree !== a.inDegree) return b.inDegree - a.inDegree
      return a.path.localeCompare(b.path)
    })

    for (let i = 0; i < layerNodes.length; i++) {
      const n = layerNodes[i]
      const x = PADDING + i * (NODE_W + NODE_GAP)
      const y = yOffset
      layoutNodes.push({
        id: n.path,
        x,
        y,
        width: NODE_W,
        height: NODE_H,
        label: shortLabel(n.path),
        role: n.role,
        depth: n.depthFromFocus,
        language: n.language,
        outDegree: n.outDegree,
        inDegree: n.inDegree,
      })
      posMap.set(n.path, { x: x + NODE_W / 2, y: y + NODE_H / 2 })
    }

    yOffset += NODE_H + LAYER_GAP
  }

  // ── Step 3: Edge routing ────────────────────────────────────────────
  const layoutEdges: LayoutEdge[] = []
  const maxW =
    layoutNodes.length > 0
      ? Math.max(...layoutNodes.map((n) => n.x + n.width))
      : PADDING * 2

  for (const e of edges) {
    const fromPos = posMap.get(e.from)
    const toPos = posMap.get(e.to)
    if (!fromPos || !toPos) continue

    // Right-angle S-route: source-bottom → midpoint → target-top
    const midY = (fromPos.y + NODE_H / 2 + toPos.y - NODE_H / 2) / 2
    layoutEdges.push({
      from: e.from,
      to: e.to,
      points: [
        { x: fromPos.x, y: fromPos.y + NODE_H / 2 },
        { x: fromPos.x, y: midY },
        { x: toPos.x, y: midY },
        { x: toPos.x, y: toPos.y - NODE_H / 2 },
      ],
    })
  }

  const graphW = maxW + PADDING
  const graphH = yOffset > PADDING ? yOffset - LAYER_GAP + PADDING : PADDING * 2

  return { nodes: layoutNodes, edges: layoutEdges, width: graphW, height: graphH }
}
