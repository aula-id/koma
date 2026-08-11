/**
 * Three-column React Flow layout for the import graph.
 *
 * Fixed columns: dependents (left) / focus (center) / dependencies (right).
 * Stable sort: relevant-degree desc then path asc. Generous card + column gaps.
 * Defensively excludes depth>1 and edges not touching focus.
 *
 * Each edge carries sourceHandle/targetHandle metadata for React Flow routing.
 * Handle IDs must match the four handles defined in ImportGraphFlow's FileCard:
 *   left-target, left-source, right-target, right-source.
 *
 * Returns React-Flow-ready positions and valid edge pairs — no layout library dependency.
 */

import type { ImportGraphNode, ImportGraphEdge } from '../store/koma'

// ─── Layout constants ──────────────────────────────────────────────────────
const CARD_W = 220
const CARD_H = 72
const CARD_GAP_Y = 14
const COLUMN_GAP_X = 260
const PADDING_X = 60
const PADDING_Y = 60
const FOCUS_CARD_H = 88

export type FlowPosition = { id: string; x: number; y: number; side: 'left' | 'center' | 'right' }
export type FlowEdgePair = {
  id: string
  source: string
  target: string
  sourceHandle: string
  targetHandle: string
}

export type FlowLayoutResult = {
  positions: FlowPosition[]
  edges: FlowEdgePair[]
  width: number
  height: number
}

function fileName(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/')
  return parts[parts.length - 1] || path
}

function parentPath(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/')
  if (parts.length <= 2) return ''
  return parts.slice(0, -1).join('/')
}

/**
 * Pure layout function — no hooks, no side effects.
 * Classifies nodes into three columns, sorts stably, and returns positions.
 * Edges are filtered to only those whose both endpoints are in the result set
 * and that touch the focus node.
 *
 * Edge handles (must match FileCard's four invisible handles):
 *   dependent -> focus:   source 'right-source' -> target 'left-target'
 *   focus -> dependency:  source 'right-source' -> target 'left-target'
 *   reciprocal (focus -> dependent):   source 'left-source' -> target 'right-target'
 *   reciprocal (dependency -> focus):  source 'left-source' -> target 'right-target'
 */
export function computeFlowLayout(
  nodes: ImportGraphNode[],
  edges: ImportGraphEdge[],
  focus: string | null,
): FlowLayoutResult {
  if (nodes.length === 0 || !focus) {
    return { positions: [], edges: [], width: PADDING_X * 2, height: PADDING_Y * 2 }
  }

  // ── Classify: focus, dependents (depth1 incoming), dependencies (depth1 outgoing)
  const focusNode = nodes.find((n) => n.path === focus && n.role === 'Focus')
  if (!focusNode) {
    return { positions: [], edges: [], width: PADDING_X * 2, height: PADDING_Y * 2 }
  }

  const dependents: ImportGraphNode[] = []
  const dependencies: ImportGraphNode[] = []

  for (const n of nodes) {
    if (n.path === focus) continue
    // Strictly exclude depth>1
    if (n.depthFromFocus !== null && n.depthFromFocus > 1) continue
    if (n.role === 'Dependent') dependents.push(n)
    else if (n.role === 'Dependency') dependencies.push(n)
    else if (n.role === 'Overview') {
      // Overview nodes under depth1 fallback
      if (n.depthFromFocus === 1) {
        // Classify by edge direction if possible
        const hasIncoming = edges.some((e) => e.to === focus && e.from === n.path)
        const hasOutgoing = edges.some((e) => e.from === focus && e.to === n.path)
        if (hasIncoming && !hasOutgoing) dependents.push(n)
        else if (hasOutgoing && !hasIncoming) dependencies.push(n)
        else dependents.push(n)
      }
    }
  }

  // ── Stable sort within each column: relevant-degree desc, then path asc ──
  // Dependents: files that import focus → sort by outDegree (how many they import, relevant to focus relationship)
  dependents.sort((a, b) => {
    const dA = a.outDegree
    const dB = b.outDegree
    if (dB !== dA) return dB - dA
    return a.path.localeCompare(b.path)
  })

  // Dependencies: files focus imports → sort by inDegree (how many import them, relevant to focus relationship)
  dependencies.sort((a, b) => {
    const dA = a.inDegree
    const dB = b.inDegree
    if (dB !== dA) return dB - dA
    return a.path.localeCompare(b.path)
  })

  // ── Vertically center both columns independently ──
  const depCount = dependents.length
  const depRightCount = dependencies.length
  const maxColCount = Math.max(depCount, depRightCount, 1)

  // Column 0: dependents (left) — vertically centered in max column height
  const leftTotalH = depCount * CARD_H + Math.max(0, depCount - 1) * CARD_GAP_Y
  const leftMaxH = maxColCount * CARD_H + Math.max(0, maxColCount - 1) * CARD_GAP_Y
  const leftStartY = PADDING_Y + (leftMaxH - leftTotalH) / 2

  // Column 2: dependencies (right) — vertically centered independently
  const rightTotalH = depRightCount * CARD_H + Math.max(0, depRightCount - 1) * CARD_GAP_Y
  const rightMaxH = maxColCount * CARD_H + Math.max(0, maxColCount - 1) * CARD_GAP_Y
  const rightStartY = PADDING_Y + (rightMaxH - rightTotalH) / 2

  // Focus: vertically centered relative to the max column height
  const focusTotalH = FOCUS_CARD_H
  const focusStartY = PADDING_Y + (leftMaxH - focusTotalH) / 2

  const positions: FlowPosition[] = []
  const depRightX = PADDING_X + (CARD_W + COLUMN_GAP_X) * 2
  const focusX = PADDING_X + CARD_W + COLUMN_GAP_X

  for (let i = 0; i < depCount; i++) {
    positions.push({
      id: dependents[i].path,
      x: PADDING_X,
      y: leftStartY + i * (CARD_H + CARD_GAP_Y),
      side: 'left',
    })
  }

  // Column 1: focus (center)
  positions.push({
    id: focusNode.path,
    x: focusX,
    y: focusStartY,
    side: 'center',
  })

  for (let i = 0; i < depRightCount; i++) {
    positions.push({
      id: dependencies[i].path,
      x: depRightX,
      y: rightStartY + i * (CARD_H + CARD_GAP_Y),
      side: 'right',
    })
  }

  // ── Valid edge pairs: only edges touching focus with both endpoints present ──
  const nodeSet = new Set(positions.map((p) => p.id))
  const nodeSide = new Map(positions.map((p) => [p.id, p.side]))
  const edgesOut: FlowEdgePair[] = []
  const seen = new Set<string>()

  for (const e of edges) {
    // Only include edges that touch the focus node
    if (e.from !== focus && e.to !== focus) continue
    // Both endpoints must be in the positioned set
    if (!nodeSet.has(e.from) || !nodeSet.has(e.to)) continue
    const key = `${e.from}→${e.to}`
    if (seen.has(key)) continue
    seen.add(key)

    const fromSide = nodeSide.get(e.from) ?? 'center'
    const toSide = nodeSide.get(e.to) ?? 'center'

    // Handle routing (must match FileCard handle IDs exactly):
    // dependent -> focus:   source 'right-source' -> target 'left-target'
    // focus -> dependency:  source 'right-source' -> target 'left-target'
    // reciprocal (focus -> dependent):      source 'left-source' -> target 'right-target'
    // reciprocal (dependency -> focus):     source 'left-source' -> target 'right-target'
    let sourceHandle: string
    let targetHandle: string

    if (fromSide === 'left' && toSide === 'center') {
      // dependent -> focus
      sourceHandle = 'right-source'
      targetHandle = 'left-target'
    } else if (fromSide === 'center' && toSide === 'right') {
      // focus -> dependency
      sourceHandle = 'right-source'
      targetHandle = 'left-target'
    } else if (fromSide === 'center' && toSide === 'left') {
      // reciprocal: focus -> dependent (reverse direction)
      sourceHandle = 'left-source'
      targetHandle = 'right-target'
    } else if (fromSide === 'right' && toSide === 'center') {
      // reciprocal: dependency -> focus (reverse direction)
      sourceHandle = 'left-source'
      targetHandle = 'right-target'
    } else {
      sourceHandle = 'right-source'
      targetHandle = 'left-target'
    }

    edgesOut.push({ id: key, source: e.from, target: e.to, sourceHandle, targetHandle })
  }

  // ── Compute bounding box ───────────────────────────────────────────────
  let maxX = 0
  let maxY = 0
  for (const p of positions) {
    maxX = Math.max(maxX, p.x + CARD_W)
    maxY = Math.max(maxY, p.y + CARD_H)
  }

  return {
    positions,
    edges: edgesOut,
    width: maxX + PADDING_X,
    height: maxY + PADDING_Y,
  }
}

// ─── Export helpers for the component ──────────────────────────────────────

export { fileName, parentPath, CARD_W, CARD_H, FOCUS_CARD_H }
