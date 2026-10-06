import { describe, expect, it } from 'vitest'
import { nextSkillSelection } from './skillListSelection'

const ids = ['a', 'b', 'c', 'd']

describe('skill list selection', () => {
  it('plain click replaces selection and anchor', () => {
    expect(nextSkillSelection(ids, ['a', 'c'], 'a', 'b')).toEqual({ selected: ['b'], anchor: 'b' })
  })

  it('Ctrl/Cmd toggles while preserving visible order', () => {
    expect(nextSkillSelection(ids, ['c'], 'c', 'a', { ctrlKey: true })).toEqual({ selected: ['a', 'c'], anchor: 'a' })
    expect(nextSkillSelection(ids, ['a', 'c'], 'a', 'c', { metaKey: true })).toEqual({ selected: ['a'], anchor: 'c' })
  })

  it('Shift replaces selection with the inclusive anchor range', () => {
    expect(nextSkillSelection(ids, ['a'], 'b', 'd', { shiftKey: true })).toEqual({ selected: ['b', 'c', 'd'], anchor: 'b' })
  })

  it('ignores identities outside the current filtered list', () => {
    expect(nextSkillSelection(ids, ['a'], 'a', 'missing')).toEqual({ selected: ['a'], anchor: 'a' })
  })
})
