import '../styles.css'
import { expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { useKoma } from '../store/koma'

vi.mock('./CodingTasks', () => ({ showCodingTasks: vi.fn() }))
vi.mock('../lib/coding-service', async importOriginal => {
  const actual = await importOriginal<typeof import('../lib/coding-service')>()
  return {
    ...actual,
    codingRequest: vi.fn().mockResolvedValue({
      platform: 'macos', fingerprint: 'test', packs: [{
        id: 'python', label: 'Python', runtime: '/usr/bin/python3', runtimeName: 'python3',
        selected: '/usr/bin/python3', candidates: [],
        lsp: '/Users/test/.koma/lsp/basedpyright/venv/bin/basedpyright-langserver',
        server: 'basedpyright', serverOptions: ['basedpyright'],
        adapter: null, adapterName: 'debugpy', test: 'pytest',
      }],
    }),
  }
})

import { CodingPacks, showCodingPacks } from './CodingPacks'

it.each([360, 780])('separates environment and installation at %ipx with one scroll body', async width => {
  useKoma.setState(state => ({
    coding: { ...state.coding, activeRoot: '/workspace' },
    remoteState: { ...state.remoteState, hostId: 'local' },
  }))
  const screen = await render(<div style={{ width, height: 248, display: 'flex' }}><CodingPacks visible /></div>)
  await expect.element(screen.getByLabelText('Python interpreter')).toBeVisible()
  await expect.element(screen.getByRole('button', { name: 'Review changes', exact: true })).not.toBeInTheDocument()
  const section = document.querySelector<HTMLElement>('[aria-label="Language packs"]')!
  expect(section.scrollWidth).toBeLessThanOrEqual(section.clientWidth)
  const language = screen.getByLabelText('Language', { exact: true }).element().getBoundingClientRect()
  const arrow = screen.getByLabelText('Language', { exact: true }).element().parentElement!.querySelector('svg')!.getBoundingClientRect()
  expect(arrow.right).toBeLessThanOrEqual(language.right)
  expect(arrow.left).toBeGreaterThan(language.right - 24)
  expect(language.width).toBeLessThanOrEqual(160)
  await screen.getByRole('button', { name: 'Tools', exact: true }).click()
  await expect.element(screen.getByRole('button', { name: 'Review changes', exact: true })).toBeVisible()
  await expect.element(screen.getByLabelText('Python interpreter')).not.toBeInTheDocument()
  await screen.getByRole('button', { name: 'Environment', exact: true }).click()
  showCodingPacks({ hostId: 'local', root: '/workspace' }, { notebook: true })
  await expect.element(screen.getByRole('checkbox', { name: 'Notebook kernel' })).toBeChecked()
  await expect.element(screen.getByRole('button', { name: 'Tools', exact: true })).toHaveAttribute('aria-pressed', 'true')
})
