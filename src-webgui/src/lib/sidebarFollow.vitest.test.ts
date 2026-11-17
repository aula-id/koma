import { describe, expect, it } from 'vitest'
import { resolveTabFollow, tabFollowView } from './sidebarFollow'

const diagram = { id: 'diagram-1', kind: 'diagram' } as const
const diagram2 = { id: 'diagram-2', kind: 'diagram' } as const
const design = { id: 'design-1', kind: 'design' } as const
const welcomeDiagram = { id: 'welcome-diagram', kind: 'welcome', which: 'diagram' } as const
const welcomeDesign = { id: 'welcome-design', kind: 'welcome', which: 'design' } as const

describe('resolveTabFollow', () => {
  it('diagram editor or Welcome becoming active switches to Diagram and reopens', () => {
    expect(resolveTabFollow('chat', diagram)).toEqual({ view: 'diagram', open: true })
    expect(resolveTabFollow('chat', welcomeDiagram)).toEqual({ view: 'diagram', open: true })
  })

  it('design editor or Welcome becoming active switches to Design and reopens', () => {
    expect(resolveTabFollow('chat', design)).toEqual({ view: 'design', open: true })
    expect(resolveTabFollow('diagram-1', welcomeDesign)).toEqual({ view: 'design', open: true })
  })

  it('does not force again while the same tab stays active', () => {
    expect(resolveTabFollow('design-1', design)).toBeNull()
    expect(resolveTabFollow('welcome-diagram', welcomeDiagram)).toBeNull()
  })

  it('a different tab of the same kind (split focus, tab switch) acts again', () => {
    expect(resolveTabFollow('diagram-1', diagram2)).toEqual({ view: 'diagram', open: true })
  })

  it('other tab kinds and a missing tab do nothing', () => {
    expect(resolveTabFollow('design-1', { id: 'chat', kind: 'chat' })).toBeNull()
    expect(resolveTabFollow('design-1', { id: 'settings', kind: 'settings' })).toBeNull()
    expect(resolveTabFollow('design-1', undefined)).toBeNull()
  })
})

describe('tabFollowView', () => {
  it('maps kinds and welcome variants', () => {
    expect(tabFollowView({ kind: 'diagram' })).toBe('diagram')
    expect(tabFollowView({ kind: 'welcome', which: 'design' })).toBe('design')
    expect(tabFollowView({ kind: 'code' } as never)).toBeNull()
    expect(tabFollowView(undefined)).toBeNull()
  })
})
