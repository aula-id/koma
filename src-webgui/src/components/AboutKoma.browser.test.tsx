import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'
import { render } from 'vitest-browser-react'
import { AboutPanel } from './AboutKoma'
import HelpTab from './HelpTab'
import SettingsTab from './SettingsTab'
import { CodingPalette, showCodingPalette } from './CodingPalette'
import { useKoma } from '../store/koma'

const build = { version: '0.4.15', channel: 'stable', sha: 'abcdef0', target: 'x86_64-pc-windows-msvc', os: 'windows', buildDate: null }
const pause = (ms = 60) => new Promise((resolve) => setTimeout(resolve, ms))
const req = vi.fn()

/** Mirrors the shell: the settings tab is mounted only once a tab entry exists. */
function Shell() {
  const open = useKoma((s) => s.ui.tabs.some((t) => t.id === 'settings'))
  return (
    <>
      <HelpTab />
      <CodingPalette />
      {open && <SettingsTab />}
    </>
  )
}

const aboutSection = () => document.getElementById('settings-section-about')

beforeEach(() => {
  req.mockClear()
  useKoma.setState((s) => ({ req, ui: { ...s.ui, tabs: [], activeTabId: '' } }))
})
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

  it.each([
    ['missing', undefined],
    ['null', null],
    ['wrong shape', 'oops'],
    ['null fields', { version: null, channel: null, sha: null, target: null, os: null, buildDate: null }],
    ['partial', { version: '1.0.0' }],
  ])('renders unknown cleanly when window.__komaBuild is %s', async (_name, value) => {
    window.__komaBuild = value
    const screen = await render(<AboutPanel />)
    await expect.element(screen.getByText('Built')).toBeVisible()
    expect(document.body.textContent).toContain('unknown')
    expect(document.body.textContent).not.toMatch(/null|undefined|\[object/)
  })

  it('opens Settings on the About section from the Help tab, even without build info', async () => {
    await render(<Shell />)
    expect(aboutSection()).toBeNull()
    await userEvent.click(document.querySelector<HTMLElement>('[aria-label="About Koma"]')!)
    await pause()
    expect(useKoma.getState().ui.activeTabId).toBe('settings')
    expect(aboutSection()).not.toBeNull()
    expect(aboutSection()!.textContent).toContain('Copy build info')
    expect(document.activeElement).toBe(aboutSection())
    expect(document.querySelectorAll('[aria-label="About Koma"]').length).toBeGreaterThan(0)
    expect(document.body.textContent).not.toContain('Help sections About')
  })

  it('opens Settings on the About section from the command palette', async () => {
    window.__komaBuild = build
    await render(<Shell />)
    showCodingPalette('commands')
    await pause()
    const input = document.querySelector<HTMLInputElement>('[role="dialog"] input')!
    await userEvent.type(input, 'About Koma')
    await userEvent.keyboard('{Enter}')
    await pause()
    expect(useKoma.getState().ui.activeTabId).toBe('settings')
    expect(aboutSection()!.textContent).toContain('0.4.15')
    expect(document.activeElement).toBe(aboutSection())
  })

  it('focuses About in an already-open Settings tab', async () => {
    await render(<Shell />)
    useKoma.getState().openSettingsTab()
    await pause()
    await userEvent.click(document.querySelector<HTMLElement>('[aria-label="About Koma"]')!)
    await pause()
    expect(document.activeElement).toBe(aboutSection())
  })
})
