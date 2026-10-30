// JSON ↔ document tables. Pasting sample documents (a MongoDB export, an API response)
// builds a document table whose nesting follows the JSON; a table copies back out as a
// sample document or as a MongoDB `$jsonSchema` validator.
import { TABLE_ROW_LIMIT, isNestType, tableHeight, type DiagramNode, type DiagramPoint, type DiagramRow } from './diagram'

type Kind = 'objectId' | 'date' | 'int' | 'number' | 'bool' | 'null' | 'string' | 'object' | 'array'

/** What we learn about one field across every document that was pasted. */
type FieldStats = { present: number; kinds: Set<Kind>; object?: DocStats; items?: FieldStats }
type DocStats = { docs: number; fields: Map<string, FieldStats> }

const ISO_DATE = /^\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2})?)?$/
const OBJECT_ID = /^[0-9a-f]{24}$/i

export function looksLikeJson(text: string): boolean {
  const trimmed = text.trim()
  if (!(trimmed.startsWith('{') || trimmed.startsWith('['))) return false
  try {
    const value: unknown = JSON.parse(trimmed)
    return typeof value === 'object' && value !== null
  } catch {
    return false
  }
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** The kind of one JSON value, reading MongoDB Extended JSON wrappers (`$oid`, `$date`, …). */
function kindOf(value: unknown): Kind {
  if (value === null || value === undefined) return 'null'
  if (typeof value === 'boolean') return 'bool'
  if (typeof value === 'number') return Number.isInteger(value) ? 'int' : 'number'
  if (typeof value === 'string') return OBJECT_ID.test(value) ? 'objectId' : ISO_DATE.test(value) ? 'date' : 'string'
  if (Array.isArray(value)) return 'array'
  if (isPlainObject(value)) {
    const keys = Object.keys(value)
    if (keys.length === 1) {
      if (keys[0] === '$oid') return 'objectId'
      if (keys[0] === '$date') return 'date'
      if (keys[0] === '$numberInt' || keys[0] === '$numberLong') return 'int'
      if (keys[0] === '$numberDouble' || keys[0] === '$numberDecimal') return 'number'
    }
    return 'object'
  }
  return 'string'
}

function newDocStats(): DocStats {
  return { docs: 0, fields: new Map() }
}

function addValue(field: FieldStats, value: unknown): void {
  const kind = kindOf(value)
  field.kinds.add(kind)
  if (kind === 'object') {
    field.object ??= newDocStats()
    addDoc(field.object, value as Record<string, unknown>)
  } else if (kind === 'array') {
    field.items ??= { present: 0, kinds: new Set() }
    for (const item of value as unknown[]) {
      field.items.present += 1
      addValue(field.items, item)
    }
  }
}

function addDoc(stats: DocStats, doc: Record<string, unknown>): void {
  stats.docs += 1
  for (const [key, value] of Object.entries(doc)) {
    let field = stats.fields.get(key)
    if (!field) {
      field = { present: 0, kinds: new Set() }
      stats.fields.set(key, field)
    }
    field.present += 1
    addValue(field, value)
  }
}

/** One type name for everything seen in a field. Nulls alone make a field `null`; mixed with others they drop out. */
function typeName(field: FieldStats): string {
  const kinds = new Set(field.kinds)
  if (kinds.size > 1) kinds.delete('null')
  if (kinds.size === 2 && kinds.has('int') && kinds.has('number')) return 'number'
  if (kinds.size !== 1) return 'mixed'
  const only = [...kinds][0] as Kind
  if (only !== 'array') return only
  const items = field.items
  if (!items || !items.kinds.size) return 'array<mixed>'
  const inner = typeName(items)
  return inner === 'mixed' ? 'array<mixed>' : `array<${inner}>`
}

function statsToRows(stats: DocStats, mint: (prefix: string) => string, budget: { left: number }): DiagramRow[] {
  const rows: DiagramRow[] = []
  for (const [name, field] of stats.fields) {
    if (budget.left <= 0) break
    budget.left -= 1
    const row: DiagramRow = { id: mint('r'), name, type: typeName(field) }
    if (name === '_id') row.key = 'pk'
    if (field.present < stats.docs) row.optional = true
    // An object, or an array whose elements are objects, holds the merged shape of its fields.
    const nested = row.type === 'object' ? field.object : field.items?.kinds.has('object') ? field.items.object : undefined
    if (nested) {
      const children = statsToRows(nested, mint, budget)
      if (children.length) row.children = children
    }
    rows.push(row)
  }
  return rows
}

/**
 * A document table from pasted JSON: one document, an array of documents, or a single key
 * holding them (`{ "orders": [...] }`, which also names the table). Fields missing from some
 * documents become optional. Null when there is no object to learn from.
 */
export function jsonToTable(value: unknown, at: DiagramPoint, mint: (prefix: string) => string): DiagramNode | null {
  let name = 'collection'
  let docs: unknown[] = []
  if (Array.isArray(value)) docs = value
  else if (isPlainObject(value)) {
    const keys = Object.keys(value)
    const only = keys.length === 1 ? value[keys[0] as string] : undefined
    if (keys.length === 1 && (Array.isArray(only) || isPlainObject(only))) {
      name = keys[0] as string
      docs = Array.isArray(only) ? only : [only]
    } else docs = [value]
  }
  const stats = newDocStats()
  for (const doc of docs) if (isPlainObject(doc)) addDoc(stats, doc)
  if (!stats.docs || !stats.fields.size) return null
  const rows = statsToRows(stats, mint, { left: TABLE_ROW_LIMIT })
  const h = tableHeight(rows)
  const w = 240
  return { id: mint('n'), kind: 'table', dialect: 'document', x: at.x - w / 2, y: at.y - h / 2, w, h, text: name, rows }
}

const SAMPLE_OID = '000000000000000000000000'

function sampleOf(type: string | undefined, children: DiagramRow[] | undefined): unknown {
  const t = (type ?? '').trim().toLowerCase()
  if (children?.length && t !== 'array' && !t.startsWith('array')) return sampleDoc(children)
  if (t.startsWith('array')) {
    if (children?.length) return [sampleDoc(children)]
    const inner = t.match(/^array<(.+)>$/)?.[1]
    return inner && inner !== 'mixed' ? [sampleOf(inner, undefined)] : []
  }
  if (t === 'string') return ''
  if (t === 'number' || t === 'int' || t === 'double' || t === 'long') return 0
  if (t === 'bool' || t === 'boolean') return false
  if (t === 'date') return { $date: '1970-01-01T00:00:00Z' }
  if (t === 'objectid') return { $oid: SAMPLE_OID }
  if (t === 'object') return {}
  return null
}

function sampleDoc(rows: DiagramRow[]): Record<string, unknown> {
  const doc: Record<string, unknown> = {}
  for (const row of rows) if (row.name) doc[row.name] = sampleOf(row.type, row.children)
  return doc
}

/** One example document, with typed placeholders, shaped like the table. */
export function tableToJsonSample(node: DiagramNode): string {
  return JSON.stringify(sampleDoc(node.rows ?? []), null, 2)
}

const BSON: Record<string, string> = {
  string: 'string',
  number: 'double',
  double: 'double',
  int: 'int',
  long: 'long',
  bool: 'bool',
  boolean: 'bool',
  date: 'date',
  objectid: 'objectId',
  null: 'null',
}

function schemaOf(type: string | undefined, children: DiagramRow[] | undefined): Record<string, unknown> {
  const t = (type ?? '').trim().toLowerCase()
  if (t.startsWith('array')) {
    const inner = t.match(/^array<(.+)>$/)?.[1]
    const items = children?.length ? objectSchema(children) : inner && inner !== 'mixed' ? schemaOf(inner, undefined) : undefined
    return items ? { bsonType: 'array', items } : { bsonType: 'array' }
  }
  if (children?.length || isNestType(t)) return objectSchema(children ?? [])
  const bson = BSON[t]
  return bson ? { bsonType: bson } : {}
}

function objectSchema(rows: DiagramRow[]): Record<string, unknown> {
  const properties: Record<string, unknown> = {}
  const required: string[] = []
  for (const row of rows) {
    if (!row.name) continue
    properties[row.name] = schemaOf(row.type, row.children)
    if (!row.optional) required.push(row.name)
  }
  return required.length ? { bsonType: 'object', required, properties } : { bsonType: 'object', properties }
}

/** A MongoDB `$jsonSchema` validator for the table; optional fields are left out of `required`. */
export function tableToJsonSchema(node: DiagramNode): string {
  return JSON.stringify({ $jsonSchema: objectSchema(node.rows ?? []) }, null, 2)
}
