import '../styles.css'
import { page, userEvent } from 'vitest/browser'
import { beforeEach, afterEach, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { ChatView } from '../components/ChatView'
import { BottomPanel } from '../components/BottomPanel'
import { TerminalTab } from '../components/TerminalTab'
import SettingsTab, { UiScaleSetting } from '../components/SettingsTab'
import { GlobalContextMenu } from '../components/GlobalContextMenu'
import { canvasPoint, setUiScale, useUiScale, UI_SCALES } from './uiScale'
import { useKoma } from '../store/koma'

beforeEach(async () => { await page.viewport(1280, 900) })
const paint = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
afterEach(async () => { await setUiScale(1); localStorage.clear() })

async function pickScale(screen: Awaited<ReturnType<typeof render>>, scale: number) {
  const slider = screen.getByRole('slider', { name: 'UI scale' })
  const idx = UI_SCALES.indexOf(scale as (typeof UI_SCALES)[number])
  expect(idx).toBeGreaterThanOrEqual(0)
  const el = slider.element() as HTMLInputElement
  // React ignores plain .value= on controlled inputs — poke the native setter.
  const setNative = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
  setNative?.call(el, String(idx))
  el.dispatchEvent(new Event('input', { bubbles: true }))
  el.dispatchEvent(new Event('change', { bubbles: true }))
  await paint()
  await screen.getByRole('button', { name: 'Apply' }).click()
  await paint()
}

it('scales text, icons, spacing and controls exactly once at every multiplier', async () => {
  const screen = await render(<div data-testid="sample" style={{ width: 200, height: 100, padding: 10, fontSize: 12 }}>
    <svg width="16" height="16"><rect width="16" height="16" /></svg><UiScaleSetting />
  </div>)
  const sample = screen.getByTestId('sample').element()
  const baseline = sample.getBoundingClientRect()
  const icon = sample.querySelector('svg')!
  const baseIcon = icon.getBoundingClientRect().width
  for (const scale of UI_SCALES) {
    await pickScale(screen, scale)
    expect(sample.getBoundingClientRect().width).toBeCloseTo(baseline.width * scale, 0)
    expect(sample.getBoundingClientRect().height).toBeCloseTo(baseline.height * scale, 0)
    expect(icon.getBoundingClientRect().width).toBeCloseTo(baseIcon * scale, 0)
    expect(getComputedStyle(sample).fontSize).toBe('12px')
    await setUiScale(scale)
    expect(sample.getBoundingClientRect().width).toBeCloseTo(baseline.width * scale, 0)
  }
  await setUiScale(1)
  expect(sample.getBoundingClientRect().width).toBe(baseline.width)
})

it('keeps Settings and its scale selector reachable at 1.5× across session switches', async () => {
  useKoma.setState({ req: vi.fn() })
  await render(<div style={{ width: '100%', height: '100%' }}><SettingsTab /></div>)
  await setUiScale(1.5)
  await paint()
  const group = document.querySelector('[aria-label="UI scale"]')!
  group.scrollIntoView({ block: 'center' })
  await paint()
  const rect = group.getBoundingClientRect()
  expect(rect.right).toBeLessThanOrEqual(window.innerWidth + 1)
  expect(rect.top).toBeGreaterThanOrEqual(0)
  expect(rect.bottom).toBeLessThanOrEqual(window.innerHeight)
  const slider = group.querySelector('input[type="range"]')!
  expect(slider.getBoundingClientRect().width).toBeGreaterThan(40)
  useKoma.getState().push({ k: 'Snapshot', session: 'another-session', state: 'idle', messages: [], title: '', subagents: [], bash: [], fileChanges: [], attachments: [], pendingSteer: [], awaitingApproval: false, palette: useKoma.getState().palette } as any)
  expect(useUiScale.getState().scale).toBe(1.5)
})

it('positions a context menu at the pointer and keeps it inside the zoomed viewport', async () => {
  const screen = await render(<><div data-testid="target">Target</div><GlobalContextMenu onResume={() => {}} /></>)
  await setUiScale(1.5)
  await paint()
  screen.getByTestId('target').element().dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 20, clientY: 30 }))
  await paint()
  const menu = document.querySelector('[role="menu"]')!
  const rect = menu.getBoundingClientRect()
  expect(rect.left).toBeCloseTo(20, 0)
  expect(rect.top).toBeCloseTo(30, 0)
  expect(rect.right).toBeLessThanOrEqual(window.innerWidth)
  expect(rect.bottom).toBeLessThanOrEqual(window.innerHeight)
})

it('keeps the guide cutout and popover aligned with a zoomed target', async () => {
  const { driver } = await import('driver.js')
  await import('driver.js/dist/driver.css')
  const screen = await render(<button data-testid="guide-target" style={{ marginLeft: 50, marginTop: 40 }}>Guide target</button>)
  const target = screen.getByTestId('guide-target').element()
  for (const scale of [1, 1.2, 1.5] as const) {
    await setUiScale(scale)
    const guide = driver({ animate: false, steps: [{ element: target, popover: { title: 'Guide', description: 'Target', side: 'bottom' } }] })
    guide.drive()
    await paint()
    const overlay = document.querySelector('.driver-overlay')!.getBoundingClientRect()
    expect(overlay.width).toBeCloseTo(window.innerWidth, 0)
    expect(overlay.height).toBeCloseTo(window.innerHeight, 0)
    const popover = document.querySelector('.driver-popover')!.getBoundingClientRect()
    expect(popover.left).toBeGreaterThanOrEqual(0)
    expect(popover.right).toBeLessThanOrEqual(window.innerWidth)
    expect(popover.top).toBeGreaterThanOrEqual(target.getBoundingClientRect().bottom)
    expect(popover.bottom).toBeLessThanOrEqual(window.innerHeight)
    guide.destroy()
  }
})

it('refits an existing terminal and preserves its connection and input at 1.5×', async () => {
  const req = vi.fn()
  const tab = { id: 'scale-terminal', kind: 'terminal' as const, terminalId: 'scale-terminal', title: 'Terminal' }
  useKoma.setState(state => ({ req, ui: { ...state.ui, tabs: [tab], activeTabId: tab.id, groups: ['g0'], tabGroup: { [tab.id]: 'g0' }, groupActive: { g0: tab.id }, activeGroupId: 'g0' } }))
  await render(<div style={{ width: '100%', height: '100%' }}><TerminalTab tab={tab} /></div>)
  await paint()
  const terminal = document.querySelector('.xterm')!
  const write = (globalThis as any).__terminalWriters[tab.terminalId]
  write('terminal output retained\r\n')
  const before = req.mock.calls.filter(([request]) => request.r === 'TerminalResize').slice(-1)[0][0]
  await setUiScale(1.5)
  await paint()
  const after = req.mock.calls.filter(([request]) => request.r === 'TerminalResize').slice(-1)[0][0]
  expect(after.cols).toBeLessThan(before.cols)
  expect(document.querySelector('.xterm')).toBe(terminal)
  expect((globalThis as any).__terminalWriters[tab.terminalId]).toBe(write)
  await userEvent.click(document.querySelector('.xterm-helper-textarea')!)
  await userEvent.keyboard('hello')
  expect(req.mock.calls.filter(([request]) => request.r === 'TerminalInput').map(([request]) => request.data).join('')).toBe('hello')
  await setUiScale(1)
  await paint()
  expect(document.querySelector('.xterm')).toBe(terminal)
  expect(req.mock.calls.some(([request]) => request.r === 'TerminalKill')).toBe(false)
})

it('preserves Monaco drafts, selection and font sizes while zooming and typing', async () => {
  const { initMonaco } = await import('./monaco-setup')
  const monaco = await import('monaco-editor/esm/vs/editor/editor.api')
  initMonaco()
  const screen = await render(<div data-testid="editor" style={{ width: '100%', height: '100%' }} />)
  const editor = monaco.editor.create(screen.getByTestId('editor').element() as HTMLElement, {
    occurrencesHighlight: 'off', selectionHighlight: false, value: 'draft text\nsecond line', fontSize: 12, automaticLayout: true, minimap: { enabled: false },
  })
  try {
    editor.setSelection(new monaco.Selection(1, 1, 1, 6))
    const selection = editor.getSelection()
    await setUiScale(1.5)
    await paint()
    editor.layout()
    expect(editor.getValue()).toBe('draft text\nsecond line')
    expect(editor.getSelection()).toEqual(selection)
    expect(editor.getOption(monaco.editor.EditorOption.fontSize)).toBe(12)
    editor.focus()
    await userEvent.keyboard('saved')
    expect(editor.getValue()).toBe('saved text\nsecond line')
    await setUiScale(1)
    await paint()
    expect(editor.getValue()).toBe('saved text\nsecond line')
  } finally { editor.dispose() }
})

it('keeps notification search, confirmation and chat content usable at 1.5×', async () => {
  const { default: NotificationsTab } = await import('../components/NotificationsTab')
  const { receiveNotifications } = await import('./notifications')
  const req = vi.fn()
  useKoma.setState(state => ({ req, session: { ...state.session, id: 'zoom-chat', working: false, stream: '', messages: [{ role: 'user', content: 'Chat content retained', reasoning: null }] } }))
  receiveNotifications({ id: 'zoom-notice', session: 'zoom-chat', error: null, entries: [{ id: 'notice', timestamp: 1, severity: 'error', source: 'test', message: 'Zoom notification', read: false }] })
  const screen = await render(<div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
    <div style={{ flex: 1, minHeight: 0 }}><NotificationsTab /></div>
    <div style={{ flex: 1, minHeight: 0 }}><ChatView /></div>
  </div>)
  await setUiScale(1.5)
  await paint()
  await screen.getByRole('textbox', { name: 'Search notifications' }).fill('Zoom')
  await screen.getByRole('button', { name: 'Clear history', exact: true }).click()
  await screen.getByRole('button', { name: 'Confirm clear', exact: true }).click()
  expect(req.mock.calls.some(([r]) => r.request?.operation.op === 'clear')).toBe(true)
  expect(document.body.textContent).toContain('Chat content retained')
  const search = screen.getByRole('textbox', { name: 'Search notifications' }).element().getBoundingClientRect()
  expect(search.right).toBeLessThanOrEqual(window.innerWidth)
})

it('uses scaled pointer deltas for a workspace drag handle', async () => {
  useKoma.setState({ bottomPanelTab: 'tasks', req: vi.fn() })
  await setUiScale(1.5)
  await render(<div style={{ width: 500, height: 320 }}><BottomPanel /></div>)
  await paint()
  const handle = document.querySelector<HTMLElement>('[aria-label="Resize workspace panel"]')!
  const panel = document.querySelector('#workspace-bottom-panel')!
  const before = panel.getBoundingClientRect().height
  handle.setPointerCapture = () => {}
  handle.hasPointerCapture = () => false
  handle.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1, button: 0, clientY: 100 }))
  await paint()
  handle.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, pointerId: 1, clientY: 150 }))
  await paint()
  handle.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1 }))
  expect(panel.getBoundingClientRect().height).toBeCloseTo(before - 50, 0)
})

it('leaves designer and diagram artwork at its original visual scale while scaling chrome', async () => {
  const screen = await render(<div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
    <UiScaleSetting />
    {['designer', 'diagram'].map(name => <div key={name} style={{ display: 'flex', flex: 1, minHeight: 0 }}>
      <div className="koma-authored-canvas" style={{ flex: 1 }}><div data-testid={name} style={{ width: 200, height: 100, fontSize: 12 }}>Authored content</div></div>
    </div>)}
  </div>)
  for (const scale of [1, 1.2, 1.5, 1] as const) {
    await setUiScale(scale)
    await paint()
    for (const name of ['designer', 'diagram']) {
      const artwork = screen.getByTestId(name).element().getBoundingClientRect()
      expect(artwork.width).toBeCloseTo(200, 0)
      expect(artwork.height).toBeCloseTo(100, 0)
      expect(canvasPoint(artwork.width)).toBeCloseTo(200, 0)
    }
  }
})

it('does not apply scale until Apply is pressed', async () => {
  const screen = await render(<div data-testid="sample" style={{ width: 200, height: 100 }}><UiScaleSetting /></div>)
  const sample = screen.getByTestId('sample').element()
  const baseline = sample.getBoundingClientRect().width
  const slider = screen.getByRole('slider', { name: 'UI scale' }).element() as HTMLInputElement
  const idx = UI_SCALES.indexOf(1.5)
  const setNative = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
  setNative?.call(slider, String(idx))
  slider.dispatchEvent(new Event('input', { bubbles: true }))
  slider.dispatchEvent(new Event('change', { bubbles: true }))
  await paint()
  expect(useUiScale.getState().scale).toBe(1)
  expect(sample.getBoundingClientRect().width).toBeCloseTo(baseline, 0)
  // Draft preview updates immediately (Lorem sample at draft absolute size).
  const preview = screen.getByTestId('ui-scale-preview').element()
  expect(preview.textContent).toMatch(/Lorem ipsum/)
  expect(getComputedStyle(preview).fontSize).toBe('18px') // 12 * 1.5
  await screen.getByRole('button', { name: 'Apply' }).click()
  await paint()
  expect(useUiScale.getState().scale).toBe(1.5)
  expect(sample.getBoundingClientRect().width).toBeCloseTo(baseline * 1.5, 0)
  // After apply, draft===scale so preview is back to 12px CSS (page zoom carries the rest).
  expect(getComputedStyle(preview).fontSize).toBe('12px')
})

it('renders full-width slider with a snap circle per scale step', async () => {
  const screen = await render(<div style={{ width: 640 }}><UiScaleSetting /></div>)
  const group = screen.getByRole('group', { name: 'UI scale' }).element()
  const slider = screen.getByRole('slider', { name: 'UI scale' }).element()
  const snaps = group.querySelectorAll('.ui-scale-snap')
  expect(snaps.length).toBe(UI_SCALES.length)
  // Slider spans the content width (no max-w-sm clamp).
  expect(slider.getBoundingClientRect().width).toBeGreaterThan(280)
  expect(screen.getByTestId('ui-scale-preview').element().textContent).toMatch(/Lorem ipsum/)
})
