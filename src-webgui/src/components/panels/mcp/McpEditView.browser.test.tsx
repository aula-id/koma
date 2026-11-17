import { describe, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'
import { render } from 'vitest-browser-react'
import '../../../styles.css'
import { McpEditView } from './McpEditView'
import { blankDraft } from './mcpForm'

function mount(draft = blankDraft()) {
  return render(
    <div data-testid="host" style={{ width: 240, height: 500, display: 'flex', flexDirection: 'column' }} className="bg-koma-panel">
      <McpEditView draft={draft} isNew onChange={vi.fn()} onSave={vi.fn()} onCancel={vi.fn()} />
    </div>,
  )
}
const toggles = () => [...document.querySelectorAll<HTMLButtonElement>('button[aria-expanded]')]

describe('McpEditView layout', () => {
  it('fits 240px, starts with advanced groups collapsed, keeps Save pinned', async () => {
    await mount()
    expect(toggles().length).toBeGreaterThan(0)
    expect(toggles().every((b) => b.getAttribute('aria-expanded') === 'false')).toBe(true)
    const scroller = document.querySelector<HTMLElement>('.overflow-y-auto')!
    expect(scroller.scrollWidth).toBeLessThanOrEqual(scroller.clientWidth)
    expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(document.documentElement.clientWidth)
    await userEvent.click(toggles()[0])
    expect(toggles()[0].getAttribute('aria-expanded')).toBe('true')
    expect(scroller.scrollWidth).toBeLessThanOrEqual(scroller.clientWidth)
    const save = [...document.querySelectorAll('button')].find((b) => b.textContent === 'Save')!
    expect(save.getBoundingClientRect().bottom).toBeLessThanOrEqual(document.querySelector('[data-testid="host"]')!.getBoundingClientRect().bottom)
  })
  it('shows a summary and opens the group when it holds values', async () => {
    const d = { ...blankDraft(), envText: 'A=1\nB=2', toolSec: 'abc' }
    await mount(d)
    expect(document.body.textContent).toContain('2 variables')
    const env = toggles().find((b) => b.textContent?.includes('Environment'))!
    expect(env.getAttribute('aria-expanded')).toBe('true')
    const limits = toggles().find((b) => b.textContent?.includes('timeouts'))!
    expect(limits.getAttribute('aria-expanded')).toBe('true')
    expect(document.body.textContent).toContain('Enter a number above 0')
    const tools = toggles().find((b) => b.textContent?.includes('Tool access'))!
    expect(tools.getAttribute('aria-expanded')).toBe('false')
    expect(tools.textContent).toContain('all allowed')
  })
})
