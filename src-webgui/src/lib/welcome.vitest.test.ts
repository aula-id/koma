import { describe, expect, it } from 'vitest'
import { recentWelcomeFiles, shouldAutoOpenWelcome, welcomeTabId } from './welcome'

const chat = { id: 'chat', kind: 'chat' } as const

describe('shouldAutoOpenWelcome', () => {
  it('opens when the listing is known and has no files of the kind', () => {
    expect(shouldAutoOpenWelcome('diagram', [chat], [])).toBe(true)
  })
  it('never opens while the listing is unknown', () => {
    expect(shouldAutoOpenWelcome('diagram', [chat], null)).toBe(false)
  })
  it('does not open when files exist', () => {
    expect(shouldAutoOpenWelcome('diagram', [chat], ['.koma/a.diag'])).toBe(false)
  })
  it('an open editor tab suppresses only its own kind', () => {
    const tabs = [chat, { id: 'diagram:/w:.koma/a.diag', kind: 'diagram' } as const]
    expect(shouldAutoOpenWelcome('diagram', tabs, [])).toBe(false)
    expect(shouldAutoOpenWelcome('design', tabs, [])).toBe(true)
  })
  it('an open welcome tab suppresses only its own kind', () => {
    const tabs = [chat, { id: welcomeTabId('diagram'), kind: 'welcome' } as const]
    expect(shouldAutoOpenWelcome('diagram', tabs, [])).toBe(false)
    expect(shouldAutoOpenWelcome('design', tabs, [])).toBe(true)
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
