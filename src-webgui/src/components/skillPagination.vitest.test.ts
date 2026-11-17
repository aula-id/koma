import { describe, expect, it } from 'vitest'
import { SKILL_PAGE_FALLBACK, clampPage, pageBounds, pageCount, pageOfIndex, skillPageSize } from './skillPagination'

describe('skillPageSize', () => {
  it('falls back before the list is measured', () => {
    expect(skillPageSize(0)).toBe(SKILL_PAGE_FALLBACK)
    expect(skillPageSize(Number.NaN)).toBe(SKILL_PAGE_FALLBACK)
  })

  it('fits whole rows into the list height', () => {
    expect(skillPageSize(700)).toBe(16)
  })

  it('never goes below three rows', () => {
    expect(skillPageSize(20)).toBe(3)
  })
})

describe('paging', () => {
  it('counts at least one page', () => {
    expect(pageCount(0, 10)).toBe(1)
    expect(pageCount(10, 10)).toBe(1)
    expect(pageCount(11, 10)).toBe(2)
  })

  it('clamps after the list shrinks', () => {
    expect(clampPage(5, 25, 10)).toBe(2)
    expect(clampPage(-1, 25, 10)).toBe(0)
    expect(clampPage(1, 0, 10)).toBe(0)
  })

  it('maps an index to its page', () => {
    expect(pageOfIndex(0, 10)).toBe(0)
    expect(pageOfIndex(9, 10)).toBe(0)
    expect(pageOfIndex(10, 10)).toBe(1)
    expect(pageOfIndex(-1, 10)).toBe(0)
  })

  it('bounds the last partial page', () => {
    expect(pageBounds(0, 25, 10)).toEqual({ start: 0, end: 10 })
    expect(pageBounds(2, 25, 10)).toEqual({ start: 20, end: 25 })
    expect(pageBounds(0, 0, 10)).toEqual({ start: 0, end: 0 })
  })
})
