import { describe, expect, it } from 'vitest'
import type { McpServer } from '../../../types/config'
import {
  HEADER_KEEP,
  blankDraft,
  draftToRequest,
  linesToList,
  linesToPairs,
  secToMs,
  splitList,
  toDraft,
} from './mcpForm'

const server: McpServer = {
  id: 'u1',
  name: 'gh',
  enabled: true,
  transport: 'stdio',
  command: 'npx',
  args: 'a b -y',
  env: 'K=x,y',
  url: '',
  toolCount: 0,
  argv: ['a b', '-y'],
  envPairs: [['K', 'x,y']],
  headers: [['Authorization', HEADER_KEEP]],
  cwd: '/w',
  startupTimeoutMs: 1500,
  toolTimeoutMs: 60000,
  allowedTools: ['read_*'],
  disabledTools: ['rm', 'mv'],
}

describe('mcpForm', () => {
  it('round-trips an argument containing spaces', () => {
    const req = draftToRequest(toDraft(server), false)
    expect(req.extras.argv).toEqual(['a b', '-y'])
    expect(req.extras.envPairs).toEqual([['K', 'x,y']])
    expect(req.uuid).toBe('u1')
  })

  it('sends the legacy string fields next to the extras', () => {
    const req = draftToRequest(toDraft(server), false)
    expect(req.args).toBe('a b -y')
    expect(req.env).toBe('K=x,y')
    expect(req.extras.cwd).toBe('/w')
    expect(req.extras.startupTimeoutMs).toBe(1500)
    expect(req.extras.toolTimeoutMs).toBe(60000)
    expect(req.extras.allowedTools).toEqual(['read_*'])
    expect(req.extras.disabledTools).toEqual(['rm', 'mv'])
  })

  it('keeps the masked header value so the host keeps the stored one', () => {
    const d = toDraft(server)
    expect(d.headersText).toBe(`Authorization: ${HEADER_KEEP}`)
    expect(draftToRequest(d, false).extras.headers).toEqual([['Authorization', HEADER_KEEP]])
  })

  it('falls back to the legacy strings when the host sent no structured fields', () => {
    const d = toDraft({ ...server, argv: undefined, envPairs: undefined })
    expect(d.argsText).toBe('a\nb\n-y')
    expect(d.envText).toBe('K=x\ny=')
  })

  it('a new server sends a null uuid', () => {
    expect(draftToRequest(blankDraft(), true).uuid).toBeNull()
  })

  it('linesToPairs splits at the first separator, trims, drops empty keys', () => {
    expect(linesToPairs(' A = b=c \n=x\n\nB', '=')).toEqual([
      ['A', 'b=c'],
      ['B', ''],
    ])
    expect(linesToPairs('Authorization:  Bearer a:b ', ':')).toEqual([
      ['Authorization', 'Bearer a:b'],
    ])
  })

  it('splitList handles commas and newlines', () => {
    expect(splitList('a, b\nc,,  ')).toEqual(['a', 'b', 'c'])
    expect(linesToList(' x \n\n y')).toEqual(['x', 'y'])
  })

  it('secToMs converts seconds and treats blanks and junk as 0', () => {
    expect(secToMs('')).toBe(0)
    expect(secToMs('1.5')).toBe(1500)
    expect(secToMs('abc')).toBe(0)
    expect(secToMs('-3')).toBe(0)
  })
})
