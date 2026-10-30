// Pasting `CREATE TABLE` statements builds tables, like draw.io's SQL import, and also draws
// the foreign keys as entity relation lines between the rows (draw.io stops at the tables).
import { ER_LINE, TABLE_ROW_LIMIT, tableHeight, type DiagramEdge, type DiagramKey, type DiagramNode, type DiagramPoint, type DiagramRow } from './diagram'

export type SqlColumn = { name: string; type?: string; pk: boolean; fk: boolean }
export type SqlTable = { name: string; columns: SqlColumn[] }
/** `table.column` references `refTable.refColumn` (the referenced column may be implied). */
export type SqlRelation = { table: string; column: string; refTable: string; refColumn?: string }

export function looksLikeSql(text: string): boolean {
  return /\bcreate\s+table\b/i.test(stripComments(text))
}

function stripComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, ' ').replace(/--[^\n]*/g, ' ')
}

/** `"public"."users"`, `` `users` ``, `[users]` → `users`. The schema is dropped. */
function identifier(raw: string): string {
  const parts = raw.trim().split('.')
  return (parts[parts.length - 1] ?? '').replace(/^["`[]|["`\]]$/g, '').trim()
}

function identifiers(list: string): string[] {
  return list.split(',').map(identifier).filter(Boolean)
}

/** Split on commas that are not inside parentheses or quotes. */
function splitTopLevel(body: string): string[] {
  const parts: string[] = []
  let depth = 0
  let quote: string | null = null
  let start = 0
  for (let i = 0; i < body.length; i++) {
    const ch = body[i]
    if (quote) {
      if (ch === quote) quote = null
      continue
    }
    if (ch === "'" || ch === '"' || ch === '`') quote = ch
    else if (ch === '(') depth++
    else if (ch === ')') depth--
    else if (ch === ',' && depth === 0) {
      parts.push(body.slice(start, i))
      start = i + 1
    }
  }
  parts.push(body.slice(start))
  return parts.map((part) => part.trim()).filter(Boolean)
}

/** Words that end a column's type in a definition like `email varchar(255) NOT NULL UNIQUE`. */
const TYPE_END = /^(not|null|default|primary|references|unique|check|auto_increment|autoincrement|identity|generated|constraint|collate|on|comment|key)$/i

const REFERENCES = /\breferences\s+([^\s(]+)\s*(?:\(([^)]*)\))?/i

export function parseSqlTables(text: string): { tables: SqlTable[]; relations: SqlRelation[] } {
  const source = stripComments(text)
  const tables: SqlTable[] = []
  const relations: SqlRelation[] = []
  const head = /\bcreate\s+(?:temporary\s+|temp\s+)?table\s+(?:if\s+not\s+exists\s+)?([^\s(]+)\s*\(/gi
  for (let match = head.exec(source); match; match = head.exec(source)) {
    // Find the parenthesis that closes the column list.
    let depth = 1
    let end = head.lastIndex
    for (; end < source.length && depth > 0; end++) {
      if (source[end] === '(') depth++
      else if (source[end] === ')') depth--
    }
    const table: SqlTable = { name: identifier(match[1] ?? ''), columns: [] }
    const keys = new Set<string>()
    const foreign = new Set<string>()
    for (let part of splitTopLevel(source.slice(head.lastIndex, end - 1))) {
      part = part.replace(/^constraint\s+\S+\s+/i, '')
      const primary = part.match(/^primary\s+key\s*\(([^)]*)\)/i)
      if (primary) {
        for (const name of identifiers(primary[1] ?? '')) keys.add(name.toLowerCase())
        continue
      }
      const fk = part.match(/^foreign\s+key\s*\(([^)]*)\)/i)
      if (fk) {
        const columns = identifiers(fk[1] ?? '')
        const ref = part.match(REFERENCES)
        const refColumns = ref?.[2] ? identifiers(ref[2]) : []
        columns.forEach((column, i) => {
          foreign.add(column.toLowerCase())
          if (ref?.[1]) relations.push({ table: table.name, column, refTable: identifier(ref[1]), refColumn: refColumns[i] })
        })
        continue
      }
      if (/^(unique|check|index|key|fulltext|spatial|exclude)\b/i.test(part)) continue
      const tokens = part.match(/"[^"]*"|`[^`]*`|\[[^\]]*\]|\S+\([^)]*\)|\S+/g) ?? []
      const name = identifier(tokens[0] ?? '')
      if (!name) continue
      const typeWords: string[] = []
      for (const token of tokens.slice(1)) {
        if (TYPE_END.test(token)) break
        typeWords.push(token)
      }
      const column: SqlColumn = { name, pk: /\bprimary\s+key\b/i.test(part), fk: false }
      if (typeWords.length) column.type = typeWords.join(' ')
      const ref = part.match(REFERENCES)
      if (ref?.[1]) {
        column.fk = true
        relations.push({ table: table.name, column: name, refTable: identifier(ref[1]), refColumn: ref[2] ? identifiers(ref[2])[0] : undefined })
      }
      table.columns.push(column)
    }
    for (const column of table.columns) {
      if (keys.has(column.name.toLowerCase())) column.pk = true
      if (foreign.has(column.name.toLowerCase())) column.fk = true
    }
    if (table.name) tables.push(table)
    head.lastIndex = end
  }
  return { tables, relations }
}

function keyOf(column: SqlColumn): DiagramKey | undefined {
  return column.pk && column.fk ? 'pk,fk' : column.pk ? 'pk' : column.fk ? 'fk' : undefined
}

/**
 * Tables in a grid of up to three columns starting at `origin`, and one ER line per
 * foreign key, from the referenced row (one) to the referencing row (many).
 */
export function sqlToDiagram(
  parsed: { tables: SqlTable[]; relations: SqlRelation[] },
  origin: DiagramPoint,
  mint: (prefix: string) => string,
): { nodes: DiagramNode[]; edges: DiagramEdge[] } {
  const width = 220
  const gapX = 80
  const gapY = 60
  const perRow = Math.min(3, Math.max(1, parsed.tables.length))
  const nodes: DiagramNode[] = []
  const byName = new Map<string, DiagramNode>()
  let y = origin.y
  for (let start = 0; start < parsed.tables.length; start += perRow) {
    let tallest = 0
    parsed.tables.slice(start, start + perRow).forEach((table, column) => {
      const rows: DiagramRow[] = table.columns.slice(0, TABLE_ROW_LIMIT).map((col, i) => {
        const row: DiagramRow = { id: `r${i + 1}`, name: col.name }
        if (col.type) row.type = col.type
        const key = keyOf(col)
        if (key) row.key = key
        return row
      })
      const h = tableHeight(rows)
      tallest = Math.max(tallest, h)
      const node: DiagramNode = { id: mint('n'), kind: 'table', x: origin.x + column * (width + gapX), y, w: width, h, text: table.name, rows }
      nodes.push(node)
      byName.set(table.name.toLowerCase(), node)
    })
    y += tallest + gapY
  }
  const rowFor = (node: DiagramNode, column: string | undefined) => {
    const rows = node.rows ?? []
    const named = column ? rows.find((row) => row.name.toLowerCase() === column.toLowerCase()) : undefined
    return (named ?? rows.find((row) => row.key?.includes('pk')))?.id
  }
  const edges: DiagramEdge[] = []
  for (const relation of parsed.relations) {
    const child = byName.get(relation.table.toLowerCase())
    const parent = byName.get(relation.refTable.toLowerCase())
    if (!child || !parent) continue
    const edge: DiagramEdge = { id: mint('e'), from: parent.id, to: child.id, ...ER_LINE }
    const fromRow = rowFor(parent, relation.refColumn)
    const toRow = rowFor(child, relation.column)
    if (fromRow) edge.fromRow = fromRow
    if (toRow) edge.toRow = toRow
    edges.push(edge)
  }
  return { nodes, edges }
}
