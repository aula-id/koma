import { emptyDesign } from './model'
import type { DesignDoc, DesignNode } from './types'

export type DesignImportResult = { doc: DesignDoc; error: string | null }

function mint(prefix: string, n: { i: number }): string {
  n.i += 1
  return `${prefix}${n.i}`
}

/** Best-effort Open-Pencil / Figma-ish graph → DesignDoc. Unknown nodes become named frames. */
export function graphToDesign(graph: unknown): DesignImportResult {
  const doc = emptyDesign()
  doc.version = 2
  const counter = { i: 0 }
  const nodes = extractNodes(graph)
  if (!nodes.length) return { doc, error: 'No pages or nodes to import' }
  doc.screens = nodes.map((node) => mapUnknownNode(node, counter))
  return { doc, error: null }
}

function extractNodes(graph: unknown): unknown[] {
  if (!graph || typeof graph !== 'object') return []
  const row = graph as Record<string, unknown>
  if (Array.isArray(row.pages)) return row.pages
  if (Array.isArray(row.screens)) return row.screens
  if (Array.isArray(row.children)) return row.children
  if (row.document && typeof row.document === 'object') return extractNodes(row.document)
  return [graph]
}

function mapUnknownNode(value: unknown, counter: { i: number }): DesignNode {
  const row = value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
  const type = typeof row.type === 'string' ? row.type.toUpperCase() : typeof row.kind === 'string' ? row.kind : 'FRAME'
  const id = typeof row.id === 'string' && row.id ? row.id : mint('imp', counter)
  const name = typeof row.name === 'string' && row.name ? row.name : type
  const x = num(row.x) ?? num(row.left) ?? 0
  const y = num(row.y) ?? num(row.top) ?? 0
  const w = Math.max(1, num(row.w) ?? num(row.width) ?? 100)
  const h = Math.max(1, num(row.h) ?? num(row.height) ?? 100)
  const children = Array.isArray(row.children) ? row.children.map((child) => mapUnknownNode(child, counter)) : undefined
  if (type === 'TEXT' || type === 'text') {
    return { id, kind: 'text', name, x, y, w, h, text: typeof row.characters === 'string' ? row.characters : typeof row.text === 'string' ? row.text : 'Text' }
  }
  if (type === 'ELLIPSE' || type === 'ellipse') return { id, kind: 'ellipse', name, x, y, w, h, fill: '#d9d9d9', stroke: 'none' }
  if (type === 'RECTANGLE' || type === 'rect') return { id, kind: 'rect', name, x, y, w, h, fill: '#d9d9d9', stroke: 'none' }
  if (type === 'LINE' || type === 'line') return { id, kind: 'line', name, x, y, w, h, stroke: '#1c1c1c' }
  if (type === 'GROUP' || type === 'group') return { id, kind: 'group', name, x, y, w, h, fill: 'none', stroke: 'none', children }
  return { id, kind: 'frame', name: type === 'FRAME' || type === 'frame' ? name : `${name}`, x, y, w, h, fill: '#ffffff', children }
}

function num(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

export async function importFigToDesign(buffer: ArrayBuffer): Promise<DesignImportResult> {
  try {
    const fig = await import('@open-pencil/fig')
    const parse = (fig as { parseFigBuffer?: (data: ArrayBuffer) => { graph?: unknown } }).parseFigBuffer
    if (typeof parse === 'function') return graphToDesign(parse(buffer).graph ?? parse(buffer))
  } catch {
    /* parser optional */
  }
  try {
    const text = new TextDecoder().decode(buffer)
    if (text.trim().startsWith('{')) return graphToDesign(JSON.parse(text))
  } catch {
    /* not json */
  }
  return { doc: emptyDesign(), error: 'Could not parse this .fig file' }
}

export async function importPenToDesign(text: string): Promise<DesignImportResult> {
  try {
    const pen = await import('@open-pencil/pen')
    const parse = (pen as { parsePenFile?: (raw: string) => unknown }).parsePenFile
    if (typeof parse === 'function') return graphToDesign(parse(text))
  } catch {
    /* parser optional */
  }
  try {
    return graphToDesign(JSON.parse(text))
  } catch {
    return { doc: emptyDesign(), error: 'Could not parse this .pen file' }
  }
}
