import { unzipSync } from 'fflate'
import { emptyDesign } from './model'
import type { DesignDoc, DesignLayout, DesignNode, DesignPaint, DesignToken } from './types'

export type DesignImportResult = { doc: DesignDoc; error: string | null }

function mint(prefix: string, n: { i: number }): string {
  n.i += 1
  return `${prefix}${n.i}`
}

function num(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function hexColor(value: unknown): string | null {
  if (typeof value !== 'string') return null
  if (value.startsWith('$')) return value.slice(1)
  if (/^#[0-9a-fA-F]{3,8}$/.test(value)) {
    const body = value.slice(1)
    if (body.length === 3) return `#${body.split('').map((item) => item + item).join('')}`.toLowerCase()
    return `#${body.slice(0, 6).toLowerCase()}`
  }
  if (value === 'none' || value === 'transparent') return 'none'
  return null
}

function paintFromUnknown(value: unknown): DesignPaint | null {
  if (typeof value === 'string') {
    const color = hexColor(value)
    return color ? { type: 'solid', color } : null
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const row = value as Record<string, unknown>
  if (row.enabled === false) return null
  const type = typeof row.type === 'string' ? row.type.toLowerCase() : 'solid'
  if (type === 'image' && typeof row.hash === 'string') return { type: 'image', hash: row.hash }
  if (type.includes('gradient')) {
    const stops = Array.isArray(row.stops)
      ? row.stops.flatMap((stop) => {
          if (!stop || typeof stop !== 'object') return []
          const item = stop as Record<string, unknown>
          const color = hexColor(item.color ?? item.hex)
          const at = num(item.at ?? item.position) ?? 0
          return color ? [{ color, at }] : []
        })
      : []
    return { type: 'gradient', kind: type.includes('radial') ? 'radial' : 'linear', stops: stops.length ? stops : [{ color: '#000000', at: 0 }, { color: '#ffffff', at: 1 }] }
  }
  const color = hexColor(row.color ?? row.hex)
  return color ? { type: 'solid', color } : null
}

function layoutFrom(value: unknown): DesignLayout | undefined {
  const raw = typeof value === 'string' ? value.toLowerCase() : ''
  if (raw === 'row' || raw === 'horizontal' || raw === 'flex-row') return 'row'
  if (raw === 'column' || raw === 'vertical' || raw === 'flex-col' || raw === 'flex') return 'column'
  if (raw === 'grid') return 'grid'
  return undefined
}

function mapUnknownNode(value: unknown, counter: { i: number }): DesignNode {
  const row = value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
  const type = typeof row.type === 'string' ? row.type.toUpperCase() : typeof row.kind === 'string' ? row.kind.toUpperCase() : 'FRAME'
  const id = typeof row.id === 'string' && row.id ? row.id : mint('imp', counter)
  const name = typeof row.name === 'string' && row.name ? row.name : type
  const x = num(row.x) ?? num(row.left) ?? 0
  const y = num(row.y) ?? num(row.top) ?? 0
  const w = Math.max(1, num(row.w) ?? num(row.width) ?? 100)
  const h = Math.max(1, num(row.h) ?? num(row.height) ?? 100)
  const children = Array.isArray(row.children) ? row.children.map((child) => mapUnknownNode(child, counter)) : undefined
  const fillPaint = Array.isArray(row.fill) ? paintFromUnknown(row.fill[0]) : paintFromUnknown(row.fill)
  const node: DesignNode = { id, kind: 'frame', name, x, y, w, h }
  if (fillPaint?.color) node.fill = fillPaint.color
  if (fillPaint) node.fills = [fillPaint]
  if (typeof row.opacity === 'number') node.opacity = row.opacity
  if (typeof row.rotation === 'number') node.rotation = row.rotation
  if (row.clip === false) node.clip = false
  const layout = layoutFrom(row.layout ?? row.layoutMode)
  if (layout) node.layout = layout
  const gap = num(row.gap)
  if (gap != null) node.gap = gap
  if (row.padding != null) {
    if (typeof row.padding === 'number') node.pad = row.padding
    else if (Array.isArray(row.padding)) {
      const [a, b, c, d] = row.padding.map((item) => num(item) ?? 0)
      if (row.padding.length === 4) {
        node.padTop = a
        node.padRight = b
        node.padBottom = c
        node.padLeft = d
      } else if (row.padding.length === 2) {
        node.padTop = a
        node.padBottom = a
        node.padLeft = b
        node.padRight = b
      }
    }
  }
  if (type === 'TEXT' || type === 'text') {
    return {
      ...node,
      kind: 'text',
      text: typeof row.characters === 'string' ? row.characters : typeof row.content === 'string' ? row.content : typeof row.text === 'string' ? row.text : 'Text',
      fontSize: num(row.fontSize) ?? 14,
      fontFamily: typeof row.fontFamily === 'string' ? row.fontFamily : undefined,
    }
  }
  if (type === 'ELLIPSE' || type === 'ellipse') return { ...node, kind: 'ellipse', fill: node.fill ?? '#d9d9d9', stroke: 'none' }
  if (type === 'RECTANGLE' || type === 'RECT' || type === 'rect') return { ...node, kind: 'rect', fill: node.fill ?? '#d9d9d9', stroke: 'none' }
  if (type === 'LINE' || type === 'line') return { ...node, kind: 'line', stroke: '#1c1c1c' }
  if (type === 'GROUP' || type === 'group') return { ...node, kind: 'group', fill: node.fill ?? 'none', stroke: 'none', children }
  if (type === 'VECTOR' || type === 'PATH' || type === 'BOOLEAN_OPERATION') return { ...node, kind: 'vector', fill: node.fill ?? 'none', stroke: '#1c1c1c', vector: { vertices: [{ x: 0, y: 0 }, { x: w, y: 0 }, { x: w, y: h }, { x: 0, y: h }], segments: [{ start: 0, end: 1, tangentStart: { x: 0, y: 0 }, tangentEnd: { x: 0, y: 0 } }, { start: 1, end: 2, tangentStart: { x: 0, y: 0 }, tangentEnd: { x: 0, y: 0 } }, { start: 2, end: 3, tangentStart: { x: 0, y: 0 }, tangentEnd: { x: 0, y: 0 } }, { start: 3, end: 0, tangentStart: { x: 0, y: 0 }, tangentEnd: { x: 0, y: 0 } }], regions: [{ winding: 'nonzero', loops: [[0, 1, 2, 3]] }] } }
  if (type === 'INSTANCE' && typeof row.ref === 'string') return { ...node, kind: 'instance', component: row.ref }
  if (type === 'SECTION') return { ...node, kind: 'frame', section: true, children }
  return { ...node, children }
}

function extractNodes(graph: unknown): unknown[] {
  if (!graph || typeof graph !== 'object') return []
  const row = graph as Record<string, unknown>
  if (Array.isArray(row.pages)) return row.pages
  if (Array.isArray(row.screens)) return row.screens
  if (Array.isArray(row.children)) return row.children
  if (row.document && typeof row.document === 'object') return extractNodes(row.document)
  if (row.graph && typeof row.graph === 'object') return extractNodes(row.graph)
  return [graph]
}

function tokensFromPen(variables: unknown, themes: unknown): DesignToken[] {
  if (!variables || typeof variables !== 'object') return []
  const modes = themes && typeof themes === 'object' ? Object.values(themes as Record<string, string[]>).flat() : ['light']
  const tokens: DesignToken[] = []
  for (const [name, def] of Object.entries(variables as Record<string, unknown>)) {
    if (!def || typeof def !== 'object') continue
    const row = def as Record<string, unknown>
    const kind = row.type === 'color' ? 'color' : row.type === 'number' ? 'space' : 'type'
    const values: Record<string, string> = {}
    if (Array.isArray(row.value)) {
      row.value.forEach((entry, index) => {
        if (!entry || typeof entry !== 'object') return
        const item = entry as Record<string, unknown>
        const mode = item.theme && typeof item.theme === 'object' ? Object.values(item.theme as Record<string, string>)[0] : modes[index] ?? 'light'
        values[String(mode)] = String(item.value ?? '')
      })
    } else if (row.value != null) {
      values[modes[0] ?? 'light'] = String(row.value)
    }
    if (Object.keys(values).length) tokens.push({ name, kind, values })
  }
  return tokens
}

/** Best-effort Open-Pencil / Figma-ish graph → DesignDoc. Unknown nodes become named frames. */
export function graphToDesign(graph: unknown): DesignImportResult {
  const doc = emptyDesign()
  doc.version = 2
  const counter = { i: 0 }
  const nodes = extractNodes(graph)
  if (!nodes.length) return { doc, error: 'No pages or nodes to import' }
  doc.screens = nodes.map((node) => mapUnknownNode(node, counter))
  const row = graph && typeof graph === 'object' ? graph as Record<string, unknown> : {}
  const tokens = tokensFromPen(row.variables, row.themes)
  if (tokens.length) doc.tokens = tokens
  return { doc, error: null }
}

function parseJsonText(text: string): unknown | null {
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

async function parseFigArchive(buffer: ArrayBuffer): Promise<unknown | null> {
  try {
    const bytes = new Uint8Array(buffer)
    const archive = unzipSync(bytes)
    for (const [name, data] of Object.entries(archive)) {
      if (!data || !name.toLowerCase().endsWith('.json')) continue
      const parsed = parseJsonText(new TextDecoder().decode(data))
      if (parsed) return parsed
    }
    for (const data of Object.values(archive)) {
      if (!data) continue
      const text = new TextDecoder().decode(data)
      if (text.trim().startsWith('{') || text.trim().startsWith('[')) {
        const parsed = parseJsonText(text)
        if (parsed) return parsed
      }
    }
  } catch {
    /* not a zip archive */
  }
  return null
}

export async function importFigToDesign(buffer: ArrayBuffer): Promise<DesignImportResult> {
  try {
    const text = new TextDecoder().decode(buffer)
    if (text.trim().startsWith('{')) return graphToDesign(JSON.parse(text))
  } catch {
    /* binary */
  }
  const graph = await parseFigArchive(buffer)
  if (graph) return graphToDesign(graph)
  return { doc: emptyDesign(), error: 'Could not parse this .fig file' }
}

export async function importPenToDesign(text: string): Promise<DesignImportResult> {
  try {
    const parsed = JSON.parse(text)
    return graphToDesign(parsed)
  } catch {
    return { doc: emptyDesign(), error: 'Could not parse this .pen file' }
  }
}
