import { describe, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'
import { render } from 'vitest-browser-react'
import '../../../styles.css'
import { McpEditView } from './McpEditView'
import { blankDraft } from './mcpForm'

function mount(draft = blankDraft(), width = 240) {
  return render(
    <div data-testid="host" style={{ width, height: 500, display: 'flex', flexDirection: 'column' }} className="bg-koma-panel">
      <McpEditView draft={draft} isNew onChange={vi.fn()} onSave={vi.fn()} onCancel={vi.fn()} />
    </div>,
  )
}
const toggles = () => [...document.querySelectorAll<HTMLButtonElement>('button[aria-expanded]:not([aria-label^="Help:"])')]

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
  it('fits 400px with every group open and ties labels and the switch to their controls', async () => {
    const d = { ...blankDraft(), envText: 'A=1', toolSec: 'abc', allowedText: 'x' }
    await mount(d, 400)
    for (const b of toggles()) if (b.getAttribute('aria-expanded') === 'false') await userEvent.click(b)
    const scroller = document.querySelector<HTMLElement>('.overflow-y-auto')!
    expect(scroller.scrollWidth).toBeLessThanOrEqual(scroller.clientWidth)
    expect(document.querySelectorAll('.overflow-y-auto').length).toBe(1)
    const inputs = [...document.querySelectorAll<HTMLElement>('input, textarea')]
    expect(inputs.length).toBeGreaterThan(4)
    for (const el of inputs) expect(document.querySelector(`label[for="${el.id}"]`)).not.toBeNull()
    const args = document.querySelector<HTMLElement>('textarea[placeholder^="-y"]')!
    expect(args.getAttribute('aria-describedby')).toBeNull()
    expect(document.querySelector('[role="switch"]')?.getAttribute('aria-labelledby')).toBeTruthy()
    expect(document.activeElement?.getAttribute('placeholder')).toBe('e.g. filesystem')
  })
  it('opens a help popover on click, closes on Escape with focus back, and stays inside a 240px viewport', async () => {
    await mount()
    const tip = document.querySelector<HTMLButtonElement>('button[aria-label="Help: Arguments"]')
    const run = document.querySelector<HTMLButtonElement>('button[aria-label="Help: How does it run?"]')!
    expect(tip).not.toBeNull()
    run.click()
    await vi.waitFor(() => expect(document.querySelector('[role="dialog"]')).not.toBeNull())
    const dlg = document.querySelector<HTMLElement>('[role="dialog"]')!
    expect(run.getAttribute('aria-expanded')).toBe('true')
    expect(run.getAttribute('aria-controls')).toBe(dlg.id)
    expect(dlg.textContent).toContain('stdio')
    const r = dlg.getBoundingClientRect()
    expect(r.left).toBeGreaterThanOrEqual(0)
    expect(r.right).toBeLessThanOrEqual(document.documentElement.clientWidth)
    const scroller = document.querySelector<HTMLElement>('.overflow-y-auto')!
    expect(scroller.scrollWidth).toBeLessThanOrEqual(scroller.clientWidth)
    expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(document.documentElement.clientWidth)
    await userEvent.keyboard('{Escape}')
    expect(document.querySelector('[role="dialog"]')).toBeNull()
    expect(document.activeElement).toBe(run)
    run.click()
    await vi.waitFor(() => expect(document.querySelector('[role="dialog"]')).not.toBeNull())
    run.click()
    await vi.waitFor(() => expect(document.querySelector('[role="dialog"]')).toBeNull())
  })
})
