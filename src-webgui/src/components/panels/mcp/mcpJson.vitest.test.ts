import { describe, expect, it } from 'vitest'
import { HEADER_KEEP, blankDraft, draftToRequest, toDraft, type McpDraft } from './mcpForm'
import { MCP_JSON_SCHEMA, checkImportJson, draftToJson, jsonToDraft } from './mcpJson'
import type { McpServer } from '../../../types/config'

const stdio: McpDraft = {
  ...blankDraft(),
  id: 'u1',
  name: 'fs',
  enabled: false,
  transport: 'stdio',
  command: 'npx',
  argsText: '-y\n@scope/server fs\n/path with space',
  envText: 'TOKEN=${TOKEN}\nA=b=c',
  cwd: '/work dir',
  startupSec: '1.5',
  toolSec: '90',
  allowedText: 'read_*, list_*',
  disabledText: 'delete_*',
}
const http: McpDraft = {
  ...blankDraft(),
  id: 'u2',
  name: 'remote',
  transport: 'http',
  url: 'https://x.test/mcp',
  headersText: `Authorization: ${HEADER_KEEP}\nX-Other: Bearer a:b`,
  startupSec: '5',
  allowedText: 'a',
}

function roundTrip(d: McpDraft): McpDraft {
  const r = jsonToDraft(draftToJson(d), d)
  if (!r.ok) throw new Error(r.error)
  return r.draft
}

describe('mcp JSON view round trip', () => {
  it('stdio keeps every field and sends the same request as the form', () => {
    expect(roundTrip(stdio)).toEqual(stdio)
    expect(draftToRequest(roundTrip(stdio), false)).toEqual(draftToRequest(stdio, false))
    const req = draftToRequest(roundTrip(stdio), false)
    expect(req.extras.argv).toEqual(['-y', '@scope/server fs', '/path with space'])
    expect(req.extras.startupTimeoutMs).toBe(1500)
    expect(req.extras.cwd).toBe('/work dir')
    expect(req.enabled).toBe(false)
  })

  it('http keeps url, headers, timeouts and tool lists', () => {
    expect(roundTrip(http)).toEqual(http)
    expect(draftToRequest(roundTrip(http), false)).toEqual(draftToRequest(http, false))
  })

  it('writes the Claude Code shape keyed by name', () => {
    expect(JSON.parse(draftToJson(stdio))).toEqual({
      fs: {
        command: 'npx',
        args: ['-y', '@scope/server fs', '/path with space'],
        env: { TOKEN: '${TOKEN}', A: 'b=c' },
        cwd: '/work dir',
        disabled: true,
        startupTimeoutMs: 1500,
        toolTimeoutMs: 90000,
        allowedTools: ['read_*', 'list_*'],
        disabledTools: ['delete_*'],
      },
    })
    expect(JSON.parse(draftToJson(http)).remote.type).toBe('http')
  })

  it('keeps masked header values masked and sends them back unchanged to keep the secret', () => {
    const saved: McpServer = {
      id: 'u2', name: 'remote', enabled: true, transport: 'http', command: '', args: '', env: '',
      url: 'https://x.test/mcp', toolCount: 0, headers: [['Authorization', HEADER_KEEP]],
    }
    const text = draftToJson(toDraft(saved))
    expect(text).toContain(HEADER_KEEP)
    expect(text).not.toMatch(/Bearer/)
    const r = jsonToDraft(text, toDraft(saved))
    expect(r.ok && draftToRequest(r.draft, false).extras.headers).toEqual([['Authorization', HEADER_KEEP]])
    // A replaced value is sent as typed.
    const edited = jsonToDraft(text.replace(HEADER_KEEP, 'Bearer new'), toDraft(saved))
    expect(edited.ok && draftToRequest(edited.draft, false).extras.headers).toEqual([['Authorization', 'Bearer new']])
  })

  it('a bare entry keeps the draft name; a new key renames; blank draft writes a bare entry', () => {
    const blank = { ...blankDraft(), transport: 'stdio' as const, command: 'x' }
    expect(JSON.parse(draftToJson(blank))).toEqual({ command: 'x' })
    const named = jsonToDraft('{"command":"y"}', { ...blank, name: 'keep' })
    expect(named.ok && named.draft.name).toBe('keep')
    const renamed = jsonToDraft('{"other":{"command":"y"}}', { ...blank, name: 'keep' })
    expect(renamed.ok && renamed.draft.name).toBe('other')
    const none = jsonToDraft('{"command":"y"}', blank)
    expect(none.ok).toBe(false)
  })

  it('accepts the wrapper shapes with one server and rejects several', () => {
    for (const text of [
      '{"mcpServers":{"a":{"command":"c"}}}',
      '{"servers":{"a":{"type":"stdio","command":"c"}}}',
      '"a": {"command":"c"}',
    ]) {
      const r = jsonToDraft(text, blankDraft())
      expect(r.ok && r.draft.name).toBe('a')
    }
    const many = jsonToDraft('{"a":{"command":"c"},"b":{"command":"d"}}', blankDraft())
    expect(!many.ok && many.error).toMatch(/one server/)
  })

  it('reports invalid input with a message instead of throwing', () => {
    expect(jsonToDraft('{ nope', blankDraft())).toMatchObject({ ok: false })
    expect(jsonToDraft('{"a":{"command":1}}', blankDraft())).toMatchObject({ ok: false, error: expect.stringContaining('"command"') })
    expect(jsonToDraft('{"a":{"command":"c","args":"x"}}', blankDraft())).toMatchObject({ ok: false })
    expect(jsonToDraft('{"a":{"command":"c","env":{"K":1}}}', blankDraft())).toMatchObject({ ok: false })
    expect(jsonToDraft('{"a":{"type":"ws","url":"u"}}', blankDraft())).toMatchObject({ ok: false })
  })

  it('maps sse to http with a warning and infers transport from url', () => {
    const sse = jsonToDraft('{"a":{"type":"sse","url":"https://h"}}', blankDraft())
    expect(sse.ok && sse.draft.transport).toBe('http')
    expect(sse.ok && sse.warnings.length).toBe(1)
    const inferred = jsonToDraft('{"a":{"url":"https://h"}}', blankDraft())
    expect(inferred.ok && inferred.draft.transport).toBe('http')
  })
})

describe('import check', () => {
  it('accepts claude code, vs code, bare map and fragment', () => {
    for (const text of [
      '{"mcpServers":{"a":{"command":"c"}}}',
      '{"servers":{"a":{"type":"http","url":"https://h"}}}',
      '{"a":{"command":"c"},"b":{"type":"sse","url":"u"}}',
      '"a": {"command":"c"},',
    ]) expect(checkImportJson(text).ok).toBe(true)
  })

  it('shows the parser error for bad input', () => {
    expect(checkImportJson('')).toMatchObject({ ok: false })
    expect(checkImportJson('{ nope')).toMatchObject({ ok: false, error: expect.stringContaining('not valid JSON') })
    expect(checkImportJson('{"command":"c"}')).toMatchObject({ ok: false, error: expect.stringContaining('needs a name') })
    expect(checkImportJson('{"a":{"args":[]}}')).toMatchObject({ ok: false, error: expect.stringContaining('no server could be imported') })
  })
})

describe('schema', () => {
  const anyOf = (MCP_JSON_SCHEMA.anyOf as Record<string, any>[])
  it('describes the Claude Code, VS Code, single and bare map shapes', () => {
    expect(anyOf.some((s) => s.properties?.mcpServers)).toBe(true)
    expect(anyOf.some((s) => s.properties?.servers)).toBe(true)
    expect(anyOf.some((s) => s.properties?.command && s.properties?.args && s.properties?.env && s.properties?.cwd)).toBe(true)
    expect(anyOf.some((s) => s.properties?.url && s.properties?.headers)).toBe(true)
    expect(anyOf.some((s) => s.additionalProperties?.anyOf)).toBe(true)
  })
})

describe('import pre-check follows the host on multi-line values', () => {
  it('accepts a line break in an env value and an upper-case transport', () => {
    const r = checkImportJson('{"a":{"command":"x","env":{"K":"a\\nb"}},"b":{"type":"HTTP","url":"https://h"}}')
    expect(r.ok && r.names).toEqual(['a', 'b'])
  })
})
