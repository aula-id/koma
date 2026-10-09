import '../styles.css'
import 'driver.js/dist/driver.css'
import { afterEach, describe, expect, it } from 'vitest'

const palettes = [
  { name: 'dark', bg: '#000000', fg: '#e6e6e6', dim: '#adadad', accent: '#39ff14' },
  { name: 'autumn', bg: '#2e2a20', fg: '#f1dca7', dim: '#baa587', accent: '#ffcb69' },
] as const

function rgb(hex: string): string {
  const value = Number.parseInt(hex.slice(1), 16)
  return `rgb(${value >> 16}, ${(value >> 8) & 255}, ${value & 255})`
}

afterEach(() => {
  document.querySelector('.driver-popover')?.remove()
})

describe('Tutorial popover theme contrast', () => {
  for (const palette of palettes) {
    it(`keeps the ${palette.name} surface and text after Driver's late CSS`, () => {
      const root = document.documentElement
      root.style.setProperty('--koma-bg', palette.bg)
      root.style.setProperty('--koma-fg', palette.fg)
      root.style.setProperty('--koma-dim', palette.dim)
      root.style.setProperty('--koma-accent', palette.accent)

      const popover = document.createElement('div')
      popover.className = 'driver-popover koma-driver-theme'
      popover.innerHTML = `
        <div class="driver-popover-title">Skills panel</div>
        <div class="driver-popover-description">Open the dedicated Skills catalogue.</div>
        <div class="driver-popover-arrow driver-popover-arrow-side-left"></div>
        <button class="driver-popover-next-btn">Next</button>
      `
      document.body.append(popover)

      expect(getComputedStyle(popover).backgroundColor).toBe(rgb(palette.bg))
      expect(getComputedStyle(popover.querySelector('.driver-popover-title')!).color).toBe(rgb(palette.fg))
      expect(getComputedStyle(popover.querySelector('.driver-popover-next-btn')!).backgroundColor).toBe(rgb(palette.accent))
      expect(getComputedStyle(popover.querySelector('.driver-popover-arrow')!).borderLeftColor).toBe(rgb(palette.bg))
    })
  }
})
