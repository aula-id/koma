import { describe, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'
import { render } from 'vitest-browser-react'
import '../../../styles.css'
import { McpEditView } from './McpEditView'
import { blankDraft } from './mcpForm'

function mount(draft = blankDraft(), width = 1000, isNew = true) {
  return render(
    <div data-testid="host" style={{ width, height: 600, display: 'flex', flexDirection: 'column' }} className="bg-koma-bg">
      <McpEditView draft={draft} isNew={isNew} onChange={vi.fn()} onSave={vi.fn()} onCancel={vi.fn()} />
    </div>,
  )
}
const toggles = () => [...document.querySelectorAll<HTMLButtonElement>('button[aria-expanded]:not([aria-label^="Help:"])')]
const scroller = () => document.querySelector<HTMLElement>('.overflow-y-auto')!
const noOverflow = () => {
  expect(scroller().scrollWidth).toBeLessThanOrEqual(scroller().clientWidth)
  expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(document.documentElement.clientWidth)
}
const save = () => [...document.querySelectorAll('button')].find((b) => b.textContent === 'Save')!
const segmented = () => [...document.querySelectorAll<HTMLElement>('button[aria-pressed]')]

describe('McpEditView tab layout', () => {
  it('at 1000px keeps the segmented choice on one line, never truncates titles, and Save stays visible', async () => {
    await mount({ ...blankDraft(), envText: 'A=1', toolSec: 'abc', allowedText: 'x' })
    const seg = segmented()
    expect(seg).toHaveLength(2)
    expect(Math.abs(seg[0].getBoundingClientRect().top - seg[1].getBoundingClientRect().top)).toBeLessThan(2)
    for (const b of seg) expect(b.getBoundingClientRect().height).toBeLessThan(32)
    for (const b of toggles()) {
      const title = b.querySelector<HTMLElement>('span > span')!
      expect(title.scrollWidth).toBeLessThanOrEqual(title.clientWidth)
      expect(getComputedStyle(title).textOverflow).not.toBe('ellipsis')
    }
    expect(document.body.textContent).toContain('Working directory and timeouts')
    // two columns at this width
    const [a, b] = [toggles()[0], toggles()[1]].map((t) => t.closest('section')!.getBoundingClientRect())
    expect(Math.abs(a.top - b.top)).toBeLessThan(2)
    noOverflow()
    const host = document.querySelector('[data-testid="host"]')!.getBoundingClientRect()
    const r = save().getBoundingClientRect()
    expect(r.bottom).toBeLessThanOrEqual(host.bottom)
    expect(r.top).toBeGreaterThanOrEqual(host.top)
  })

  it('at 420px stacks the sections in one column, no horizontal overflow, Save visible after scrolling', async () => {
    await mount({ ...blankDraft(), envText: 'A=1', toolSec: 'abc', allowedText: 'x' }, 420)
    for (const b of toggles()) if (b.getAttribute('aria-expanded') === 'false') await userEvent.click(b)
    const [a, b] = [toggles()[0], toggles()[1]].map((t) => t.closest('section')!.getBoundingClientRect())
    expect(b.top).toBeGreaterThan(a.bottom - 1)
    noOverflow()
    scroller().scrollTop = scroller().scrollHeight
    const host = document.querySelector('[data-testid="host"]')!.getBoundingClientRect()
    expect(save().getBoundingClientRect().bottom).toBeLessThanOrEqual(host.bottom)
    expect(save().getBoundingClientRect().top).toBeGreaterThanOrEqual(host.top)
  })

  it('shows a summary and opens the group when it holds values', async () => {
    await mount({ ...blankDraft(), envText: 'A=1\nB=2', toolSec: 'abc' })
    expect(document.body.textContent).toContain('2 variables')
    expect(toggles().find((b) => b.textContent?.includes('Environment'))!.getAttribute('aria-expanded')).toBe('true')
    expect(toggles().find((b) => b.textContent?.includes('timeouts'))!.getAttribute('aria-expanded')).toBe('true')
    expect(document.body.textContent).toContain('Enter a number above 0')
    const tools = toggles().find((b) => b.textContent?.includes('Tool access'))!
    expect(tools.getAttribute('aria-expanded')).toBe('false')
    expect(tools.textContent).toContain('all allowed')
  })

  it('starts collapsed when empty, ties labels and the switch to their controls, focuses Name on a new server', async () => {
    await mount()
    expect(document.activeElement?.getAttribute('placeholder')).toBe('e.g. filesystem')
    expect(toggles().every((b) => b.getAttribute('aria-expanded') === 'false')).toBe(true)
    for (const b of toggles()) await userEvent.click(b)
    const inputs = [...document.querySelectorAll<HTMLElement>('input, textarea')]
    expect(inputs.length).toBeGreaterThan(4)
    for (const el of inputs) expect(document.querySelector(`label[for="${el.id}"]`)).not.toBeNull()
    expect(document.querySelector('[role="switch"]')?.getAttribute('aria-labelledby')).toBeTruthy()
    expect(document.querySelector('h1')?.textContent).toBe('New MCP server')
  })

  it('shows the server name and status for an existing server and does not steal focus', async () => {
    const server = { id: 'u1', name: 'files', enabled: true, transport: 'stdio', command: 'npx', args: '', env: '', url: '', toolCount: 3, connected: true } as never
    await render(
      <div style={{ width: 1000, height: 600, display: 'flex', flexDirection: 'column' }}>
        <McpEditView draft={{ ...blankDraft(), name: 'files' }} isNew={false} server={server} onChange={vi.fn()} onSave={vi.fn()} onCancel={vi.fn()} />
      </div>,
    )
    expect(document.querySelector('h1')?.textContent).toContain('files')
    expect(document.querySelector('[data-testid="mcp-status"]')?.textContent).toContain('Connected, 3 tools')
    expect(document.activeElement?.tagName).not.toBe('INPUT')
  })

  it('opens a help popover on click, closes on Escape with focus back, and stays inside a 420px viewport', async () => {
    await mount(blankDraft(), 420)
    const run = document.querySelector<HTMLButtonElement>('button[aria-label="Help: How does it run?"]')!
    expect(document.querySelector('button[aria-label="Help: Arguments"]')).not.toBeNull()
    run.click()
    await vi.waitFor(() => expect(document.querySelector('[role="dialog"]')).not.toBeNull())
    const dlg = document.querySelector<HTMLElement>('[role="dialog"]')!
    expect(run.getAttribute('aria-expanded')).toBe('true')
    expect(run.getAttribute('aria-controls')).toBe(dlg.id)
    expect(dlg.textContent).toContain('stdio')
    const r = dlg.getBoundingClientRect()
    expect(r.left).toBeGreaterThanOrEqual(0)
    expect(r.right).toBeLessThanOrEqual(document.documentElement.clientWidth)
    noOverflow()
    await userEvent.keyboard('{Escape}')
    expect(document.querySelector('[role="dialog"]')).toBeNull()
    expect(document.activeElement).toBe(run)
  })
})
