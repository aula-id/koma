// Diagrams go to the model as Mermaid. The drawing the user sees is a separate
// snapshot of the canvas (see DiagramObservationCard).
import {
  edgeRoute,
  edgeStyle,
  emptyDiagram,
  nodeCenter,
  rotatePoint,
  type DiagramDoc,
  type DiagramEdge,
  type DiagramKind,
  type DiagramNode,
  type DiagramPoint,
} from './diagram'
export type DiagramRect = { x: number; y: number; w: number; h: number }

const views = new Map<string, DiagramDoc>()

export function rememberDiagramView(mermaid: string, doc: DiagramDoc): void {
  views.set(mermaid, doc)
}

export function diagramViewForMermaid(mermaid: string): { doc: DiagramDoc; captured: boolean } {
  const captured = views.get(mermaid)
  if (captured) return { doc: captured, captured: true }
  return { doc: parseMermaidDiagram(mermaid).doc, captured: false }
}

export function nodeBounds(node: DiagramNode): DiagramRect {
  const center = nodeCenter(node)
  const corners = [
    { x: node.x, y: node.y },
    { x: node.x + node.w, y: node.y },
    { x: node.x + node.w, y: node.y + node.h },
    { x: node.x, y: node.y + node.h },
  ].map((point) => rotatePoint(point, center, node.rotation ?? 0))
  const xs = corners.map((point) => point.x)
  const ys = corners.map((point) => point.y)
  const x = Math.min(...xs)
  const y = Math.min(...ys)
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y }
}

function rectsIntersect(a: DiagramRect, b: DiagramRect): boolean {
  return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
}

function pointInRect(point: DiagramPoint, rect: DiagramRect): boolean {
  return point.x >= rect.x && point.x <= rect.x + rect.w && point.y >= rect.y && point.y <= rect.y + rect.h
}

function cross(ax: number, ay: number, bx: number, by: number): number {
  return ax * by - ay * bx
}

function segmentsCross(a: DiagramPoint, b: DiagramPoint, c: DiagramPoint, d: DiagramPoint): boolean {
  const d1 = cross(b.x - a.x, b.y - a.y, c.x - a.x, c.y - a.y)
  const d2 = cross(b.x - a.x, b.y - a.y, d.x - a.x, d.y - a.y)
  const d3 = cross(d.x - c.x, d.y - c.y, a.x - c.x, a.y - c.y)
  const d4 = cross(d.x - c.x, d.y - c.y, b.x - c.x, b.y - c.y)
  return ((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0))
}

function segmentHitsRect(a: DiagramPoint, b: DiagramPoint, rect: DiagramRect): boolean {
  if (pointInRect(a, rect) || pointInRect(b, rect)) return true
  const right = rect.x + rect.w
  const bottom = rect.y + rect.h
  const edges: [DiagramPoint, DiagramPoint][] = [
    [{ x: rect.x, y: rect.y }, { x: right, y: rect.y }],
    [{ x: right, y: rect.y }, { x: right, y: bottom }],
    [{ x: right, y: bottom }, { x: rect.x, y: bottom }],
    [{ x: rect.x, y: bottom }, { x: rect.x, y: rect.y }],
  ]
  return edges.some(([c, d]) => segmentsCross(a, b, c, d))
}

function routeHits(points: DiagramPoint[], rect: DiagramRect): boolean {
  for (let i = 1; i < points.length; i++) {
    if (segmentHitsRect(points[i - 1], points[i], rect)) return true
  }
  return false
}

/** Nodes and connectors that meet the rectangle. A connector keeps both ends. */
export function captureDiagram(doc: DiagramDoc, area: DiagramRect): DiagramDoc {
  const byId = new Map(doc.nodes.map((node) => [node.id, node]))
  const hit = new Set<string>()
  for (const node of doc.nodes) {
    if (rectsIntersect(nodeBounds(node), area)) hit.add(node.id)
  }
  const edges: DiagramEdge[] = []
  for (const edge of doc.edges) {
    if (edge.stroke === false) continue
    const from = byId.get(edge.from)
    const to = byId.get(edge.to)
    if (!from || !to) continue
    const both = hit.has(from.id) && hit.has(to.id)
    if (!both && !routeHits(edgeRoute(from, to, edge), area)) continue
    edges.push(edge)
    hit.add(from.id)
    hit.add(to.id)
  }
  return {
    snap: doc.snap,
    grid: doc.grid,
    nodes: doc.nodes.filter((node) => hit.has(node.id)),
    edges,
  }
}

function mermaidId(id: string, used: Set<string>): string {
  let base = id.replace(/[^A-Za-z0-9_]/g, '_')
  if (!/^[A-Za-z]/.test(base)) base = `n_${base}`
  if (!base) base = 'n'
  let next = base
  let salt = 2
  while (used.has(next)) next = `${base}_${salt++}`
  used.add(next)
  return next
}

function quote(text: string): string {
  const clean = (text || ' ').replace(/\r?\n/g, ' ').replace(/\\/g, '\\\\').replace(/"/g, '#quot;')
  return `"${clean}"`
}

function unquote(text: string): string {
  return text.replace(/\\\\/g, '\\').replace(/#quot;/g, '"')
}

function shapeLine(kind: DiagramKind, id: string, label: string): string {
  const text = quote(label)
  if (kind === 'ellipse') return `${id}((${text}))`
  if (kind === 'diamond') return `${id}{${text}}`
  return `${id}[${text}]`
}

function linkToken(edge: DiagramEdge): string | null {
  const style = edgeStyle(edge)
  if (!style.stroke) return null
  const dashed = style.dash !== 'solid'
  const start = style.start !== 'none'
  const end = style.end !== 'none'
  if (dashed && start && end) return '<-.->'
  if (dashed && start) return '<-.-'
  if (dashed && end) return '-.->'
  if (dashed) return '-.-'
  if (start && end) return '<-->'
  if (start) return '<--'
  if (end) return '-->'
  return '---'
}

/** Fenced Mermaid for one diagram. Empty when there is nothing to send. */
export function diagramToMermaid(doc: DiagramDoc, title?: string, notes?: { path: string; doc?: DiagramDoc }): string {
  const visible = doc.edges.filter((edge) => edge.stroke !== false)
  if (!doc.nodes.length && !visible.length) return ''
  const used = new Set<string>()
  const ids = new Map<string, string>()
  const alias = (id: string) => {
    const existing = ids.get(id)
    if (existing) return existing
    const next = mermaidId(id, used)
    ids.set(id, next)
    return next
  }
  const nodes = [...doc.nodes].sort((a, b) => a.y - b.y || a.x - b.x || a.id.localeCompare(b.id))
  const lines: string[] = ['flowchart TD']
  const heading = title?.replace(/[\r\n]/g, ' ').trim()
  if (heading) lines.push(`  %% ${heading}`)
  for (const node of nodes) {
    const id = alias(node.id)
    if (node.kind === 'text') lines.push(`  %% koma-text ${id}`)
  }
  for (const node of nodes) lines.push(`  ${shapeLine(node.kind, alias(node.id), node.text)}`)
  const byId = new Map(doc.nodes.map((node) => [node.id, node]))
  for (const edge of doc.edges) {
    const token = linkToken(edge)
    const from = byId.get(edge.from)
    const to = byId.get(edge.to)
    if (!token || !from || !to) continue
    const label = edge.text?.replace(/[\r\n|]/g, ' ').replace(/"/g, '#quot;').trim()
    const marked = label ? `${token}|${label}|` : token
    lines.push(`  ${alias(edge.from)} ${marked} ${alias(edge.to)}`)
  }
  return `\`\`\`mermaid\n${lines.join('\n')}\n\`\`\``
}

const NODE_LINE = /^([A-Za-z_][A-Za-z0-9_]*)(?:\(\("(.*)"\)\)|\{"(.*)"\}|\["(.*)"\])$/
const EDGE_LINE = /^([A-Za-z_][A-Za-z0-9_]*)\s+(<-\.->|-\.->|<-.-|-\.-|<-->|<--|-->|---)(?:\|([^|\n]*)\|)?\s+([A-Za-z_][A-Za-z0-9_]*)\s*$/

function kindFor(match: RegExpMatchArray): DiagramKind {
  if (match[2] != null) return 'ellipse'
  if (match[3] != null) return 'diamond'
  return 'rect'
}

function labelFor(match: RegExpMatchArray): string {
  return unquote(match[2] ?? match[3] ?? match[4] ?? '')
}

function edgeFromToken(token: string): Pick<DiagramEdge, 'dash' | 'start' | 'end'> {
  const dashed = token.includes('.')
  const start = token.startsWith('<')
  const end = token.endsWith('>')
  return {
    ...(dashed ? { dash: 'dashed' as const } : {}),
    ...(start ? { start: 'arrow' as const } : {}),
    ...(end ? { end: 'arrow' as const } : { end: 'none' as const }),
  }
}

export function parseMermaidDiagram(mermaid: string): { doc: DiagramDoc; title: string } {
  const body = mermaid.replace(/^```[ \t]*mermaid[ \t]*\r?\n?/i, '').replace(/```[\s]*$/, '')
  const doc = emptyDiagram()
  const textIds = new Set<string>()
  let title = 'Diagram'
  const known = new Set<string>()
  const ensure = (id: string) => {
    if (known.has(id)) return
    known.add(id)
    const index = doc.nodes.length
    doc.nodes.push({ id, kind: 'rect', x: 24, y: 24 + index * 88, w: 160, h: 48, text: id })
  }
  for (const raw of body.split(/\r?\n/)) {
    const line = raw.trim()
    if (!line || line === 'flowchart TD' || line === 'flowchart LR') continue
    if (line.startsWith('%% koma-text ')) {
      textIds.add(line.slice('%% koma-text '.length).trim())
      continue
    }
    if (line.startsWith('%% ')) {
      if (title === 'Diagram') title = line.slice(3).trim() || title
      continue
    }
    const edge = line.match(EDGE_LINE)
    if (edge?.[1] && edge[2] && edge[4]) {
      ensure(edge[1])
      ensure(edge[4])
      const label = edge[3]?.trim() ? unquote(edge[3].trim()) : ''
      doc.edges.push({
        id: `e${doc.edges.length + 1}`,
        from: edge[1],
        to: edge[4],
        ...(label ? { text: label } : {}),
        ...edgeFromToken(edge[2]),
      })
      continue
    }
    const node = line.match(NODE_LINE)
    if (!node) continue
    ensure(node[1])
    const found = doc.nodes.find((item) => item.id === node[1])
    if (!found) continue
    found.kind = kindFor(node)
    found.text = labelFor(node)
  }
  for (const node of doc.nodes) {
    if (textIds.has(node.id)) node.kind = 'text'
  }
  return { doc, title }
}

export function mermaidTitle(mermaid: string): string {
  return parseMermaidDiagram(mermaid).title
}

export type DiagramMessagePart =
  | { type: 'text'; text: string }
  | { type: 'diagram'; mermaid: string }

const MERMAID_FENCE = /```[ \t]*mermaid[ \t]*\r?\n[\s\S]*?```/gi

/** Pull fenced Mermaid out of a user message. The fence is what the model read. */
export function splitDiagramMessage(content: string): { prose: string; diagrams: { mermaid: string }[] } {
  const diagrams: { mermaid: string }[] = []
  const prose = content.replace(new RegExp(MERMAID_FENCE.source, 'gi'), (fence) => {
    diagrams.push({ mermaid: fence })
    return ''
  })
  return { prose: prose.replace(/\n{3,}/g, '\n\n').trim(), diagrams }
}

/** Mermaid fences in the order they appear, with the prose between them kept. */
export function diagramMessageParts(content: string): DiagramMessagePart[] {
  const parts: DiagramMessagePart[] = []
  const re = new RegExp(MERMAID_FENCE.source, 'gi')
  let last = 0
  for (const match of content.matchAll(re)) {
    const index = match.index ?? 0
    if (index > last) parts.push({ type: 'text', text: content.slice(last, index) })
    parts.push({ type: 'diagram', mermaid: match[0] })
    last = index + match[0].length
  }
  if (last < content.length) parts.push({ type: 'text', text: content.slice(last) })
  return parts
}

export function pointInDiagramRect(point: DiagramPoint, rect: DiagramRect): boolean {
  return pointInRect(point, rect)
}
