import { StrictMode, useState } from 'react'
import { userEvent } from 'vitest/browser'
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { useKoma, type SettingsValues, type Tab } from '../../store/koma'
import { SkillsPanel } from './SkillsPanel'
import { Sidebar } from '../Sidebar'
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
    skillFilter: 'global',
    skillSelection: [],
    refreshSkills: () => {},
    req,
    settingsValues: {
      name: 'Browser test',
      workdir: ['/work'],
      shortSend: false,
      slidingCache: false,
      bashSaving: false,
      codingAutosave: false,
      internetMode: 'simple',
      palette: 'dark',
      effort: '',
      subagentMaxTurns: 500,
      shortSendEngageN: 0,
      shortSendTailN: 0,
      maxOutputTokens: 0,
      contextWindowLimit: 0,
      contextModelAlias: '',
      extraSkillRoots: [],
    } satisfies SettingsValues,
    coding: { ...state.coding, activeRoot: '/work' },
  }))
})

function DuplicateDialogHarness() {
  const [open, setOpen] = useState(false)
  return <div><button type="button" onClick={() => setOpen(true)}>Open duplicate</button>{open && <SkillDuplicateDialog skills={[entries[7]]} onClose={() => setOpen(false)} />}</div>
}

function openSkillMenu(row: HTMLButtonElement) {
  const rect = row.getBoundingClientRect()
  row.dispatchEvent(new MouseEvent('contextmenu', {
    bubbles: true,
    cancelable: true,
    clientX: rect.left + 8,
    clientY: rect.top + 12,
  }))
}

function skillMenu() {
  return document.querySelector<HTMLElement>('[data-tour="skills-context-menu"]')
}

function menuButton(name: string) {
  return [...(skillMenu()?.querySelectorAll<HTMLButtonElement>('button') ?? [])].find((button) => button.textContent === name)
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

    const filters = host.querySelector<HTMLElement>('[aria-label="Skill filter"]')!
    const controls = [...filters.querySelectorAll<HTMLButtonElement>('button')]
    expect(controls.map((button) => button.textContent)).toEqual(['Global', 'Project'])
    const filterRect = filters.getBoundingClientRect()
    expect(filterRect.left).toBeGreaterThanOrEqual(hostRect.left - 1)
    expect(filterRect.right).toBeLessThanOrEqual(hostRect.right + 1)
    expect(controls[0].getAttribute('aria-pressed')).toBe('true')
    expect(controls[1].getBoundingClientRect().left).toBeGreaterThanOrEqual(controls[0].getBoundingClientRect().right - 1)
    expect(host.querySelector('button[aria-label="Rescan skill locations"]')).toBeNull()
  })

  it('opens session actions from a right-click and keeps the menu inside the viewport', async () => {
    const { host } = await mount(150)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]

    openSkillMenu(rows[0])
    await new Promise((resolve) => requestAnimationFrame(resolve))
    const menu = skillMenu()!
    const remove = menuButton('Remove from chat')!
    const reload = menuButton('Reload from disk')!
    expect(remove.disabled).toBe(false)
    expect(reload.disabled).toBe(false)
    const menuRect = menu.getBoundingClientRect()
    expect(menuRect.width).toBeGreaterThan(0)
    expect(menuRect.left).toBeGreaterThanOrEqual(0)
    expect(menuRect.right).toBeLessThanOrEqual(window.innerWidth)
    expect(host.querySelector('[data-tour="skills-context-menu"]')).toBeNull()

    openSkillMenu(rows[1])
    await new Promise((resolve) => requestAnimationFrame(resolve))
    const load = menuButton('Load into chat')!
    expect(load.disabled).toBe(false)
    expect(load.getBoundingClientRect().width).toBeGreaterThan(0)
  })

  it('explains detached discovery and removes the notice when a chat becomes active', async () => {
    useKoma.setState((state) => ({ session: { ...state.session, id: null } }))
    const { host } = await mount(320)
    const notice = host.querySelector<HTMLElement>('[data-testid="skills-no-active-chat"]')!
    expect(notice.textContent).toContain('Open a chat to discover Project skills and load skills into chat.')

    const projectTab = [...host.querySelectorAll<HTMLButtonElement>('[aria-label="Skill filter"] button')].find((button) => button.textContent === 'Project')!
    expect(projectTab.disabled).toBe(true)
    expect(projectTab.title).toContain('Open a chat')
    expect(projectTab.getAttribute('aria-describedby')).toBe('skills-no-active-chat')

    const rows = host.querySelectorAll<HTMLButtonElement>('[role="option"]')
    openSkillMenu(rows[1])
    await new Promise((resolve) => requestAnimationFrame(resolve))
    const load = menuButton('Load into chat')!
    const deleteButton = menuButton('Delete')!
    expect(load.disabled).toBe(true)
    expect(load.title).toContain('Open a chat')
    expect(load.getAttribute('aria-describedby')).toBe('skills-no-active-chat')
    expect(deleteButton.disabled).toBe(false)
    expect(host.querySelector<HTMLButtonElement>('[data-tour="skills-add"]')?.disabled).toBe(true)

    openSkillMenu(rows[7])
    await new Promise((resolve) => requestAnimationFrame(resolve))
    const duplicateButton = menuButton('Duplicate')!
    expect(duplicateButton.disabled).toBe(false)

    useKoma.setState((state) => ({ session: { ...state.session, id: 'session-b' } }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(host.querySelector('[data-testid="skills-no-active-chat"]')).toBeNull()
    expect(projectTab.disabled).toBe(false)
    expect(menuButton('Load into chat')?.disabled).toBe(false)
  })

  it('labels refresh as a request-driven rescan in the Skills header', async () => {
    const screen = await render(
      <div style={{ width: 320, height: 700 }}>
        <Sidebar width={320} view="skills" />
      </div>,
    )
    const rescan = document.querySelector<HTMLButtonElement>('button[aria-label="Rescan skill locations"]')!
    expect(rescan.title).toBe('Rescan skill locations')
    expect(rescan.closest('.uppercase')).toBeTruthy()
    screen.unmount()
  })

  it('rediscovers Loaded rows for A→B→A without clicking Rescan while the panel stays mounted', async () => {
    useKoma.setState({ refreshSkills: realRefreshSkills })
    const { host } = await mount(320)
    expect(catalogueRequests()).toHaveLength(1)
    const initial = catalogueRequests()[0]
    replyToCatalogue(initial, ['skill-0'])
    useKoma.getState().setSkillQuery('skill-')
    await nextPaint()
    const rowIds = () => [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')].map((row) => row.dataset.skillId)
    expect(rowIds()[0]).toBe('opaque-0')
    expect(host.querySelector('[data-skill-id="opaque-0"] [data-skill-marker="loaded"]')?.textContent).toBe('active')

    switchSnapshot('session-b', ['skill-1'])
    await nextPaint()
    expect(catalogueRequests()).toHaveLength(2)
    // The previous chat's delayed reply cannot put its rows into B.
    replyToCatalogue(initial, ['skill-0'])
    replyToCatalogue(catalogueRequests()[1], ['skill-1'])
    await nextPaint()
    expect(rowIds()[0]).toBe('opaque-1')
    expect(host.querySelector('[data-skill-id="opaque-1"] [data-skill-marker="loaded"]')).toBeTruthy()

    switchSnapshot('session-a', ['skill-0'])
    await nextPaint()
    expect(catalogueRequests()).toHaveLength(3)
    replyToCatalogue(catalogueRequests()[2], ['skill-0'])
    await nextPaint()
    expect(rowIds()[0]).toBe('opaque-0')
    expect(host.querySelector<HTMLButtonElement>('button[aria-pressed="true"]')?.textContent).toBe('Global')
    expect(host.querySelector<HTMLInputElement>('input[aria-label="Search skills"]')?.value).toBe('skill-')

    switchSnapshot('session-empty', [])
    await nextPaint()
    expect(catalogueRequests()).toHaveLength(4)
    replyToCatalogue(catalogueRequests()[3], [])
    await nextPaint()
    expect(rowIds().length).toBeGreaterThan(0)
    expect(host.querySelector('[data-skill-marker="loaded"]')).toBeNull()
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
    const screen = await render(
      <div style={{ width: 320, height: 700 }}>
        <Sidebar width={320} view="skills" />
      </div>,
    )
    expect(catalogueRequests()).toHaveLength(1)
    const first = catalogueRequests()[0]
    // The store's fake-timer test covers the 12-second transition. Here we
    // exercise the header refresh once that unconfirmed state is visible.
    useKoma.setState({ skillsLoading: false, skillsUnconfirmed: 'Skill catalogue response not confirmed.' })
    await nextPaint()
    document.querySelector<HTMLButtonElement>('button[aria-label="Rescan skill locations"]')!.click()
    expect(catalogueRequests()).toHaveLength(2)
    expect(catalogueRequests()[1].requestId).not.toBe(first.requestId)
    replyToCatalogue(first, ['skill-0'])
    expect(useKoma.getState().skillsLoading).toBe(true)
    replyToCatalogue(catalogueRequests()[1], [])
    expect(useKoma.getState().skillsLoading).toBe(false)
    screen.unmount()
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
    expect(getComputedStyle(description).color).toBe('rgb(241, 220, 167)')
    expect(getComputedStyle(description).opacity).toBe('0.4')
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

  it('disables Add skill outside a project and still opens a global skill', async () => {
    useKoma.setState((state) => ({
      session: { ...state.session, id: 'session-a' },
      settingsValues: state.settingsValues ? { ...state.settingsValues, workdir: [] } : null,
      coding: { ...state.coding, activeRoot: null },
    }))
    const { host } = await mount(320)
    const add = host.querySelector<HTMLButtonElement>('[data-tour="skills-add"]')!
    expect(add.disabled).toBe(true)
    expect(add.title).toContain('Open a project')
    add.click()
    expect(document.querySelector('[data-tour="skills-add-methods"]')).toBeNull()
    expect(req.mock.calls.some(([request]) => request.r === 'NewSession')).toBe(false)

    const row = host.querySelector<HTMLButtonElement>('[role="option"]')!
    row.click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(useKoma.getState().ui.tabs.some((tab) => tab.kind === 'skill' && tab.skillId === 'opaque-0')).toBe(true)
  })

  it('enables Duplicate only for an External right-click', async () => {
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    openSkillMenu(rows[0])
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(menuButton('Duplicate')?.disabled).toBe(true)
    expect(menuButton('Duplicate')?.title).toContain('Only External')
    openSkillMenu(rows[7])
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(menuButton('Duplicate')?.disabled).toBe(false)
    expect(menuButton('Duplicate')?.title).toBe('Duplicate selected External skills')
  })

  it('opens a skill on click and clears a Ctrl selection', async () => {
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    rows[0].dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    openSkillMenu(rows[0])
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(rows[0].getAttribute('aria-selected')).toBe('true')
    expect(skillMenu()?.textContent).toContain('skill-0')

    rows[0].dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    rows[0].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(useKoma.getState().skillSelection).toEqual([])
    expect(skillMenu()).toBeNull()
    expect(rows[0].getAttribute('aria-current')).toBe('true')
    expect(rows[0].textContent).not.toContain('Open')
    expect(rows[0].textContent).not.toMatch(/global|project|external/i)
    expect(rows[0].querySelector('svg')).toBeNull()

    useKoma.getState().activateTab('chat')
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(host.querySelector('[role="option"][aria-current="true"]')).toBeNull()
  })

  it('opens the focused skill on Enter without a selection chrome', async () => {
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    rows[1].focus()
    rows[1].dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(useKoma.getState().skillSelection).toEqual([])
    expect(rows[1].getAttribute('aria-current')).toBe('true')
    expect(skillMenu()).toBeNull()
    expect(host.querySelector('[data-tour="skills-add"]')).toBeTruthy()
  })

  it('keeps Open separate from a Ctrl multi-selection and offers it on right-click', async () => {
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    rows[0].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    rows[0].dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    rows[1].dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))

    expect(useKoma.getState().skillSelection).toEqual(['opaque-0', 'opaque-1'])
    expect(rows[0].getAttribute('aria-current')).toBe('true')
    expect(rows[0].getAttribute('aria-selected')).toBe('true')
    expect(rows[1].getAttribute('aria-selected')).toBe('true')
    expect(rows[0].textContent).not.toContain('Open')
    openSkillMenu(rows[0])
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(skillMenu()?.textContent).toContain('2 selected')
    expect(host.querySelector('input[type="checkbox"]')).toBeNull()
    expect(rows[1].querySelector('svg')).toBeNull()
    expect(rows[7].querySelector('svg')).toBeNull()
  })

  it('tints Ctrl-selected rows with the panel header color and draws no accent bar', async () => {
    const { host } = await mount(320)
    const rows = [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')]
    rows[0].click()
    await new Promise((resolve) => requestAnimationFrame(resolve))
    for (const row of [rows[1], rows[2], rows[3]]) {
      row.dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }))
      await new Promise((resolve) => requestAnimationFrame(resolve))
    }

    expect(useKoma.getState().skillSelection).toEqual(['opaque-1', 'opaque-2', 'opaque-3'])
    expect(rows[0].getAttribute('aria-current')).toBe('true')
    expect(rows[0].textContent).not.toContain('Open')
    for (const row of rows.slice(1, 4)) {
      expect(row.className).toContain('bg-koma-head')
      expect(getComputedStyle(row).outlineStyle).not.toBe('solid')
    }
    expect(rows[4].className).not.toContain('bg-koma-head')
    rows[3].dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(rows[1].getAttribute('aria-selected')).toBe('false')
    expect(rows[0].getAttribute('aria-current')).toBe('true')
    expect(host.querySelector('[data-tour="skills-add"]')).toBeTruthy()
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
    expect(getComputedStyle(rows[0]).outlineStyle).not.toBe('solid')
    for (let index = 1; index < rows.length; index++) {
      await userEvent.keyboard('{ArrowDown}')
      await new Promise((resolve) => requestAnimationFrame(resolve))
      expect(document.activeElement).toBe(rows[index])
      expect(rows.filter((row) => row.tabIndex === 0)).toEqual([rows[index]])
      expect(getComputedStyle(rows[index]).outlineStyle).not.toBe('solid')
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
