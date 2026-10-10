import { describe, expect, it } from 'vitest'
import { fitComputerPreview } from './computerPreviewLayout'

const desktop = { width: 1400, height: 900 }

describe('fitComputerPreview', () => {
  it('restores the default size when a saved box collapsed below the floor', () => {
    const box = fitComputerPreview({ x: 20, y: 80, width: 1, height: 1 }, desktop)
    expect(box.width).toBe(560)
    expect(box.height).toBe(315)
  })

  it('keeps at least 250×180 when the viewport is empty', () => {
    const box = fitComputerPreview({ width: 1, height: 1 }, { width: 0, height: 0 })
    expect(box.width).toBeGreaterThanOrEqual(250)
    expect(box.height).toBeGreaterThanOrEqual(180)
  })

  it('keeps a user size above the floor', () => {
    const box = fitComputerPreview({ x: 40, y: 80, width: 400, height: 220 }, desktop)
    expect(box.width).toBe(400)
    expect(box.height).toBe(220)
    expect(box.x).toBe(40)
    expect(box.y).toBe(80)
  })

  it('returns a box that misses the viewport to the default corner', () => {
    const box = fitComputerPreview({ x: 9000, y: 9000, width: 400, height: 220 }, desktop)
    expect(box.x).toBe(1400 - 400 - 24)
    expect(box.y).toBe(70)
  })
})
