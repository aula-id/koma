import { describe, expect, it } from 'vitest'
import { folderGroupKey, groupRecentByFolder } from './sessionFolderGroups'

describe('sessionFolderGroups', () => {
  it('trims dirLabel and treats empty as Other', () => {
    expect(folderGroupKey('koma')).toBe('koma')
    expect(folderGroupKey('  koma  ')).toBe('koma')
    expect(folderGroupKey('')).toBe('')
    expect(folderGroupKey(undefined)).toBe('')
  })

  it('groups live and history by folder, currentDir first, Other last', () => {
    const groups = groupRecentByFolder(
      [
        { kind: 'session', id: 'live-a', name: 'live in koma', dirLabel: 'koma', currentDir: true },
        { kind: 'session', id: 'live-b', name: 'live other' },
      ],
      [
        { id: 'h1', name: 'old koma', lastActive: 2, dirLabel: 'koma', currentDir: true },
        { id: 'h2', name: 'docs', lastActive: 3, dirLabel: 'docs', currentDir: false },
        { id: 'h3', name: 'stray', lastActive: 1, dirLabel: '', currentDir: false },
      ],
    )
    expect(groups.map((g) => g.key)).toEqual(['koma', 'docs', ''])
    expect(groups[0]?.currentDir).toBe(true)
    expect(groups[0]?.live[0]?.id).toBe('live-a')
    expect(groups[0]?.history[0]?.id).toBe('h1')
    expect(groups[2]?.label).toBe('Other')
    expect(groups[2]?.live[0]?.id).toBe('live-b')
    expect(groups[2]?.history[0]?.id).toBe('h3')
  })
})
