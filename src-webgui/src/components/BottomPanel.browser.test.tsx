import '../styles.css'
import { page } from 'vitest/browser'
import { expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { BottomPanel } from './BottomPanel'
import { useKoma } from '../store/koma'

const paint = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
const mount = () => render(<div style={{ width: 500, height: 400 }}><BottomPanel /></div>)

it('has one close control and no collapse control', async () => {
  useKoma.setState({ bottomPanelTab: 'problems' })
  await mount(); await paint()
  expect(document.querySelectorAll('[aria-label="Close workspace panel"]').length).toBe(1)
  expect(document.querySelector('[aria-label="Collapse workspace panel"]')).toBeNull()
})

it('maximizes to 75% of the parent and restores', async () => {
  useKoma.setState({ bottomPanelTab: 'problems' })
  await mount(); await paint()
  const panel = document.querySelector('#workspace-bottom-panel')!
  const before = panel.getBoundingClientRect().height
  await page.getByRole('button', { name: 'Maximize workspace panel' }).click()
  await paint()
  expect(panel.getBoundingClientRect().height).toBeCloseTo(300, 0)
  await page.getByRole('button', { name: 'Restore workspace panel' }).click()
  await paint()
  expect(panel.getBoundingClientRect().height).toBeCloseTo(before, 0)
})

it('shows task actions in the header', async () => {
  useKoma.setState({ bottomPanelTab: 'tasks', req: vi.fn() })
  await mount(); await paint()
  const refresh = document.querySelector('[aria-label="Refresh tasks"]')!
  const header = document.querySelector('[role="tablist"]')!.parentElement!
  expect(header.contains(refresh)).toBe(true)
})
