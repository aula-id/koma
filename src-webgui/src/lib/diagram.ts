// Diagram documents stored as `<workspace>/.koma/<name>.diag`.
// The canvas and the sidebar both speak this shape. Snap is per file.

export const DIAGRAM_GRID = 16
export const SHAPE_MIME = 'application/x-koma-shape'

export type DiagramKind = 'rect' | 'ellipse' | 'diamond' | 'text'

export type DiagramNode = {
  id: string
  kind: DiagramKind
  x: number
  y: number
  w: number
  h: number
  text: string
}

export type DiagramEdge = {
  id: string
  from: string
  to: string
}

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
    nodes.push({ id: row.id, kind: row.kind, x, y, w, h, text: row.text })
  }
  return nodes
}

function parseEdges(value: unknown): DiagramEdge[] | null {
  if (!Array.isArray(value)) return null
  const edges: DiagramEdge[] = []
  for (const item of value) {
    if (!item || typeof item !== 'object') return null
    const row = item as Record<string, unknown>
    if (typeof row.id !== 'string' || !row.id || typeof row.from !== 'string' || typeof row.to !== 'string') return null
    edges.push({ id: row.id, from: row.from, to: row.to })
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
    nodes: doc.nodes.map((n) => ({ id: n.id, kind: n.kind, x: n.x, y: n.y, w: n.w, h: n.h, text: n.text })),
    edges: doc.edges.map((e) => ({ id: e.id, from: e.from, to: e.to })),
  })
}
