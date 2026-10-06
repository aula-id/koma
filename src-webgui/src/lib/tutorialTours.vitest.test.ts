import { describe, expect, it } from 'vitest'
import { TOUR_CATALOGUE, tourMeta } from './tutorialTours'

describe('Tutorial tour catalogue', () => {
  it('registers one Skills spotlight for topic cards and coach routing', () => {
    const ids = TOUR_CATALOGUE.map((tour) => tour.id)
    expect(new Set(ids).size).toBe(ids.length)
    expect(tourMeta('skills')).toMatchObject({
      id: 'skills',
      title: 'Skills',
      kind: 'spotlight',
    })
  })
})
