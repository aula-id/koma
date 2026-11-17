import type { McpServer, Transport } from '../../../types/config'

// Placeholder the host sends (and accepts back unchanged) for a stored header value, so
// header secrets never reach the webview. Mirrors `HEADER_KEEP` in app/mode/mcp/state.rs.
export const HEADER_KEEP = '********'

export type McpDraft = {
  id: string
  name: string
  enabled: boolean
  transport: Transport
  command: string
  /** One argument per line. */
  argsText: string
  /** `KEY=value` per line. */
  envText: string
  url: string
  /** `Name: value` per line. */
  headersText: string
  cwd: string
  startupSec: string
  toolSec: string
  allowedText: string
  disabledText: string
}

export type McpServerRequest = {
  r: 'SetMcpServer'
  uuid: string | null
  name: string
  enabled: boolean
  transport: Transport
  command: string
  args: string
  env: string
  url: string
  extras: {
    argv: string[]
    envPairs: [string, string][]
    headers: [string, string][]
    cwd: string
    startupTimeoutMs: number
    toolTimeoutMs: number
    allowedTools: string[]
    disabledTools: string[]
  }
}

let seq = 0

export function blankDraft(): McpDraft {
  seq += 1
  return {
    id: `srv-${seq}`,
    name: '',
    enabled: true,
    transport: 'stdio',
    command: '',
    argsText: '',
    envText: '',
    url: '',
    headersText: '',
    cwd: '',
    startupSec: '',
    toolSec: '',
    allowedText: '',
    disabledText: '',
  }
}

/** Non-empty trimmed lines. */
export function linesToList(text: string): string[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l !== '')
}

/** `key<sep>value` per line, split at the FIRST separator; both sides trimmed; lines with
 *  an empty key are dropped. A line without the separator is a key with an empty value. */
export function linesToPairs(text: string, sep: '=' | ':'): [string, string][] {
  const out: [string, string][] = []
  for (const line of text.split('\n')) {
    const at = line.indexOf(sep)
    const key = (at < 0 ? line : line.slice(0, at)).trim()
    if (key === '') continue
    out.push([key, at < 0 ? '' : line.slice(at + 1).trim()])
  }
  return out
}

/** Comma- or newline-separated list, trimmed, empties dropped. */
export function splitList(text: string): string[] {
  return text
    .split(/[,\n]/)
    .map((s) => s.trim())
    .filter((s) => s !== '')
}

/** Seconds text to milliseconds; blank, invalid or non-positive is 0 (= use the default). */
export function secToMs(text: string): number {
  const sec = Number.parseFloat(text.trim())
  return Number.isFinite(sec) && sec > 0 ? Math.round(sec * 1000) : 0
}

function msToSec(ms: number | undefined): string {
  return ms ? String(ms / 1000) : ''
}

function parseLegacyEnv(env: string): [string, string][] {
  return env
    .split(',')
    .map((p) => p.trim())
    .filter((p) => p !== '')
    .map((p): [string, string] => {
      const at = p.indexOf('=')
      return at < 0 ? [p, ''] : [p.slice(0, at).trim(), p.slice(at + 1).trim()]
    })
}

export function toDraft(s: McpServer): McpDraft {
  const argv = s.argv ?? s.args.split(' ').filter((a) => a !== '')
  const env = s.envPairs ?? parseLegacyEnv(s.env)
  return {
    id: s.id,
    name: s.name,
    enabled: s.enabled,
    transport: s.transport,
    command: s.command,
    argsText: argv.join('\n'),
    envText: env.map(([k, v]) => `${k}=${v}`).join('\n'),
    url: s.url,
    headersText: (s.headers ?? []).map(([k, v]) => `${k}: ${v}`).join('\n'),
    cwd: s.cwd ?? '',
    startupSec: msToSec(s.startupTimeoutMs),
    toolSec: msToSec(s.toolTimeoutMs),
    allowedText: (s.allowedTools ?? []).join(', '),
    disabledText: (s.disabledTools ?? []).join(', '),
  }
}

export function draftToRequest(d: McpDraft, isNew: boolean): McpServerRequest {
  const argv = linesToList(d.argsText)
  const envPairs = linesToPairs(d.envText, '=')
  return {
    r: 'SetMcpServer',
    uuid: isNew ? null : d.id,
    name: d.name,
    enabled: d.enabled,
    transport: d.transport,
    command: d.command,
    args: argv.join(' '),
    env: envPairs.map(([k, v]) => `${k}=${v}`).join(', '),
    url: d.url,
    extras: {
      argv,
      envPairs,
      headers: linesToPairs(d.headersText, ':'),
      cwd: d.cwd.trim(),
      startupTimeoutMs: secToMs(d.startupSec),
      toolTimeoutMs: secToMs(d.toolSec),
      allowedTools: splitList(d.allowedText),
      disabledTools: splitList(d.disabledText),
    },
  }
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`
}

export function envSummary(d: McpDraft): string {
  const n = linesToPairs(d.envText, '=').length
  return n === 0 ? 'none' : plural(n, 'variable', 'variables')
}

export function headersSummary(d: McpDraft): string {
  const n = linesToPairs(d.headersText, ':').length
  return n === 0 ? 'none' : plural(n, 'header', 'headers')
}

function secLabel(text: string): string {
  return `${secToMs(text) / 1000}s`
}

/** What the "working directory and timeouts" group holds; `cwd` only counts for stdio. */
export function limitsSummary(d: McpDraft): string {
  const parts: string[] = []
  if (d.transport === 'stdio' && d.cwd.trim()) parts.push('folder set')
  if (secToMs(d.startupSec) > 0) parts.push(`startup ${secLabel(d.startupSec)}`)
  if (secToMs(d.toolSec) > 0) parts.push(`tool ${secLabel(d.toolSec)}`)
  return parts.length ? parts.join(', ') : 'default'
}

export function toolsSummary(d: McpDraft): string {
  const allowed = splitList(d.allowedText).length
  const disabled = splitList(d.disabledText).length
  const parts: string[] = []
  if (allowed) parts.push(`${allowed} allowed`)
  if (disabled) parts.push(`${disabled} disabled`)
  return parts.length ? parts.join(', ') : 'all allowed'
}

/** A timeout the user typed that the host would ignore (not a positive number). */
export function badSeconds(text: string): boolean {
  return text.trim() !== '' && secToMs(text) === 0
}

/** True when the limits group holds anything worth showing expanded. */
export function limitsHasValues(d: McpDraft): boolean {
  return limitsSummary(d) !== 'default' || d.startupSec.trim() !== '' || d.toolSec.trim() !== ''
}

export type McpStatus = { label: string; tone: 'ok' | 'warn' | 'off' | 'wait' }

/** One-line live state of a saved server, for the editor header. */
export function serverStatus(s: McpServer): McpStatus {
  if (!s.enabled) return { label: 'Disabled', tone: 'off' }
  if (s.error) return { label: `Error: ${s.error}`, tone: 'warn' }
  if (s.connected) return { label: `Connected, ${plural(s.toolCount, 'tool', 'tools')}`, tone: 'ok' }
  return { label: 'Checking…', tone: 'wait' }
}
