import { describe, expect, it } from 'vitest'
import { recentWelcomeFiles, shouldOpenWelcome } from './welcome'

describe('shouldOpenWelcome', () => {
  it('opens whatever the active tab is, unless it is an editor of the same kind', () => {
    expect(shouldOpenWelcome('diagram', 'chat')).toBe(true)
    expect(shouldOpenWelcome('diagram', undefined)).toBe(true)
    expect(shouldOpenWelcome('diagram', 'welcome')).toBe(true)
    expect(shouldOpenWelcome('diagram', 'diagram')).toBe(false)
    expect(shouldOpenWelcome('design', 'design')).toBe(false)
  })
  it('an editor of the other kind does not suppress it', () => {
    expect(shouldOpenWelcome('design', 'diagram')).toBe(true)
    expect(shouldOpenWelcome('diagram', 'design')).toBe(true)
  })
})

describe('recentWelcomeFiles', () => {
  it('sorts by basename and caps at the limit', () => {
    expect(recentWelcomeFiles(['.koma/c.diag', '.koma/a.diag', '.koma/b.diag'], 2)).toEqual(['.koma/a.diag', '.koma/b.diag'])
  })
  it('returns an empty list for no files', () => {
    expect(recentWelcomeFiles([])).toEqual([])
  })
})
