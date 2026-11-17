import type { Transport } from '../../../types/config'
import { blankDraft, linesToList, linesToPairs, secToMs, splitList, type McpDraft } from './mcpForm'

// JSON view of an MCP server. It mirrors the host parser (`parse_servers_json` in
// src-agent/src/app/mcp/project.rs) so a snippet that validates here is accepted there:
// `{ "mcpServers": { name: {..} } }` (Claude Code / Cursor), `{ "servers": {..} }` (VS Code),
// a bare `{ name: {..} }` map, or a single server object.

type Json = Record<string, unknown>

const isObj = (v: unknown): v is Json => typeof v === 'object' && v !== null && !Array.isArray(v)

function pairsToObject(pairs: [string, string][]): Json {
  const o: Json = {}
  for (const [k, v] of pairs) o[k] = v
  return o
}

/** The JSON object for one draft: only the fields that apply to its transport. Header values
 *  are written exactly as the draft holds them, so a masked value stays masked. */
export function draftToEntry(d: McpDraft): Json {
  const o: Json = {}
  if (d.transport === 'http') {
    o.type = 'http'
    o.url = d.url
    const headers = linesToPairs(d.headersText, ':')
    if (headers.length) o.headers = pairsToObject(headers)
  } else {
    o.command = d.command
    const args = linesToList(d.argsText)
    if (args.length) o.args = args
    const env = linesToPairs(d.envText, '=')
    if (env.length) o.env = pairsToObject(env)
    if (d.cwd.trim()) o.cwd = d.cwd.trim()
  }
  if (!d.enabled) o.disabled = true
  const startup = secToMs(d.startupSec)
  if (startup > 0) o.startupTimeoutMs = startup
  const tool = secToMs(d.toolSec)
  if (tool > 0) o.toolTimeoutMs = tool
  const allowed = splitList(d.allowedText)
  if (allowed.length) o.allowedTools = allowed
  const disabled = splitList(d.disabledText)
  if (disabled.length) o.disabledTools = disabled
  return o
}

/** The text of the JSON view: `{ "name": { ...fields } }`, or the bare entry while the draft has
 *  no name yet (the key is then required when saving). */
export function draftToJson(d: McpDraft): string {
  const entry = draftToEntry(d)
  const name = d.name.trim()
  return JSON.stringify(name ? { [name]: entry } : entry, null, 2)
}

export type JsonDraftResult =
  | { ok: true; draft: McpDraft; warnings: string[] }
  | { ok: false; error: string }

const fail = (error: string): { ok: false; error: string } => ({ ok: false, error })

/** Same tolerance as the host: strict JSON first, then a README fragment `"name": {..}`. */
function parseLoose(text: string): { ok: true; value: unknown } | { ok: false; error: string } {
  const trimmed = text.trim()
  if (!trimmed) return fail('nothing to import')
  try {
    return { ok: true, value: JSON.parse(trimmed) }
  } catch (e) {
    const first = e instanceof Error ? e.message : String(e)
    try {
      return { ok: true, value: JSON.parse(`{${trimmed.replace(/,+$/, '')}}`) }
    } catch {
      return fail(`not valid JSON: ${first}`)
    }
  }
}

const isSingle = (v: unknown) => typeof v === 'string'

/** Name -> entry pairs of any accepted shape. A single server object has no name. */
function namedEntries(root: Json): { name: string | null; raw: unknown }[] {
  const wrapped = isObj(root.mcpServers) ? root.mcpServers : isObj(root.servers) ? root.servers : null
  if (wrapped) return Object.entries(wrapped).map(([name, raw]) => ({ name, raw }))
  if (isSingle(root.command) || isSingle(root.url)) return [{ name: null, raw: root }]
  return Object.entries(root).map(([name, raw]) => ({ name, raw }))
}

function strField(o: Json, key: string): string | null | { error: string } {
  const v = o[key]
  if (v === undefined || v === null) return null
  return typeof v === 'string' ? v : { error: `"${key}" must be a string` }
}

function mapField(o: Json, key: string, lineBased: boolean): [string, string][] | { error: string } {
  const v = o[key]
  if (v === undefined || v === null) return []
  if (!isObj(v)) return { error: `"${key}" must be an object of strings` }
  const out: [string, string][] = []
  for (const [k, val] of Object.entries(v)) {
    if (typeof val !== 'string') return { error: `"${key}.${k}" must be a string` }
    if (lineBased && (/[\r\n]/.test(k) || /[\r\n]/.test(val))) return { error: `"${key}.${k}" cannot contain a line break` }
    out.push([k, val])
  }
  return out
}

function listField(o: Json, ...keys: string[]): string[] | { error: string } {
  const key = keys.find((k) => o[k] !== undefined && o[k] !== null) ?? keys[0]
  const v = o[key]
  if (v === undefined || v === null) return []
  if (!Array.isArray(v) || v.some((x) => typeof x !== 'string')) return { error: `"${key}" must be a list of strings` }
  return v as string[]
}

function msField(o: Json, key: string): string | { error: string } {
  const v = o[key]
  if (v === undefined || v === null) return ''
  if (typeof v !== 'number' || !Number.isFinite(v)) return { error: `"${key}" must be a number of milliseconds` }
  return v > 0 ? String(v / 1000) : ''
}

const bad = (v: unknown): v is { error: string } => typeof v === 'object' && v !== null && !Array.isArray(v) && 'error' in v

/** One entry object onto a draft. Fields that do not apply to the chosen transport keep the
 *  base draft's values, like the form does when you flip the transport. */
function entryToDraft(name: string, raw: unknown, base: McpDraft, lineBased = true): JsonDraftResult {
  const label = name ? `'${name}': ` : ''
  if (!isObj(raw)) return fail(`${label}a server must be a JSON object`)
  const warnings: string[] = []
  const command = strField(raw, 'command')
  const url = strField(raw, 'url')
  const cwd = strField(raw, 'cwd')
  const kind = strField(raw, 'type')
  for (const v of [command, url, cwd, kind]) if (bad(v)) return fail(label + v.error)
  const args = listField(raw, 'args')
  const allowed = listField(raw, 'allowedTools', 'enabledTools')
  const disabledTools = listField(raw, 'disabledTools')
  const env = mapField(raw, 'env', lineBased)
  const headers = mapField(raw, 'headers', lineBased)
  const startup = msField(raw, 'startupTimeoutMs')
  const tool = msField(raw, 'toolTimeoutMs')
  for (const v of [args, allowed, disabledTools, env, headers, startup, tool]) if (bad(v)) return fail(label + v.error)
  if (lineBased && (args as string[]).some((a) => /[\r\n]/.test(a))) return fail(`${label}"args" cannot contain a line break`)

  const cmd = ((command as string | null) ?? '').trim()
  const link = ((url as string | null) ?? '').trim()
  const k = ((kind as string | null) ?? '').trim().toLowerCase().replace(/[-_]/g, '')
  let transport: Transport
  if (k === '') transport = link && !cmd ? 'http' : 'stdio'
  else if (k === 'stdio') transport = 'stdio'
  else if (k === 'http' || k === 'streamablehttp') transport = 'http'
  else if (k === 'sse') {
    transport = 'http'
    warnings.push(
      'Legacy SSE is not supported; this will connect as streamable HTTP (often at a URL ending in /mcp).',
    )
  } else return fail(`${label}unknown transport '${kind}'`)

  const stdio = transport === 'stdio'
  const draft: McpDraft = {
    ...base,
    name,
    enabled: raw.disabled !== true && raw.enabled !== false,
    transport,
    command: stdio ? cmd : base.command,
    argsText: stdio ? (args as string[]).join('\n') : base.argsText,
    envText: stdio ? (env as [string, string][]).map(([a, b]) => `${a}=${b}`).join('\n') : base.envText,
    url: stdio ? base.url : link,
    headersText: stdio ? base.headersText : (headers as [string, string][]).map(([a, b]) => `${a}: ${b}`).join('\n'),
    cwd: stdio ? ((cwd as string | null) ?? '').trim() : base.cwd,
    startupSec: startup as string,
    toolSec: tool as string,
    allowedText: (allowed as string[]).join(', '),
    disabledText: (disabledTools as string[]).join(', '),
  }
  return { ok: true, draft, warnings }
}

/** The JSON view back into a draft. Exactly one server is allowed; its key is the name (a bare
 *  entry keeps the draft's name). The draft id and anything JSON cannot carry come from `base`. */
export function jsonToDraft(text: string, base: McpDraft): JsonDraftResult {
  const parsed = parseLoose(text)
  if (!parsed.ok) return parsed
  if (!isObj(parsed.value)) return fail('expected a JSON object')
  const entries = namedEntries(parsed.value)
  if (entries.length === 0) return fail('no server found')
  if (entries.length > 1) return fail(`this tab edits one server but the JSON holds ${entries.length}; use Import to add several`)
  const { name, raw } = entries[0]
  const resolved = (name ?? base.name).trim()
  if (!resolved) return fail('a server needs a name: write it as { "name": { ... } }')
  return entryToDraft(resolved, raw, base)
}

export type ImportCheck = { ok: true; names: string[]; warnings: string[] } | { ok: false; error: string }

/** Pre-flight for the import page, mirroring the host: every entry must decode, and at least
 *  one must be importable. The host remains the authority on what actually gets added. */
export function checkImportJson(text: string): ImportCheck {
  const parsed = parseLoose(text)
  if (!parsed.ok) return parsed
  if (!isObj(parsed.value)) return fail('expected a JSON object')
  const entries = namedEntries(parsed.value)
  if (entries.some((e) => e.name === null)) {
    return fail('a single server needs a name; wrap it as {"name": {...}}')
  }
  const names: string[] = []
  const warnings: string[] = []
  const errors: string[] = []
  for (const { name, raw } of entries) {
    const n = (name ?? '').trim()
    if (!n) {
      errors.push('empty server name')
      continue
    }
    const r = entryToDraft(n, raw, blankDraft(), false)
    if (!r.ok) errors.push(r.error)
    else if (r.draft.transport === 'stdio' && !r.draft.command.trim()) warnings.push(`'${n}': no command; skipped`)
    else if (r.draft.transport === 'http' && !r.draft.url.trim()) warnings.push(`'${n}': no url; skipped`)
    else {
      names.push(n)
      warnings.push(...r.warnings.map((w) => `'${n}': ${w}`))
    }
  }
  if (names.length === 0) return fail(`no server could be imported: ${errors[0] ?? warnings[0] ?? 'no servers found'}`)
  return { ok: true, names, warnings: [...errors, ...warnings] }
}

const str = { type: 'string' }
const strMap = { type: 'object', additionalProperties: str }
const strList = { type: 'array', items: str }

const timing = {
  startupTimeoutMs: { type: 'number', minimum: 0, description: 'Startup timeout in milliseconds.' },
  toolTimeoutMs: { type: 'number', minimum: 0, description: 'Tool call timeout in milliseconds.' },
  allowedTools: { ...strList, description: 'Only these tools are offered; * matches any text.' },
  disabledTools: { ...strList, description: 'These tools are hidden; wins over allowedTools.' },
  disabled: { type: 'boolean', description: 'true keeps the server saved but unused.' },
  enabled: { type: 'boolean' },
}

const stdioServer = {
  type: 'object',
  description: 'A server started as a local command.',
  properties: {
    type: { type: 'string', examples: ['stdio'], description: 'stdio (default when there is a command).' },
    command: { ...str, description: 'Program to run, e.g. npx.' },
    args: { ...strList, description: 'Arguments, one list item each.' },
    env: { ...strMap, description: 'Environment variables. ${VAR} reads from the environment.' },
    cwd: { ...str, description: 'Working directory; empty uses the session folder.' },
    ...timing,
  },
  required: ['command'],
}

const httpServer = {
  type: 'object',
  description: 'A server reached over a URL.',
  properties: {
    type: { type: 'string', examples: ['http', 'sse', 'streamable-http'], description: 'Case-insensitive; sse connects as streamable HTTP.' },
    url: { ...str, description: 'Server URL.' },
    headers: { ...strMap, description: 'Request headers. ${VAR} reads from the environment.' },
    ...timing,
  },
  required: ['url'],
}

const server = { anyOf: [stdioServer, httpServer] }
const serverMap = { type: 'object', additionalProperties: server }

/** JSON schema for the accepted shapes, for the editor's completion and diagnostics. */
export const MCP_JSON_SCHEMA: Record<string, unknown> = {
  $schema: 'http://json-schema.org/draft-07/schema#',
  title: 'MCP servers',
  anyOf: [
    {
      type: 'object',
      description: 'Claude Code / Cursor',
      properties: { mcpServers: serverMap },
      required: ['mcpServers'],
    },
    {
      type: 'object',
      description: 'VS Code',
      properties: { servers: serverMap },
      required: ['servers'],
    },
    stdioServer,
    httpServer,
    serverMap,
  ],
}
