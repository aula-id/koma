import { unzipSync } from 'fflate'
import { cacheDesignImage, designAssetPath, mimeForName, putDesignImage } from './assets'
import { decodeVectorNetworkBlob } from './fig/vectorNetwork'
import { emptyDesign } from './model'
import type { DesignDoc, DesignLayout, DesignLayoutGrid, DesignNode, DesignPaint, DesignTextRun, DesignToken, DesignVector } from './types'

export type DesignImportImage = { hash: string; mime: string; bytes: Uint8Array; path: string }

export type DesignImportResult = { doc: DesignDoc; error: string | null; images?: DesignImportImage[] }

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

function guidId(value: { sessionID?: number; localID?: number } | string | undefined): string {
  if (!value) return ''
  if (typeof value === 'string') return value
  return `${value.sessionID ?? 0}:${value.localID ?? 0}`
}

function colorHex(color: { r?: number; g?: number; b?: number } | undefined): string | null {
  if (!color) return null
  const hex = (n: number) => Math.max(0, Math.min(255, Math.round((n ?? 0) * 255))).toString(16).padStart(2, '0')
  return `#${hex(color.r ?? 0)}${hex(color.g ?? 0)}${hex(color.b ?? 0)}`
}

function figPaint(paint: Record<string, unknown>): DesignPaint | null {
  if (paint.visible === false) return null
  const type = typeof paint.type === 'string' ? paint.type.toUpperCase() : 'SOLID'
  const opacity = typeof paint.opacity === 'number' && paint.opacity < 1 ? paint.opacity : undefined
  if (type.includes('GRADIENT')) {
    const stops = Array.isArray(paint.stops)
      ? paint.stops.flatMap((stop) => {
          if (!stop || typeof stop !== 'object') return []
          const item = stop as Record<string, unknown>
          const color = colorHex(item.color as { r?: number; g?: number; b?: number } | undefined)
          const at = typeof item.position === 'number' ? item.position : typeof item.at === 'number' ? item.at : 0
          return color ? [{ color, at }] : []
        })
      : []
    return {
      type: 'gradient',
      kind: type.includes('RADIAL') ? 'radial' : type.includes('ANGULAR') ? 'angular' : type.includes('DIAMOND') ? 'diamond' : 'linear',
      stops: stops.length >= 2 ? stops : [{ color: '#000000', at: 0 }, { color: '#ffffff', at: 1 }],
      opacity,
    }
  }
  if (type === 'IMAGE') {
    const image = paint.image && typeof paint.image === 'object' ? paint.image as Record<string, unknown> : null
    const hash = figBytesHash(image?.hash) || figBytesHash(paint.hash) || figBytesHash(paint.imageHash)
    if (!hash) return null
    return { type: 'image', hash, scale: 'fill', opacity }
  }
  const color = colorHex(paint.color as { r?: number; g?: number; b?: number } | undefined)
  return color ? { type: 'solid', color, opacity } : null
}

function figBytesHash(value: unknown): string {
  if (typeof value === 'string' && value) return value
  if (value instanceof Uint8Array && value.length) return Array.from(value, (byte) => byte.toString(16).padStart(2, '0')).join('')
  return ''
}

function figKind(type: string | undefined): DesignNode['kind'] {
  const raw = (type ?? '').toUpperCase()
  if (raw.includes('TEXT')) return 'text'
  if (raw.includes('ELLIPSE')) return 'ellipse'
  if (raw.includes('LINE')) return 'line'
  if (raw.includes('VECTOR') || raw.includes('BOOLEAN') || raw.includes('STAR') || raw.includes('POLYGON')) return 'vector'
  if (raw.includes('RECT')) return 'rect'
  if (raw.includes('GROUP')) return 'group'
  if (raw.includes('INSTANCE')) return 'instance'
  return 'frame'
}

function figVector(row: Record<string, unknown>, blobs: Uint8Array[]): DesignVector | undefined {
  const data = row.vectorData && typeof row.vectorData === 'object' ? row.vectorData as Record<string, unknown> : null
  if (data && Array.isArray(data.vertices) && data.vertices.length) {
    const vertices = data.vertices.flatMap((item) => {
      if (!item || typeof item !== 'object') return []
      const point = item as { x?: number; y?: number }
      return typeof point.x === 'number' && typeof point.y === 'number' ? [{ x: point.x, y: point.y }] : []
    })
    const segments = Array.isArray(data.segments)
      ? data.segments.flatMap((item) => {
          if (!item || typeof item !== 'object') return []
          const segment = item as { start?: number; end?: number; tangentStart?: { x?: number; y?: number }; tangentEnd?: { x?: number; y?: number } }
          if (typeof segment.start !== 'number' || typeof segment.end !== 'number') return []
          return [{
            start: segment.start,
            end: segment.end,
            tangentStart: { x: segment.tangentStart?.x ?? 0, y: segment.tangentStart?.y ?? 0 },
            tangentEnd: { x: segment.tangentEnd?.x ?? 0, y: segment.tangentEnd?.y ?? 0 },
          }]
        })
      : []
    if (vertices.length) return { vertices, segments, regions: [] }
  }
  const blobRef = data?.vectorNetworkBlob
  const blob = blobRef instanceof Uint8Array
    ? blobRef
    : typeof blobRef === 'number' && blobs[blobRef]
      ? blobs[blobRef]
      : null
  if (!blob) return undefined
  try {
    const vector = decodeVectorNetworkBlob(blob)
    return vector.vertices.length ? vector : undefined
  } catch {
    return undefined
  }
}

function figTextRuns(textData: Record<string, unknown>): DesignTextRun[] | undefined {
  const ids = Array.isArray(textData.characterStyleIDs) ? textData.characterStyleIDs : []
  const table = Array.isArray(textData.styleOverrideTable) ? textData.styleOverrideTable : []
  if (!ids.length || !table.length) return undefined
  const styles = new Map<number, Record<string, unknown>>()
  table.forEach((item, index) => {
    if (!item || typeof item !== 'object') return
    const row = item as Record<string, unknown>
    const id = typeof row.styleID === 'number' ? row.styleID : index
    styles.set(id, row)
  })
  const runs: DesignTextRun[] = []
  let start = 0
  let current = typeof ids[0] === 'number' ? ids[0] : 0
  for (let i = 1; i <= ids.length; i++) {
    const next = i < ids.length && typeof ids[i] === 'number' ? ids[i] as number : current
    if (i < ids.length && next === current) continue
    const style = styles.get(current)
    if (style && current) {
      const run: DesignTextRun = { start, end: i }
      if (typeof style.fontSize === 'number') run.fontSize = style.fontSize
      if (typeof style.fontName === 'object' && style.fontName && typeof (style.fontName as { family?: string }).family === 'string') {
        run.fontFamily = (style.fontName as { family: string }).family
      }
      const fills = Array.isArray(style.fillPaints) ? style.fillPaints.map((item) => figPaint(item as Record<string, unknown>)).find((item) => item?.color) : null
      if (fills?.color) run.color = fills.color
      runs.push(run)
    }
    start = i
    current = next
  }
  return runs.length ? runs : undefined
}

function figLayoutGrids(value: unknown): DesignLayoutGrid[] | undefined {
  if (!Array.isArray(value)) return undefined
  const grids: DesignLayoutGrid[] = []
  for (const item of value) {
    if (!item || typeof item !== 'object') continue
    const row = item as Record<string, unknown>
    const pattern = typeof row.pattern === 'string' ? row.pattern.toUpperCase() : typeof row.kind === 'string' ? row.kind.toUpperCase() : ''
    const axis = typeof row.axis === 'string' ? row.axis.toUpperCase() : ''
    const kind = pattern.includes('COL') || axis === 'X' ? 'column' : pattern.includes('ROW') || axis === 'Y' ? 'row' : pattern === 'GRID' || !pattern ? 'square' : 'square'
    const alignName = typeof row.type === 'string' ? row.type.toUpperCase() : ''
    const align = alignName === 'MIN' ? 'start' : alignName === 'CENTER' ? 'center' : alignName === 'MAX' ? 'end' : alignName === 'STRETCH' ? 'stretch' : undefined
    const next: DesignLayoutGrid = { kind }
    if (align && align !== 'stretch' && kind !== 'square') next.align = align
    const size = num(row.sectionSize ?? row.size)
    const gutter = num(row.gutterSize ?? row.gutter)
    const count = num(row.count)
    const offset = num(row.offset)
    if (size != null && size > 0) next.size = size
    if (gutter != null && gutter >= 0) next.gutter = gutter
    if (count != null && count > 0) next.count = Math.round(count)
    if (offset != null) next.offset = offset
    const color = colorHex(row.color as { r?: number; g?: number; b?: number } | undefined)
    if (color) next.color = color
    grids.push(next)
  }
  return grids.length ? grids : undefined
}

export function nodeChangesToDesign(
  changes: Array<Record<string, unknown>>,
  extras?: { blobs?: Uint8Array[]; images?: Array<[string, Uint8Array]> },
): DesignImportResult {
  const doc = emptyDesign()
  doc.version = 2
  const blobs = extras?.blobs ?? []
  const nodes = new Map<string, DesignNode>()
  const children = new Map<string, string[]>()
  const components: DesignDoc['components'] = []
  for (const row of changes) {
    if (row.phase === 'REMOVED') continue
    const id = guidId(row.guid as { sessionID?: number; localID?: number } | string | undefined) || mint('fig', { i: nodes.size })
    const size = row.size && typeof row.size === 'object' ? row.size as { x?: number; y?: number } : {}
    const transform = row.transform && typeof row.transform === 'object' ? row.transform as { m02?: number; m12?: number } : {}
    const fills = Array.isArray(row.fillPaints) ? row.fillPaints.map((item) => figPaint(item as Record<string, unknown>)).filter((item): item is DesignPaint => !!item) : []
    const strokes = Array.isArray(row.strokePaints) ? row.strokePaints.map((item) => figPaint(item as Record<string, unknown>)).filter((item): item is DesignPaint => !!item) : []
    const rawType = typeof row.type === 'string' ? row.type.toUpperCase() : ''
    const kind = figKind(rawType)
    const node: DesignNode = {
      id,
      kind,
      name: typeof row.name === 'string' ? row.name : kind,
      x: transform.m02 ?? 0,
      y: transform.m12 ?? 0,
      w: Math.max(1, size.x ?? 100),
      h: Math.max(1, size.y ?? 100),
    }
    if (fills.length) {
      node.fills = fills
      if (fills[0].color) node.fill = fills[0].color
    }
    if (strokes.length) {
      node.strokes = strokes
      if (strokes[0].color) node.stroke = strokes[0].color
    }
    if (typeof row.strokeWeight === 'number') node.strokeWidth = row.strokeWeight
    if (typeof row.cornerRadius === 'number' && row.cornerRadius > 0) node.radius = row.cornerRadius
    if (typeof row.opacity === 'number' && row.opacity < 1) node.opacity = row.opacity
    if (row.visible === false) node.visible = false
    if (row.locked === true) node.locked = true
    if (row.mask === true) node.mask = true
    const maskType = typeof row.maskType === 'string' ? row.maskType.toUpperCase() : ''
    if (maskType.includes('LUM')) node.maskType = 'luminance'
    else if (maskType.includes('VECTOR') || row.maskIsOutline === true) node.maskType = 'vector'
    if (row.stackMode === 'HORIZONTAL') node.layout = 'row'
    if (row.stackMode === 'VERTICAL') node.layout = 'column'
    if (typeof row.stackSpacing === 'number') node.gap = row.stackSpacing
    if (typeof row.fontSize === 'number') node.fontSize = row.fontSize
    if (typeof row.fontName === 'object' && row.fontName && typeof (row.fontName as { family?: string }).family === 'string') {
      node.fontFamily = (row.fontName as { family: string }).family
    }
    if (row.textData && typeof row.textData === 'object') {
      const textData = row.textData as Record<string, unknown>
      const text = typeof textData.characters === 'string' ? textData.characters : undefined
      if (text != null) node.text = text
      const runs = figTextRuns(textData)
      if (runs) node.runs = runs
    }
    const vector = figVector(row, blobs)
    if (vector && (kind === 'vector' || rawType.includes('BOOLEAN') || rawType.includes('STAR') || rawType.includes('POLYGON'))) {
      node.kind = 'vector'
      node.vector = vector
    } else if (kind === 'vector' && !node.vector) {
      node.vector = {
        vertices: [{ x: 0, y: 0 }, { x: node.w, y: 0 }, { x: node.w, y: node.h }, { x: 0, y: node.h }],
        segments: [
          { start: 0, end: 1, tangentStart: { x: 0, y: 0 }, tangentEnd: { x: 0, y: 0 } },
          { start: 1, end: 2, tangentStart: { x: 0, y: 0 }, tangentEnd: { x: 0, y: 0 } },
          { start: 2, end: 3, tangentStart: { x: 0, y: 0 }, tangentEnd: { x: 0, y: 0 } },
          { start: 3, end: 0, tangentStart: { x: 0, y: 0 }, tangentEnd: { x: 0, y: 0 } },
        ],
        regions: [{ winding: 'nonzero', loops: [[0, 1, 2, 3]] }],
      }
    }
    if (kind === 'instance') {
      const symbol = row.symbolData && typeof row.symbolData === 'object' ? (row.symbolData as { symbolID?: { sessionID?: number; localID?: number } | string }).symbolID : undefined
      const component = guidId(symbol)
      if (component) node.component = component
    }
    const grids = figLayoutGrids(row.layoutGrids)
    if (grids) node.layoutGrids = grids
    if (rawType.includes('BOOLEAN')) {
      const op = typeof row.booleanOperation === 'string' ? row.booleanOperation.toUpperCase() : ''
      if (op === 'UNION') node.booleanOp = 'union'
      else if (op === 'SUBTRACT') node.booleanOp = 'subtract'
      else if (op === 'INTERSECT') node.booleanOp = 'intersect'
      else if (op === 'XOR') node.booleanOp = 'exclude'
    }
    nodes.set(id, node)
    if (rawType === 'COMPONENT' || rawType === 'SYMBOL' || rawType.includes('COMPONENT')) {
      components.push({ id, name: node.name ?? 'Component', variants: [{ props: {}, node: { ...node, kind: 'frame' } }] })
    }
    const parent = row.parentIndex && typeof row.parentIndex === 'object' ? guidId((row.parentIndex as { guid?: { sessionID?: number; localID?: number } }).guid) : ''
    if (parent) {
      const list = children.get(parent) ?? []
      list.push(id)
      children.set(parent, list)
    }
  }
  const attach = (node: DesignNode): DesignNode => {
    const kids = (children.get(node.id) ?? []).map((id) => nodes.get(id)).filter((item): item is DesignNode => !!item).map(attach)
    return kids.length ? { ...node, children: kids } : node
  }
  const roots = [...nodes.values()].filter((node) => ![...children.values()].some((list) => list.includes(node.id)))
  doc.screens = (roots.length ? roots : [...nodes.values()]).map(attach)
  if (components.length) {
    doc.components = components.map((component) => {
      const live = nodes.get(component.id)
      const tree = live ? attach(live) : component.variants[0].node
      return { ...component, variants: [{ props: {}, node: { ...tree, kind: 'frame', x: 0, y: 0 } }] }
    })
  }
  const images: DesignImportImage[] = []
  if (extras?.images?.length) {
    let next = doc
    for (const [name, data] of extras.images) {
      const hash = name.replace(/^images\//, '').replace(/\.[^.]+$/, '')
      if (!hash || !data?.length) continue
      const mime = mimeForName(name) ?? 'image/png'
      const path = designAssetPath(hash, mime)
      cacheDesignImage(hash, data, mime)
      next = putDesignImage(next, hash, mime, path)
      images.push({ hash, mime, bytes: data, path })
    }
    doc.images = next.images
  }
  if (!doc.screens.length) return { doc, error: 'No pages or nodes to import', images }
  return { doc, error: null, images }
}

export async function importFigToDesign(buffer: ArrayBuffer): Promise<DesignImportResult> {
  try {
    const { parseFigBuffer } = await import('./fig/parseFigBuffer')
    const parsed = parseFigBuffer(buffer)
    if (parsed?.nodeChanges?.length) {
      return nodeChangesToDesign(parsed.nodeChanges as Array<Record<string, unknown>>, {
        blobs: parsed.blobs,
        images: parsed.images,
      })
    }
  } catch {
    /* fallback */
  }
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
