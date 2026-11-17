import { describe, expect, it } from 'vitest'
import { formatBuildInfo, readBuildInfo } from './buildInfo'

const FALLBACK = { version: 'unknown', channel: 'stable', sha: 'unknown', target: 'unknown', os: 'unknown', buildDate: null }

describe('readBuildInfo', () => {
  it('falls back for missing or invalid input', () => {
    expect(readBuildInfo(undefined)).toEqual(FALLBACK)
    expect(readBuildInfo(null)).toEqual(FALLBACK)
    expect(readBuildInfo({ version: 5 })).toEqual(FALLBACK)
  })

  it('round-trips a full object, keeping a null date', () => {
    const full = { version: '0.4.15', channel: 'preview', sha: 'abcdef0', target: 'x86_64-pc-windows-msvc', os: 'windows', buildDate: '2026-01-01' }
    expect(readBuildInfo(full)).toEqual(full)
    expect(readBuildInfo({ ...full, buildDate: null }).buildDate).toBeNull()
  })
})

describe('formatBuildInfo', () => {
  it('renders the five-line block', () => {
    const info = readBuildInfo({ version: '0.4.15', channel: 'stable', sha: 'unknown', target: 'aarch64-apple-darwin', os: 'macos', buildDate: null })
    expect(formatBuildInfo(info)).toBe(
      'Koma 0.4.15 (stable)\nCommit: unknown\nTarget: aarch64-apple-darwin\nOS: macos\nBuilt: unknown',
    )
  })
})

describe('readBuildInfo with malformed input', () => {
  it.each([5, 'text', true, [], () => 1])('degrades %s to unknown', (raw) => {
    expect(readBuildInfo(raw)).toEqual(FALLBACK)
  })

  it('replaces wrong types, empties and nulls field by field', () => {
    const info = readBuildInfo({ version: '1.2.3', channel: 'preview', sha: '', target: null, os: 7, buildDate: { d: 1 } })
    expect(info).toEqual({ ...FALLBACK, version: '1.2.3', channel: 'preview' })
  })

  it('treats an unknown channel as stable', () => {
    expect(readBuildInfo({ channel: 'nightly' }).channel).toBe('stable')
  })
})
