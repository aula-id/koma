import { describe, expect, it } from 'vitest'
import { CODING_PATH_DND } from './codingRef'
import { isExternalFileDrag } from './externalFileDrag'

function transfer(types: string[]): DataTransfer {
  return { types } as DataTransfer
}

describe('isExternalFileDrag', () => {
  it('matches OS file drags from Finder, Explorer, and Linux', () => {
    expect(isExternalFileDrag(transfer(['Files']))).toBe(true)
    expect(isExternalFileDrag(transfer(['text/uri-list', 'text/plain']))).toBe(true)
    expect(isExternalFileDrag(transfer(['application/x-moz-file']))).toBe(true)
  })

  it('ignores in-app tab and coding-tree drags', () => {
    expect(isExternalFileDrag(transfer(['application/x-koma-tab', 'Files']))).toBe(false)
    expect(isExternalFileDrag(transfer([CODING_PATH_DND, 'text/uri-list']))).toBe(false)
    expect(isExternalFileDrag(transfer(['text/plain']))).toBe(false)
    expect(isExternalFileDrag(null)).toBe(false)
  })
})
