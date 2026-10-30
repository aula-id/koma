// Diagrams go to the model as Mermaid. The drawing the user sees is a separate
// snapshot of the canvas (see DiagramObservationCard).
import {
  edgeRoute,
  edgeStyle,
  emptyDiagram,
  isErArrow,
  GROUP_TITLE,
  allRows,
  findRow,
  nodeBounds,
  tableHeight,
  type DiagramArrow,
  type DiagramDoc,
  type DiagramEdge,
  type DiagramKey,
  type DiagramKind,
  type DiagramNode,
  type DiagramPoint,
  type DiagramRect,
  type DiagramRow,
} from './diagram'

export { nodeBounds }
export type { DiagramRect }

const views = new Map<string, DiagramDoc>()

export function rememberDiagramView(mermaid: string, doc: DiagramDoc): void {
  views.set(mermaid, doc)
}

export function diagramViewForMermaid(mermaid: string): { doc: DiagramDoc; captured: boolean } {
  const captured = views.get(mermaid)
  if (captured) return { doc: captured, captured: true }
  return { doc: parseMermaidDiagram(mermaid).doc, captured: false }
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

/**
 * Flowchart brackets per kind, most specific first so parsing tries `((` before `(`.
 * Kinds without a bracket form use Mermaid's `@{ shape: … }` syntax (see SHAPE_NAMES).
 */
const BRACKETS: { kind: DiagramKind; open: string; close: string }[] = [
  { kind: 'ellipse', open: '((', close: '))' },
  { kind: 'stadium', open: '([', close: '])' },
  { kind: 'process', open: '[[', close: ']]' },
  { kind: 'cylinder', open: '[(', close: ')]' },
  { kind: 'hexagon', open: '{{', close: '}}' },
  { kind: 'parallelogram', open: '[/', close: '/]' },
  { kind: 'trapezoid', open: '[/', close: '\\]' },
  { kind: 'rounded', open: '(', close: ')' },
  { kind: 'diamond', open: '{', close: '}' },
  { kind: 'rect', open: '[', close: ']' },
]

/** Mermaid's named shapes, for kinds with no bracket form and for reading what models write. */
const SHAPE_NAMES: Record<string, DiagramKind> = {
  doc: 'document',
  document: 'document',
  tri: 'triangle',
  triangle: 'triangle',
  'extract': 'triangle',
  cloud: 'cloud',
  rect: 'rect',
  rounded: 'rounded',
  stadium: 'stadium',
  'subproc': 'process',
  cyl: 'cylinder',
  circle: 'ellipse',
  diam: 'diamond',
  hex: 'hexagon',
  'lean-r': 'parallelogram',
  'trap-b': 'trapezoid',
}

function shapeLine(kind: DiagramKind, id: string, label: string): string {
  const text = quote(label)
  const bracket = BRACKETS.find((item) => item.kind === kind)
  if (bracket) return `${id}${bracket.open}${text}${bracket.close}`
  const name = kind === 'document' ? 'doc' : kind === 'triangle' ? 'tri' : kind === 'cloud' ? 'cloud' : null
  return name ? `${id}@{ shape: ${name}, label: ${text} }` : `${id}[${text}]`
}

function labelText(raw: string): string {
  const inner = raw.trim()
  return unquote(inner.length >= 2 && inner.startsWith('"') && inner.endsWith('"') ? inner.slice(1, -1) : inner)
}

/** `id[…]`, `id((…))`, `id@{ shape: doc, label: "…" }` and friends. */
function parseNodeLine(line: string): { id: string; kind: DiagramKind; label: string } | null {
  const head = line.match(/^([A-Za-z_][A-Za-z0-9_]*)(.*)$/)
  if (!head?.[1] || !head[2]) return null
  const id = head[1]
  const rest = head[2].trim()
  const named = rest.match(/^@\{(.*)\}$/)
  if (named) {
    const body = named[1] ?? ''
    const shape = body.match(/shape\s*:\s*([A-Za-z-]+)/)?.[1]?.toLowerCase()
    const label = body.match(/label\s*:\s*("(?:[^"\\]|\\.)*"|[^,}]*)/)?.[1] ?? ''
    return { id, kind: (shape && SHAPE_NAMES[shape]) || 'rect', label: labelText(label) }
  }
  for (const bracket of BRACKETS) {
    if (rest.startsWith(bracket.open) && rest.endsWith(bracket.close) && rest.length >= bracket.open.length + bracket.close.length) {
      return { id, kind: bracket.kind, label: labelText(rest.slice(bracket.open.length, rest.length - bracket.close.length)) }
    }
  }
  return null
}

function linkToken(edge: DiagramEdge): string | null {
  const style = edgeStyle(edge)
  if (!style.stroke) return null
  const dashed = style.dash !== 'solid'
  // Crow's feet only mean something in an erDiagram; in a flowchart they read as a plain line.
  const start = style.start !== 'none' && !isErArrow(style.start)
  const end = style.end !== 'none' && !isErArrow(style.end)
  if (dashed && start && end) return '<-.->'
  if (dashed && start) return '<-.-'
  if (dashed && end) return '-.->'
  if (dashed) return '-.-'
  if (start && end) return '<-->'
  if (start) return '<--'
  if (end) return '-->'
  return '---'
}

function fence(body: string): string {
  return `\`\`\`mermaid\n${body}\n\`\`\``
}

/** Mermaid attribute types are one word. A missing type reads back as `string`. */
function erType(type: string | undefined): string {
  const clean = type?.trim().replace(/\s+/g, '_') ?? ''
  return clean || 'string'
}

function erIdent(name: string): string {
  const clean = name.trim().replace(/\s+/g, '_')
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(clean) ? clean : 'column'
}

function keySuffix(key: DiagramKey | undefined): string {
  if (key === 'pk') return ' PK'
  if (key === 'fk') return ' FK'
  if (key === 'pk,fk') return ' "PK, FK"'
  return ''
}

/** Left-hand cardinality of an erDiagram relation, read from the line's start head. */
function leftCard(arrow: DiagramArrow): string {
  if (arrow === 'erZeroToOne') return '|o'
  if (arrow === 'erZeroToMany') return '}o'
  if (arrow === 'erOneToMany' || arrow === 'erMany') return '}|'
  return '||'
}

/** Right-hand cardinality. The pair is the mirror of `leftCard`. */
function rightCard(arrow: DiagramArrow): string {
  if (arrow === 'erZeroToOne') return 'o|'
  if (arrow === 'erZeroToMany') return 'o{'
  if (arrow === 'erOneToMany' || arrow === 'erMany') return '|{'
  return '||'
}

/**
 * Fenced Mermaid. Tables and the lines between them go in an `erDiagram` block.
 * Everything else stays a `flowchart TD`. Empty when there is nothing to send.
 */
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
  const tables = nodes.filter((node) => node.kind === 'table')
  // Entity names come from the table title, so two "users" tables stay distinct.
  for (const table of tables) ids.set(table.id, mermaidId(table.text || 'Table', used))
  // SQL tables go to an erDiagram, document tables to a classDiagram (nesting needs classes).
  const sqlTables = tables.filter((table) => table.dialect !== 'document')
  const docTables = tables.filter((table) => table.dialect === 'document')
  const sqlIds = new Set(sqlTables.map((table) => table.id))
  const docIds = new Set(docTables.map((table) => table.id))
  const tableIds = new Set(tables.map((table) => table.id))
  const byId = new Map(doc.nodes.map((node) => [node.id, node]))
  const flowTable = new Set<string>()
  const erEdges: DiagramEdge[] = []
  const classEdges: DiagramEdge[] = []
  const flowEdges: DiagramEdge[] = []
  for (const edge of doc.edges) {
    const token = linkToken(edge)
    const from = byId.get(edge.from)
    const to = byId.get(edge.to)
    if (!from || !to) continue
    if (sqlIds.has(from.id) && sqlIds.has(to.id)) {
      if (token) erEdges.push(edge)
    } else if (docIds.has(from.id) && docIds.has(to.id)) {
      if (edge.stroke !== false) classEdges.push(edge)
    } else if (!token) continue
    else {
      flowEdges.push(edge)
      if (tableIds.has(from.id)) flowTable.add(from.id)
      if (tableIds.has(to.id)) flowTable.add(to.id)
    }
  }
  const groupIds = new Set(nodes.filter((node) => node.kind === 'group').map((node) => node.id))
  for (const table of tables) {
    if (table.parent && groupIds.has(table.parent)) flowTable.add(table.id)
  }
  const flowNodes = nodes.filter((node) => node.kind !== 'table' || flowTable.has(node.id))
  const heading = title?.replace(/[\r\n]/g, ' ').trim()
  const blocks: string[] = []
  if (sqlTables.length) {
    const lines = ['erDiagram']
    if (heading) lines.push(`  %% ${heading}`)
    for (const table of sqlTables) {
      lines.push(`  ${alias(table.id)} {`)
      for (const row of table.rows ?? []) lines.push(`    ${erType(row.type)} ${erIdent(row.name)}${keySuffix(row.key)}`)
      lines.push('  }')
    }
    for (const edge of erEdges) {
      const style = edgeStyle(edge)
      const line = style.dash === 'solid' ? '--' : '..'
      const label = edge.text?.replace(/[\r\n]/g, ' ').replace(/"/g, '#quot;').trim()
      const tail = label ? ` : "${label}"` : ''
      lines.push(`  ${alias(edge.from)} ${leftCard(style.start)}${line}${rightCard(style.end)} ${alias(edge.to)}${tail}`)
    }
    blocks.push(lines.join('\n'))
  }
  if (docTables.length) blocks.push(classBlock(docTables, classEdges, alias, used, sqlTables.length ? undefined : heading))
  // A diagram with no tables keeps the single flowchart block, title included, as before.
  if (flowNodes.length || flowEdges.length || tables.length === 0) {
    const lines: string[] = ['flowchart TD']
    if (heading && !tables.length) lines.push(`  %% ${heading}`)
    for (const node of flowNodes) {
      if (node.kind === 'text') lines.push(`  %% koma-text ${alias(node.id)}`)
    }
    const groups = new Set(flowNodes.filter((node) => node.kind === 'group').map((node) => node.id))
    const parentOf = (node: DiagramNode) => (node.parent && groups.has(node.parent) && node.kind !== 'group' ? node.parent : undefined)
    const kids = new Map<string, DiagramNode[]>()
    for (const node of flowNodes) {
      const parent = parentOf(node)
      if (!parent) continue
      const list = kids.get(parent) ?? []
      list.push(node)
      kids.set(parent, list)
    }
    for (const node of flowNodes) {
      if (parentOf(node)) continue
      if (node.kind !== 'group') {
        lines.push(`  ${shapeLine(node.kind, alias(node.id), node.text)}`)
        continue
      }
      lines.push(`  subgraph ${alias(node.id)} [${quote(node.text)}]`)
      for (const child of kids.get(node.id) ?? []) lines.push(`    ${shapeLine(child.kind, alias(child.id), child.text)}`)
      lines.push('  end')
    }
    for (const edge of flowEdges) {
      const token = linkToken(edge)
      if (!token) continue
      const label = edge.text?.replace(/[\r\n|]/g, ' ').replace(/"/g, '#quot;').trim()
      const marked = label ? `${token}|${label}|` : token
      lines.push(`  ${alias(edge.from)} ${marked} ${alias(edge.to)}`)
    }
    blocks.push(lines.join('\n'))
  }
  return blocks.map(fence).join('\n')
}

const EDGE_LINE = /^([A-Za-z_][A-Za-z0-9_]*)\s+(<-\.->|-\.->|<-.-|-\.-|<-->|<--|-->|---)(?:\|([^|\n]*)\|)?\s+([A-Za-z_][A-Za-z0-9_]*)\s*$/

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

const FENCE_BODY = /```[ \t]*mermaid[ \t]*\r?\n([\s\S]*?)```/gi
const TABLE_W = 220
const TABLE_GAP_X = 80
const TABLE_GAP_Y = 60

function mermaidBodies(source: string): string[] {
  const bodies: string[] = []
  for (const match of source.matchAll(new RegExp(FENCE_BODY.source, 'gi'))) {
    if (match[1] != null) bodies.push(match[1])
  }
  if (bodies.length) return bodies
  return [source.replace(/^```[ \t]*mermaid[ \t]*\r?\n?/i, '').replace(/```[\s]*$/, '')]
}

function layoutTables(nodes: DiagramNode[], freshIds: readonly string[]) {
  const fresh = freshIds.flatMap((id) => {
    const node = nodes.find((item) => item.id === id)
    return node ? [node] : []
  })
  let y = 24
  for (let start = 0; start < fresh.length; start += 3) {
    let tallest = 0
    fresh.slice(start, start + 3).forEach((node, column) => {
      node.w = TABLE_W
      node.h = tableHeight(node.rows)
      node.x = 24 + column * (TABLE_W + TABLE_GAP_X)
      node.y = y
      tallest = Math.max(tallest, node.h)
    })
    y += tallest + TABLE_GAP_Y
  }
}

function parseKey(raw: string | undefined): DiagramKey | undefined {
  if (!raw) return undefined
  const text = raw.replace(/^"|"$/g, '').trim()
  if (text === 'PK') return 'pk'
  if (text === 'FK') return 'fk'
  if (text === 'PK, FK') return 'pk,fk'
  return undefined
}

function parseAttr(line: string, index: number): DiagramRow | null {
  const match = line.match(/^(\S+)\s+("[^"]*"|\S+)(?:\s+("[^"]*"|\S+))?\s*$/)
  if (!match?.[1] || !match[2]) return null
  const name = match[2].replace(/^"|"$/g, '')
  if (!name) return null
  const row: DiagramRow = { id: `r${index}`, name, type: match[1] }
  const key = parseKey(match[3])
  if (key) row.key = key
  return row
}

const ER_REL =
  /^([A-Za-z_][A-Za-z0-9_]*)\s+(\|o|\|\||\}o|\}\|)\s*(\.\.|--)\s*(o\||\|\||o\{|\|\{)\s+([A-Za-z_][A-Za-z0-9_]*)(?:\s*:\s*(.*?))?\s*$/

function leftArrow(card: string): NonNullable<DiagramEdge['start']> {
  if (card === '|o') return 'erZeroToOne'
  if (card === '}o') return 'erZeroToMany'
  if (card === '}|') return 'erOneToMany'
  return 'erMandOne'
}

function rightArrow(card: string): NonNullable<DiagramEdge['end']> {
  if (card === 'o|') return 'erZeroToOne'
  if (card === 'o{') return 'erZeroToMany'
  if (card === '|{') return 'erOneToMany'
  return 'erMandOne'
}

/** Tells the model how to read a document classDiagram; parsing skips it like any comment. */
export const DOCUMENT_LEGEND = '%% koma documents: each class is a collection or a nested object; *-- embedded field (nested), --> reference by id'

function memberType(type: string | undefined): string {
  return (type?.trim() || 'mixed').replace(/\s+/g, '_').replace(/</g, '~').replace(/>/g, '~')
}

function memberName(name: string): string {
  return name.trim().replace(/[\s{}:]+/g, '_') || 'field'
}

/**
 * Document tables as a classDiagram: a class per collection and per nested object, nesting
 * as composition (`*--`, "many" for arrays), references as associations (`-->`).
 */
function classBlock(
  tables: DiagramNode[],
  edges: DiagramEdge[],
  alias: (id: string) => string,
  used: Set<string>,
  heading: string | undefined,
): string {
  const lines = ['classDiagram', `  ${DOCUMENT_LEGEND}`]
  if (heading) lines.push(`  %% ${heading}`)
  const relations: string[] = []
  const writeClass = (name: string, rows: DiagramRow[]) => {
    const nests: { row: DiagramRow; child: string }[] = []
    lines.push(`  class ${name} {`)
    for (const row of rows) {
      lines.push(`    +${memberType(row.type)} ${memberName(row.name)}${row.optional ? '?' : ''}${keySuffix(row.key)}`)
      if (row.children?.length) nests.push({ row, child: mermaidId(`${name}_${memberName(row.name)}`, used) })
    }
    lines.push('  }')
    for (const { row, child } of nests) {
      const many = (row.type ?? '').trim().toLowerCase().startsWith('array')
      relations.push(`  ${name} *-- "${many ? 'many' : '1'}" ${child} : ${memberName(row.name)}`)
      writeClass(child, row.children ?? [])
    }
  }
  for (const table of tables) writeClass(alias(table.id), table.rows ?? [])
  const byId = new Map(tables.map((table) => [table.id, table]))
  for (const edge of edges) {
    const from = byId.get(edge.from)
    const field = (edge.fromRow ? findRow(from?.rows, edge.fromRow)?.name : undefined) ?? edge.text?.trim()
    relations.push(`  ${alias(edge.from)} --> ${alias(edge.to)}${field ? ` : ${memberName(field)}` : ''}`)
  }
  return [...lines, ...relations].join('\n')
}

const CLASS_OPEN = /^class\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{?\s*$/
const CLASS_REL =
  /^([A-Za-z_][A-Za-z0-9_]*)\s*(?:"([^"]*)"\s*)?(\*--|o--|-->|\.\.>|--)\s*(?:"([^"]*)"\s*)?([A-Za-z_][A-Za-z0-9_]*)\s*(?::\s*(.*?))?\s*$/

/** `+type name? PK` (also `name: type`). Generics come back from Mermaid's `~` form. */
function parseMember(line: string, mint: () => string): DiagramRow | null {
  const text = line.replace(/^[+\-#~]\s*/, '').trim()
  if (!text || text.includes('(')) return null
  const colon = text.match(/^([^\s:?]+)(\?)?\s*:\s*(\S+)/)
  const plain = text.match(/^(\S+)\s+([^\s?]+)(\?)?(?:\s+(.*))?$/)
  const raw = colon ? { type: colon[3], name: colon[1], optional: !!colon[2], rest: '' } : plain ? { type: plain[1], name: plain[2], optional: !!plain[3], rest: plain[4] ?? '' } : null
  if (!raw?.type || !raw.name) return null
  const type = raw.type.replace(/~([^~]+)~/, '<$1>')
  const row: DiagramRow = { id: mint(), name: raw.name, type }
  if (raw.optional) row.optional = true
  const key = parseKey(raw.rest.replace(/"/g, '').trim() || undefined)
  if (key) row.key = key
  return row
}

/** A classDiagram from a document diagram (or a model) becomes document tables, nests and references. */
function parseClass(doc: DiagramDoc, body: string, title: string): string {
  const classes = new Map<string, DiagramRow[]>()
  const order: string[] = []
  const nests: { parent: string; child: string; field: string; many: boolean }[] = []
  const refs: { from: string; to: string; field: string }[] = []
  let current: string | null = null
  let rowSeq = 0
  const mint = () => `r${++rowSeq}`
  const ensure = (name: string) => {
    if (!classes.has(name)) {
      classes.set(name, [])
      order.push(name)
    }
  }
  for (const raw of body.split(/\r?\n/)) {
    const line = raw.trim()
    if (!line || line === 'classDiagram') continue
    if (line.startsWith('%%')) {
      const comment = line.replace(/^%%\s*/, '')
      if (title === 'Diagram' && comment && !comment.startsWith('koma ')) title = comment
      continue
    }
    if (current && line === '}') {
      current = null
      continue
    }
    const open = line.match(CLASS_OPEN)
    if (open?.[1]) {
      ensure(open[1])
      current = line.endsWith('{') ? open[1] : null
      continue
    }
    if (current) {
      const row = parseMember(line, mint)
      if (row) classes.get(current)?.push(row)
      continue
    }
    const rel = line.match(CLASS_REL)
    if (!rel?.[1] || !rel[3] || !rel[5]) continue
    ensure(rel[1])
    ensure(rel[5])
    const field = rel[6]?.trim() ?? ''
    if (rel[3] === '*--' || rel[3] === 'o--') {
      const card = (rel[4] ?? '').toLowerCase()
      nests.push({ parent: rel[1], child: rel[5], field: field || rel[5], many: /many|\*|\.\.|^n$/.test(card) })
    } else refs.push({ from: rel[1], to: rel[5], field })
  }
  const nested = new Set(nests.map((nest) => nest.child))
  const build = (name: string, seen: Set<string>): DiagramRow[] => {
    const rows = (classes.get(name) ?? []).map((row) => ({ ...row }))
    for (const nest of nests.filter((item) => item.parent === name)) {
      if (seen.has(nest.child)) continue
      const children = build(nest.child, new Set([...seen, nest.child]))
      let row = rows.find((item) => item.name === nest.field)
      if (!row) {
        row = { id: mint(), name: nest.field }
        rows.push(row)
      }
      row.type = nest.many ? 'array<object>' : 'object'
      if (children.length) row.children = children
    }
    return rows
  }
  const rootOf = (name: string): string => {
    const parent = nests.find((nest) => nest.child === name)?.parent
    return parent && parent !== name ? rootOf(parent) : name
  }
  const fresh: string[] = []
  for (const name of order) {
    if (nested.has(name) || doc.nodes.some((node) => node.id === name)) continue
    const rows = build(name, new Set([name]))
    doc.nodes.push({ id: name, kind: 'table', dialect: 'document', x: 24, y: 24, w: TABLE_W, h: tableHeight(rows), text: name, rows })
    fresh.push(name)
  }
  for (const ref of refs) {
    const from = rootOf(ref.from)
    const to = rootOf(ref.to)
    const table = doc.nodes.find((node) => node.id === from)
    const fromRow = ref.field ? allRows(table?.rows).find((row) => row.name === ref.field)?.id : undefined
    if (!table || !doc.nodes.some((node) => node.id === to)) continue
    doc.edges.push({ id: `e${doc.edges.length + 1}`, from, to, ...(fromRow ? { fromRow } : ref.field ? { text: ref.field } : {}) })
  }
  layoutTables(doc.nodes, fresh)
  return title
}

/** `erDiagram` entities become tables on a grid. A later flowchart must not overwrite them. */
function parseEr(doc: DiagramDoc, body: string, title: string): string {
  let current: DiagramNode | null = null
  const fresh: string[] = []
  const ensure = (id: string): DiagramNode => {
    const found = doc.nodes.find((node) => node.id === id)
    if (found) {
      if (found.kind !== 'table') {
        found.kind = 'table'
        found.rows = []
        found.h = tableHeight([])
        delete found.rotation
        fresh.push(id)
      }
      return found
    }
    const node: DiagramNode = { id, kind: 'table', x: 24, y: 24, w: TABLE_W, h: tableHeight([]), text: id, rows: [] }
    doc.nodes.push(node)
    fresh.push(id)
    return node
  }
  for (const raw of body.split(/\r?\n/)) {
    const line = raw.trim()
    if (!line || line === 'erDiagram') continue
    if (line.startsWith('%% ')) {
      if (title === 'Diagram') title = line.slice(3).trim() || title
      continue
    }
    if (current && line === '}') {
      current = null
      continue
    }
    const open = line.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*\{$/)
    if (open?.[1]) {
      current = ensure(open[1])
      current.text = open[1]
      continue
    }
    if (current) {
      const row = parseAttr(line, (current.rows?.length ?? 0) + 1)
      if (!row) continue
      current.rows = [...(current.rows ?? []), row]
      current.h = tableHeight(current.rows)
      continue
    }
    const rel = line.match(ER_REL)
    if (!rel?.[1] || !rel[2] || !rel[3] || !rel[4] || !rel[5]) continue
    ensure(rel[1])
    ensure(rel[5])
    const quoted = rel[6]?.trim() ?? ''
    const label = quoted.startsWith('"') && quoted.endsWith('"') ? unquote(quoted.slice(1, -1)) : unquote(quoted)
    doc.edges.push({
      id: `e${doc.edges.length + 1}`,
      from: rel[1],
      to: rel[5],
      route: 'entity',
      start: leftArrow(rel[2]),
      end: rightArrow(rel[4]),
      ...(rel[3] === '..' ? { dash: 'dashed' as const } : {}),
      ...(label ? { text: label } : {}),
    })
  }
  layoutTables(doc.nodes, fresh)
  return title
}

function fitGroups(doc: DiagramDoc): void {
  const pad = 24
  for (const group of doc.nodes) {
    if (group.kind !== 'group') continue
    const kids = doc.nodes.filter((node) => node.parent === group.id)
    if (!kids.length) {
      group.w = 280
      group.h = 180
      continue
    }
    let minX = Infinity
    let minY = Infinity
    let maxX = -Infinity
    let maxY = -Infinity
    for (const child of kids) {
      minX = Math.min(minX, child.x)
      minY = Math.min(minY, child.y)
      maxX = Math.max(maxX, child.x + child.w)
      maxY = Math.max(maxY, child.y + child.h)
    }
    group.x = minX - pad
    group.y = minY - GROUP_TITLE - pad
    group.w = Math.max(120, maxX - minX + pad * 2)
    group.h = Math.max(80, maxY - minY + pad * 2 + GROUP_TITLE)
  }
}

function parseFlow(doc: DiagramDoc, body: string, title: string): string {
  const textIds = new Set<string>()
  const known = new Set(doc.nodes.map((node) => node.id))
  // Keep flowchart shapes off the table grid when both blocks arrived together.
  const flowX = doc.nodes.some((node) => node.kind === 'table') ? 24 + 3 * (TABLE_W + TABLE_GAP_X) : 24
  let depth = 0
  let groupId: string | null = null
  const nest = (id: string) => {
    const found = doc.nodes.find((node) => node.id === id)
    if (found && groupId && found.kind !== 'group') found.parent = groupId
  }
  const addGroup = (id: string, text: string) => {
    const found = doc.nodes.find((node) => node.id === id)
    if (found) {
      if (found.kind !== 'table') {
        found.kind = 'group'
        found.text = text
        delete found.parent
      }
      known.add(id)
      return
    }
    known.add(id)
    doc.nodes.push({ id, kind: 'group', x: flowX, y: 24, w: 280, h: 180, text })
  }
  const ensure = (id: string) => {
    if (known.has(id)) {
      nest(id)
      return
    }
    known.add(id)
    const index = doc.nodes.length
    const node: DiagramNode = { id, kind: 'rect', x: flowX, y: 24 + index * 88, w: 160, h: 48, text: id }
    if (groupId) node.parent = groupId
    doc.nodes.push(node)
  }
  for (const raw of body.split(/\r?\n/)) {
    const line = raw.trim()
    if (line === 'end') {
      if (depth > 0) depth -= 1
      if (depth === 0) groupId = null
      continue
    }
    const subgraph = line.match(/^subgraph\s+([A-Za-z_][A-Za-z0-9_]*)(?:\s*\[(.*)\])?\s*$/)
    if (subgraph?.[1]) {
      depth += 1
      if (depth === 1) {
        groupId = subgraph[1]
        addGroup(subgraph[1], labelText(subgraph[2] ?? subgraph[1]))
      }
      continue
    }
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
    const node = parseNodeLine(line)
    if (!node) continue
    ensure(node.id)
    const found = doc.nodes.find((item) => item.id === node.id)
    // A table parsed from erDiagram keeps its rows. The flowchart only borrows the name.
    if (!found || found.kind === 'table' || found.kind === 'group') continue
    found.kind = node.kind
    found.text = node.label
  }
  for (const node of doc.nodes) {
    if (textIds.has(node.id) && node.kind !== 'table') node.kind = 'text'
  }
  return title
}

export function parseMermaidDiagram(mermaid: string): { doc: DiagramDoc; title: string } {
  const bodies = mermaidBodies(mermaid)
  const doc = emptyDiagram()
  let title = 'Diagram'
  // Tables first, so a flowchart node with the same id cannot replace one.
  for (const body of bodies) {
    if (/^\s*erDiagram\b/m.test(body)) title = parseEr(doc, body, title)
  }
  for (const body of bodies) {
    if (/^\s*classDiagram\b/m.test(body)) title = parseClass(doc, body, title)
  }
  for (const body of bodies) {
    if (!/^\s*(?:erDiagram|classDiagram)\b/m.test(body)) title = parseFlow(doc, body, title)
  }
  fitGroups(doc)
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
