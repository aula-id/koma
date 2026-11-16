import { afterEach, describe, expect, it } from 'vitest'
import { render } from 'vitest-browser-react'
import { AboutDialog, AboutPanel, showAbout } from './AboutKoma'

const build = { version: '0.4.15', channel: 'stable', sha: 'abcdef0', target: 'x86_64-pc-windows-msvc', os: 'windows', buildDate: null }

afterEach(() => {
  delete window.__komaBuild
})

describe('About Koma', () => {
  it('shows the build facts', async () => {
    window.__komaBuild = build
    const screen = await render(<AboutPanel />)
    await expect.element(screen.getByText('0.4.15')).toBeVisible()
    await expect.element(screen.getByText('x86_64-pc-windows-msvc')).toBeVisible()
  })

  it('opens from showAbout and closes on Escape', async () => {
    window.__komaBuild = build
    const screen = await render(<AboutDialog />)
    showAbout()
    await expect.element(screen.getByRole('dialog', { name: 'About Koma' })).toBeVisible()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await expect.element(screen.getByRole('dialog', { name: 'About Koma' })).not.toBeInTheDocument()
  })
})
