import { StrictMode, useState } from 'react'
import { userEvent } from 'vitest/browser'
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { useKoma, type Tab } from '../../store/koma'
import { SkillsPanel } from './SkillsPanel'
import { SkillDuplicateDialog } from '../SkillDuplicateDialog'

const entries = Array.from({ length: 8 }, (_, index) => ({
  skillId: `opaque-${index}`,
  generation: `generation-${index}`,
  name: `skill-${index}`,
  description: `Description ${index}`,
  triggers: index % 2 ? 'review' : '',
  sourceTier: index === 7 ? 'extra-root' : 'global',
  scope: (index === 7 ? 'external' : 'global') as 'external' | 'global',
  sourcePath: `/skills/skill-${index}/SKILL.md`,
}))

let stylesheet: HTMLLinkElement
const req = vi.fn()
const realRefreshSkills = useKoma.getState().refreshSkills

const catalogueRequests = () => req.mock.calls.map(([request]) => request as { r: string; requestId: string; sessionEpoch: number }).filter((request) => request.r === 'GetSkills')

function switchSnapshot(session: string, loadedSkillNames: string[]) {
  useKoma.getState().push({
    k: 'Snapshot', session, loadedSkillNames, state: 'idle', messages: [], title: '',
    subagents: [], bash: [], fileChanges: [], attachments: [], pendingSteer: [],
    awaitingApproval: false, palette: useKoma.getState().palette,
  } as any)
}

function replyToCatalogue(request: { requestId: string; sessionEpoch: number }, loadedSkillNames: string[]) {
  useKoma.getState().push({
    k: 'SkillValues', requestId: request.requestId, sessionEpoch: request.sessionEpoch,
    skills: entries, loadedSkillNames, error: null,
  })
}

const nextPaint = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))

beforeAll(async () => {
  const built = await fetch('/dist/index.html').then((response) => response.text())
  const cssHref = built.match(/href="\.\/(assets\/index-[^"]+\.css)"/)?.[1]
  if (!cssHref) throw new Error('production CSS asset not found; run vite build before browser tests')
  stylesheet = document.createElement('link')
  stylesheet.rel = 'stylesheet'
  stylesheet.href = `/dist/${cssHref}`
  document.head.append(stylesheet)
  await new Promise<void>((resolve, reject) => {
    stylesheet.addEventListener('load', () => resolve(), { once: true })
    stylesheet.addEventListener('error', () => reject(new Error('failed to load Koma CSS')), { once: true })
  })
})

afterAll(() => stylesheet.remove())

beforeEach(() => {
  req.mockReset()
  const root = document.documentElement
  root.style.setProperty('--koma-bg', '#2e2a20')
  root.style.setProperty('--koma-fg', '#f1dca7')
  root.style.setProperty('--koma-dim', '#baa587')
  root.style.setProperty('--koma-accent', '#ffcb69')
  useKoma.setState((state) => ({
    session: { ...state.session, id: 'session-a' },
    ui: {
      ...state.ui,
      tabs: [{ id: 'chat', kind: 'chat' }],
      activeTabId: 'chat',
      groups: ['g0'],
      tabGroup: { chat: 'g0' },
      groupActive: { g0: 'chat' },
      activeGroupId: 'g0',
      groupSizes: { g0: 1 },
      preserveTabsOnNextSession: false,
      preservedTabsTargetSession: null,
      preservedTabLayout: null,
    },
    skills: entries,
    loadedSkillNames: ['skill-0'],
    skillsLoading: false,
    skillsError: null,
    skillQuery: '',
    skillFilter: 'all',
    skillSelection: [],
    refreshSkills: () => {},
    req,
  }))
})

function DuplicateDialogHarness() {
  const [open, setOpen] = useState(false)
  return <div><button type="button" onClick={() => setOpen(true)}>Open duplicate</button>{open && <SkillDuplicateDialog skills={[entries[7]]} onClose={() => setOpen(false)} />}</div>
}

async function mount(width: number) {
  const screen = await render(
    <div data-test-host style={{ width, height: 700, overflow: 'hidden' }}>
      <SkillsPanel />
    </div>,
  )
  const host = document.querySelector<HTMLElement>('[data-test-host]')!
  await new Promise((resolve) => requestAnimationFrame(resolve))
  return { screen, host }
}

describe.sequential('Skills panel browser contracts', () => {
  it.each([150, 220, 320, 500])('keeps the complete filter row visible at %ipx', async (width) => {
    const { host } = await mount(width)
    const hostRect = host.getBoundingClientRect()
    expect(host.scrollWidth).toBeLessThanOrEqual(width + 1)
    const search = host.querySelector<HTMLInputElement>('input[aria-label="Search skills"]')!
    expect(search.getBoundingClientRect().right).toBeLessThanOrEqual(hostRect.right + 1)

    const controls = [
      host.querySelector<HTMLButtonElement>('button[aria-pressed="true"]')!,
      [...host.querySelectorAll<HTMLButtonElement>('[aria-label="Skill filter"] button')].find((button) => button.textContent === 'Loaded in chat')!,
      host.querySelector<HTMLButtonElement>('button[aria-label="Rescan skill locations"]')!,
    ]
    const rects = controls.map((control) => control.getBoundingClientRect())
    for (const [index, rect] of rects.entries()) {
      expect(rect.width).toBeGreaterThan(0)
      expect(rect.left).toBeGreaterThanOrEqual(hostRect.left - 1)
      expect(rect.right).toBeLessThanOrEqual(hostRect.right + 1)
      if (index > 0) expect(rect.left).toBeGreaterThanOrEqual(rects[index - 1].right - 1)
    }
    expect(controls[1].scrollWidth).toBeLessThanOrEqual(controls[1].clientWidth + 1)
  })

  it('uses session-scoped labels and keeps enabled bulk actions inside a 150px panel', async () => {
    const { host } = await mount(150)
    const hostRect = host.getBoundingClientRect()
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]

    rows[0].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    const remove = [...host.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent === 'Remove from chat')!
    const reload = [...host.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent === 'Reload from disk')!
    expect(remove.disabled).toBe(false)
    expect(reload.disabled).toBe(false)
    for (const button of [remove, reload]) {
      const rect = button.getBoundingClientRect()
      expect(rect.width).toBeGreaterThan(0)
      expect(rect.left).toBeGreaterThanOrEqual(hostRect.left - 1)
      expect(rect.right).toBeLessThanOrEqual(hostRect.right + 1)
    }

    useKoma.getState().setSkillSelection([])
    rows[1].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    const load = [...host.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent === 'Load into chat')!
    expect(load.disabled).toBe(false)
    const loadRect = load.getBoundingClientRect()
    expect(loadRect.width).toBeGreaterThan(0)
    expect(loadRect.left).toBeGreaterThanOrEqual(hostRect.left - 1)
    expect(loadRect.right).toBeLessThanOrEqual(hostRect.right + 1)
  })

  it('explains detached discovery and removes the notice when a chat becomes active', async () => {
    useKoma.setState((state) => ({ session: { ...state.session, id: null } }))
    const { host } = await mount(320)
    const notice = host.querySelector<HTMLElement>('[data-testid="skills-no-active-chat"]')!
    expect(notice.textContent).toContain('Open a chat to discover Project skills and load skills into chat.')

    const loadedFilter = [...host.querySelectorAll<HTMLButtonElement>('[aria-label="Skill filter"] button')].find((button) => button.textContent === 'Loaded in chat')!
    expect(loadedFilter.disabled).toBe(true)
    expect(loadedFilter.title).toContain('Open a chat')
    expect(loadedFilter.getAttribute('aria-describedby')).toBe('skills-no-active-chat')

    const rows = host.querySelectorAll<HTMLButtonElement>('[role="option"]')
    rows[1].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    const load = [...host.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent === 'Load into chat')!
    const deleteButton = [...host.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent === 'Delete')!
    expect(load.disabled).toBe(true)
    expect(load.title).toContain('Open a chat')
    expect(load.getAttribute('aria-describedby')).toBe('skills-no-active-chat')
    expect(deleteButton.disabled).toBe(false)

    useKoma.getState().setSkillSelection([])
    rows[7].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    const duplicateButton = [...host.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent === 'Duplicate')!
    expect(duplicateButton.disabled).toBe(false)

    useKoma.setState((state) => ({ session: { ...state.session, id: 'session-b' } }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(host.querySelector('[data-testid="skills-no-active-chat"]')).toBeNull()
    expect(loadedFilter.disabled).toBe(false)
    expect(load.disabled).toBe(false)
  })

  it('labels refresh as a request-driven rescan', async () => {
    const { host } = await mount(320)
    const rescan = host.querySelector<HTMLButtonElement>('button[aria-label="Rescan skill locations"]')!
    expect(rescan.title).toBe('Rescan skill locations')
  })

  it('rediscovers Loaded rows for A→B→A without clicking Rescan while the panel stays mounted', async () => {
    useKoma.setState({ refreshSkills: realRefreshSkills })
    const { host, screen } = await mount(320)
    expect(catalogueRequests()).toHaveLength(1)
    const initial = catalogueRequests()[0]
    replyToCatalogue(initial, ['skill-0'])
    await screen.getByRole('button', { name: 'Loaded in chat' }).click()
    useKoma.getState().setSkillQuery('skill-')
    const rowIds = () => [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')].map((row) => row.dataset.skillId)
    expect(rowIds()).toEqual(['opaque-0'])

    switchSnapshot('session-b', ['skill-1'])
    await nextPaint()
    expect(catalogueRequests()).toHaveLength(2)
    // The previous chat's delayed reply cannot put its rows into B.
    replyToCatalogue(initial, ['skill-0'])
    replyToCatalogue(catalogueRequests()[1], ['skill-1'])
    await nextPaint()
    expect(rowIds()).toEqual(['opaque-1'])

    switchSnapshot('session-a', ['skill-0'])
    await nextPaint()
    expect(catalogueRequests()).toHaveLength(3)
    replyToCatalogue(catalogueRequests()[2], ['skill-0'])
    await nextPaint()
    expect(rowIds()).toEqual(['opaque-0'])
    expect(host.querySelector<HTMLButtonElement>('button[aria-pressed="true"]')?.textContent).toBe('Loaded in chat')
    expect(host.querySelector<HTMLInputElement>('input[aria-label="Search skills"]')?.value).toBe('skill-')

    switchSnapshot('session-empty', [])
    await nextPaint()
    expect(catalogueRequests()).toHaveLength(4)
    replyToCatalogue(catalogueRequests()[3], [])
    await nextPaint()
    expect(rowIds()).toEqual([])
    expect(useKoma.getState().loadedSkillNames).toEqual([])
  })

  it('defers guided discovery until the target and avoids duplicate requests under StrictMode', async () => {
    useKoma.setState({ refreshSkills: realRefreshSkills })
    const screen = await render(<StrictMode><SkillsPanel /></StrictMode>)
    await nextPaint()
    expect(catalogueRequests()).toHaveLength(1)
    replyToCatalogue(catalogueRequests()[0], ['skill-0'])

    useKoma.setState((state) => ({ ui: {
      ...state.ui,
      preserveTabsOnNextSession: true,
      preservedTabsTargetSession: null,
      tabs: [...state.ui.tabs, { id: 'skill:opaque-0', kind: 'skill', skillId: 'opaque-0', title: 'skill-0' }],
      tabGroup: { ...state.ui.tabGroup, 'skill:opaque-0': 'g0' },
    } }))
    switchSnapshot('unrelated-session', ['skill-1'])
    await nextPaint()
    expect(catalogueRequests()).toHaveLength(1)
    useKoma.getState().push({ k: 'Switching', to: 'guided-target' } as any)
    await nextPaint()
    expect(catalogueRequests()).toHaveLength(1)

    switchSnapshot('guided-target', ['skill-2'])
    await nextPaint()
    expect(catalogueRequests()).toHaveLength(2)
    replyToCatalogue(catalogueRequests()[1], ['skill-2'])
    await nextPaint()
    expect(useKoma.getState().loadedSkillNames).toEqual(['skill-2'])
    screen.unmount()
  })

  it('keeps detached Global/External discovery and allows manual retry after an unconfirmed reply', async () => {
    useKoma.setState((state) => ({
      session: { ...state.session, id: null },
      ui: { ...state.ui, preserveTabsOnNextSession: true, preservedTabsTargetSession: null },
      refreshSkills: realRefreshSkills,
    }))
    const { host } = await mount(320)
    expect(catalogueRequests()).toHaveLength(1)
    const first = catalogueRequests()[0]
    // The store's fake-timer test covers the 12-second transition. Here we
    // exercise the real panel control once that unconfirmed state is visible.
    useKoma.setState({ skillsLoading: false, skillsUnconfirmed: 'Skill catalogue response not confirmed.' })
    await nextPaint()
    host.querySelector<HTMLButtonElement>('button[aria-label="Rescan skill locations"]')!.click()
    expect(catalogueRequests()).toHaveLength(2)
    expect(catalogueRequests()[1].requestId).not.toBe(first.requestId)
    replyToCatalogue(first, ['skill-0'])
    expect(useKoma.getState().skillsLoading).toBe(true)
    replyToCatalogue(catalogueRequests()[1], [])
    expect(useKoma.getState().skillsLoading).toBe(false)
  })

  it('uses the selected Autumn foreground roles in list and Add views', async () => {
    const { host } = await mount(320)
    const search = host.querySelector<HTMLInputElement>('input[aria-label="Search skills"]')!
    const add = host.querySelector<HTMLButtonElement>('[data-tour="skills-add"]')!
    expect(getComputedStyle(search).color).toBe('rgb(241, 220, 167)')
    expect(getComputedStyle(add).color).toBe('rgb(241, 220, 167)')

    add.click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    const create = host.querySelector<HTMLButtonElement>('[data-tour="skills-add-methods"] button')!
    const description = create.querySelector<HTMLSpanElement>('span:last-child')!
    expect(getComputedStyle(create).color).toBe('rgb(241, 220, 167)')
    expect(getComputedStyle(description).color).toBe('rgb(186, 165, 135)')
  })

  it('shows all three Add Skill methods before opening an editor', async () => {
    const { screen } = await mount(320)
    expect(document.querySelector('[data-tour="skills-panel"]')).toBeTruthy()
    expect(document.querySelector('[data-tour="skills-search"]')).toBeTruthy()
    expect(document.querySelector('[data-tour="skills-filters"]')).toBeTruthy()
    expect(document.querySelector('[data-tour="skills-list"]')).toBeTruthy()
    await screen.getByRole('button', { name: 'Add skill' }).click()
    expect(document.querySelector('[data-tour="skills-add-methods"]')).toBeTruthy()
    expect(screen.getByRole('button', { name: /Create new/ })).toBeTruthy()
    expect(screen.getByRole('button', { name: /Upload \.zip/ })).toBeTruthy()
    expect(screen.getByRole('button', { name: /Create with Koma/ })).toBeTruthy()
  })

  it('reuses an active chat for Create with Koma so existing editor tabs are preserved', async () => {
    const tabs: Tab[] = [
      { id: 'chat', kind: 'chat' },
      { id: 'skill:new', kind: 'skill', skillId: null, title: 'New skill' },
      { id: 'upload-skill', kind: 'uploadSkill' },
    ]
    useKoma.setState((state) => ({
      ui: {
        ...state.ui,
        tabs,
        activeTabId: 'upload-skill',
        tabGroup: { chat: 'g0', 'skill:new': 'g0', 'upload-skill': 'g0' },
        groupActive: { g0: 'upload-skill' },
      },
    }))
    const { screen } = await mount(320)
    await screen.getByRole('button', { name: 'Add skill' }).click()
    await screen.getByRole('button', { name: /Create with Koma/ }).click()
    expect(req.mock.calls.some(([request]) => request.r === 'NewSession')).toBe(false)
    expect(useKoma.getState().ui.composerRefill).toContain('Help me create a new Koma skill.')
    expect(useKoma.getState().ui.activeTabId).toBe('chat')
    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual(['chat', 'skill:new', 'upload-skill'])
  })

  it('creates a chat for Create with Koma only when detached', async () => {
    useKoma.setState((state) => ({ session: { ...state.session, id: null } }))
    const { screen } = await mount(320)
    await screen.getByRole('button', { name: 'Add skill' }).click()
    await screen.getByRole('button', { name: /Create with Koma/ }).click()
    expect(req.mock.calls.filter(([request]) => request.r === 'NewSession')).toHaveLength(1)
    expect(useKoma.getState().ui.preserveTabsOnNextSession).toBe(true)
    expect(useKoma.getState().ui.composerRefill).toContain('Help me create a new Koma skill.')
  })

  it('enables Duplicate only for External selections', async () => {
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    rows[0].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(host.querySelector<HTMLButtonElement>('button[title*="Only External"]')?.disabled).toBe(true)
    useKoma.getState().setSkillSelection([])
    rows[7].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(host.querySelector<HTMLButtonElement>('button[title="Duplicate selected External skills"]')?.disabled).toBe(false)
  })

  it('clears bulk intent and marks the active skill when double-click opens it', async () => {
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    rows[0].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(rows[0].getAttribute('aria-selected')).toBe('true')
    expect(host.querySelector('[data-tour="skills-bulk"]')).toBeTruthy()

    rows[0].dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(useKoma.getState().skillSelection).toEqual([])
    expect(host.querySelector('[data-tour="skills-bulk"]')).toBeNull()
    expect(rows[0].getAttribute('aria-current')).toBe('true')
    expect(rows[0].textContent).toContain('Open')
    expect(rows[0].querySelector('svg')).toBeNull()

    useKoma.getState().activateTab('chat')
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(host.querySelector('[role="option"][aria-current="true"]')).toBeNull()
  })

  it('clears bulk intent when Enter opens the focused skill', async () => {
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    rows[1].click()
    rows[1].focus()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    rows[1].dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(useKoma.getState().skillSelection).toEqual([])
    expect(rows[1].getAttribute('aria-current')).toBe('true')
    expect(host.querySelector('[data-tour="skills-bulk"]')).toBeNull()
  })

  it('keeps Open separate from multi-selection without right-side checks', async () => {
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    rows[0].dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    rows[0].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    rows[1].dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))

    expect(useKoma.getState().skillSelection).toEqual(['opaque-0', 'opaque-1'])
    expect(rows[0].getAttribute('aria-current')).toBe('true')
    expect(rows[0].getAttribute('aria-selected')).toBe('true')
    expect(rows[1].getAttribute('aria-selected')).toBe('true')
    expect(host.querySelector('[data-tour="skills-bulk"]')?.textContent).toContain('2 selected')
    expect(host.querySelector('input[type="checkbox"]')).toBeNull()
    expect(rows[1].querySelector('svg')).toBeNull()
    expect(rows[7].querySelector('svg')).toBeTruthy()
  })

  it('shows a full-height theme accent for all three selected skills, distinct from Open', async () => {
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    rows[0].dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    rows[1].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    for (const row of [rows[2], rows[3]]) {
      row.dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }))
      await new Promise((resolve) => requestAnimationFrame(resolve))
    }

    expect(useKoma.getState().skillSelection).toEqual(['opaque-1', 'opaque-2', 'opaque-3'])
    expect(host.querySelector('[data-tour="skills-bulk"]')?.textContent).toContain('3 selected')
    const openMarker = rows[0].querySelector<HTMLElement>('[data-skill-marker="open"]')!
    expect(openMarker).toBeTruthy()
    expect(rows[0].textContent).toContain('Open')
    for (const row of rows.slice(1, 4)) {
      const marker = row.querySelector<HTMLElement>('[data-skill-marker="selected"]')!
      expect(marker).toBeTruthy()
      expect(marker.getBoundingClientRect().height).toBeGreaterThan(openMarker.getBoundingClientRect().height)
      expect(getComputedStyle(marker).backgroundColor).toBe(getComputedStyle(openMarker).backgroundColor)
      expect(row.className).toContain('bg-koma-head')
    }
    expect(rows[4].querySelector('[data-skill-marker]')).toBeNull()
    rows[3].dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(host.querySelectorAll('[data-skill-marker="selected"]')).toHaveLength(0)
    expect(rows[0].querySelector('[data-skill-marker="open"]')).toBeTruthy()
  })

  it('moves real DOM focus through the roving listbox with ArrowDown', async () => {
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    rows[0].focus()
    expect(document.activeElement).toBe(rows[0])
    rows[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(document.activeElement).toBe(rows[1])
    expect(rows[1].tabIndex).toBe(0)
  })

  it('advances focus one row per arrow in a four-skill list and keeps one tab stop', async () => {
    useKoma.setState({ skills: entries.slice(0, 4) })
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    expect(rows).toHaveLength(4)
    await userEvent.click(rows[0])
    expect(document.activeElement).toBe(rows[0])
    await userEvent.keyboard('{Escape}')
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(useKoma.getState().skillSelection).toEqual([])
    expect(document.activeElement).toBe(rows[0])
    const focusColor = getComputedStyle(rows[0]).outlineColor
    expect(focusColor).toMatch(/^rgb\(/)
    expect(getComputedStyle(rows[0]).outlineWidth).toBe('2px')
    for (let index = 1; index < rows.length; index++) {
      await userEvent.keyboard('{ArrowDown}')
      await new Promise((resolve) => requestAnimationFrame(resolve))
      expect(document.activeElement).toBe(rows[index])
      expect(rows.filter((row) => row.tabIndex === 0)).toEqual([rows[index]])
      expect(getComputedStyle(rows[index]).outlineStyle).toBe('solid')
      expect(getComputedStyle(rows[index]).outlineWidth).toBe('2px')
      expect(getComputedStyle(rows[index]).outlineColor).toBe(focusColor)
    }
    await userEvent.keyboard('{ArrowDown}')
    expect(document.activeElement).toBe(rows[3])
    await userEvent.keyboard('{ArrowUp}')
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(document.activeElement).toBe(rows[2])
    const search = host.querySelector<HTMLInputElement>('input[aria-label="Search skills"]')!
    search.focus()
    expect(document.activeElement).toBe(search)
    expect(getComputedStyle(rows[2]).outlineStyle).not.toBe('solid')
  })

  it('traps duplicate-dialog focus and restores the opener on Escape', async () => {
    const screen = await render(<DuplicateDialogHarness />)
    await screen.getByRole('button', { name: 'Open duplicate' }).click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!
    const focusable = [...dialog.querySelectorAll<HTMLButtonElement | HTMLInputElement>('button:not(:disabled), input:not(:disabled)')]
    const cancel = focusable.find((element) => element.textContent === 'Cancel')!
    expect(document.activeElement).toBe(cancel)
    const last = focusable[focusable.length - 1]
    last.focus()
    last.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }))
    expect(document.activeElement).toBe(focusable[0])
    document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(document.querySelector('[role="dialog"]')).toBeNull()
    expect(document.activeElement).toBe(document.querySelector('button'))
  })
})
