import { describe, expect, it } from 'vitest'
import { isEditorTabActive, recentWelcomeFiles, shouldOpenWelcome } from './welcome'
import type { Tab } from '../store/types/tabs'

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

describe('isEditorTabActive', () => {
  const diagram = { id: 'd1', kind: 'diagram', root: '/r', path: 'a.diagram' } as unknown as Tab
  const design = { id: 'g1', kind: 'design', root: '/r', path: 'a.design' } as unknown as Tab
  const chat = { id: 'chat', kind: 'chat' } as unknown as Tab
  it('is false with no tabs or an unknown active id', () => {
    expect(isEditorTabActive('diagram', { tabs: [], activeTabId: 'chat' })).toBe(false)
  })
  it('is false when an editor of that kind is open but another tab is active', () => {
    expect(isEditorTabActive('diagram', { tabs: [chat, diagram, design], activeTabId: 'chat' })).toBe(false)
    expect(isEditorTabActive('design', { tabs: [chat, diagram, design], activeTabId: 'd1' })).toBe(false)
  })
  it('is true when the active tab is that kind', () => {
    expect(isEditorTabActive('diagram', { tabs: [chat, diagram], activeTabId: 'd1' })).toBe(true)
    expect(isEditorTabActive('design', { tabs: [chat, design], activeTabId: 'g1' })).toBe(true)
  })
})
