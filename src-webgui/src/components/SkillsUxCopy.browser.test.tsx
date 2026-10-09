import { userEvent } from 'vitest/browser'
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import SettingsTab from './SettingsTab'
import SkillTab from './SkillTab'
import { TabBar } from './TabBar'
import UploadSkillTab from './UploadSkillTab'
import { useKoma, type SettingsValues, type SkillCatalogueEntry, type SkillDetail, type Tab } from '../store/koma'

const AUTUMN = {
  bg: '#2e2a20',
  fg: '#f1dca7',
  dim: '#baa587',
  accent: '#ffcb69',
}
const DARK = {
  bg: '#0f1419',
  fg: '#b3b1ad',
  dim: '#626a73',
  accent: '#e6b450',
}

let stylesheet: HTMLLinkElement
const req = vi.fn()

function setTheme(theme: typeof AUTUMN) {
  const root = document.documentElement
  root.style.setProperty('--koma-bg', theme.bg)
  root.style.setProperty('--koma-fg', theme.fg)
  root.style.setProperty('--koma-dim', theme.dim)
  root.style.setProperty('--koma-accent', theme.accent)
}

async function waitForSelector<T extends Element>(selector: string): Promise<T> {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    const element = document.querySelector<T>(selector)
    if (element) return element
    await new Promise((resolve) => window.setTimeout(resolve, 25))
  }
  throw new Error(`Timed out waiting for ${selector}`)
}

function settingsValues(extraSkillRoots: string[]): SettingsValues {
  return {
    name: 'Browser test',
    workdir: ['/project'],
    shortSend: true,
    slidingCache: false,
    bashSaving: true,
    codingAutosave: false,
    internetMode: 'simple',
    palette: 'Autumn',
    effort: '',
    subagentMaxTurns: 500,
    shortSendEngageN: 80,
    shortSendTailN: 40,
    maxOutputTokens: 0,
    contextWindowLimit: 0,
    contextModelAlias: '',
    extraSkillRoots,
  }
}

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
  setTheme(AUTUMN)
  useKoma.setState({
    req,
    settingsValues: settingsValues([]),
    skillLastOp: null,
  })
})

describe.sequential('Skills UX copy and disclosures', () => {
  it('stacks Settings labels above controls and lets numeric drafts stay empty until committed', async () => {
    const screen = await render(<div data-testid="settings-width" style={{ width: 1100 }}><SettingsTab /></div>)
    const width = document.querySelector<HTMLElement>('[data-testid="settings-width"]')!
    for (const size of [1100, 360]) {
      width.style.width = `${size}px`
      for (const row of document.querySelectorAll<HTMLElement>('.settings-row')) {
        const caption = row.firstElementChild!.getBoundingClientRect()
        const control = row.lastElementChild!.getBoundingClientRect()
        expect(control.top).toBeGreaterThanOrEqual(caption.bottom)
      }
    }
    expect(document.querySelector('.settings-content input[type="number"]')).toBeNull()
    for (const [label, key] of [
      ['Context window limit', 'contextWindowLimit'],
      ['Reply token limit', 'maxOutputTokens'],
      ['Subagent turn limit', 'subagentMaxTurns'],
    ] as const) {
      const field = screen.getByRole('textbox', { name: label, exact: true })
      const input = document.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!
      expect(input.inputMode).toBe('numeric')
      await userEvent.fill(field, '0')
      req.mockClear()
      await userEvent.keyboard('{Backspace}')
      expect(input.value).toBe('')
      expect(req).not.toHaveBeenCalled()
      await userEvent.fill(field, '42')
      await userEvent.keyboard('{Enter}')
      expect(req.mock.calls.some(([request]) => request.r === 'SetPrefs' && request[key] === 42)).toBe(true)
      expect(input.value).toBe('42')
    }
    const context = screen.getByRole('textbox', { name: 'Context window limit', exact: true })
    await userEvent.fill(context, '400000')
    await userEvent.keyboard('{Enter}')
    expect(req.mock.calls.some(([request]) => request.contextWindowLimit === 300000)).toBe(true)
    const reply = screen.getByRole('textbox', { name: 'Reply token limit', exact: true })
    await userEvent.fill(reply, '')
    await userEvent.keyboard('{Enter}')
    expect(document.querySelector<HTMLInputElement>('input[aria-label="Reply token limit"]')!.value).toBe('0')
    const turns = screen.getByRole('textbox', { name: 'Turns on screen', exact: true })
    await userEvent.fill(turns, '')
    expect(document.querySelector<HTMLInputElement>('input[aria-label="Turns on screen"]')!.value).toBe('')
    await userEvent.fill(turns, '25')
    await userEvent.keyboard('{Enter}')
    expect(useKoma.getState().ui.chatTurns).toBe(25)
  })

  it('picks skill and workspace folders only for local sessions and ignores cancelled or stale replies', async () => {
    const originalIpc = window.ipc
    const originalRemote = useKoma.getState().remoteState
    const postMessage = vi.fn()
    window.ipc = { postMessage }
    useKoma.setState((state) => ({
      settingsValues: settingsValues(['/skills']),
      session: { ...state.session, id: 'local-session' },
      remoteState: { ...state.remoteState, hostId: null, state: 'disconnected' },
    }))
    try {
      const screen = await render(<SettingsTab />)
      const reply = (path: string | null) => {
        const request = postMessage.mock.calls.map(([json]) => JSON.parse(json)).filter((request) => request.r === 'PickSettingsFolder').at(-1)
        expect(request).toBeTruthy()
        window.__komaFolderReply?.({ requestId: request.requestId, path })
      }
      await screen.getByRole('button', { name: 'Choose folder for External skill root 1' }).click()
      reply('/picked/skills')
      const input = document.querySelector<HTMLInputElement>('input[aria-label="External skill root 1"]')!
      await expect.poll(() => input.value).toBe('/picked/skills')
      await screen.getByRole('button', { name: 'Save', exact: true }).click()
      expect(req.mock.calls.some(([request]) => request.r === 'SetExtraSkillRoots' && request.roots[0] === '/picked/skills')).toBe(true)

      await screen.getByRole('button', { name: 'Add workspace folder' }).click()
      reply('/picked/project')
      const workdir = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Working directories"]')!
      await expect.poll(() => workdir.value).toBe('/project\n/picked/project')
      expect(req.mock.calls.some(([request]) => request.r === 'SetPrefs' && request.workdir?.join('\n') === '/project\n/picked/project')).toBe(true)
      req.mockClear()
      await screen.getByRole('button', { name: 'Add workspace folder' }).click()
      reply(null)
      await expect.poll(() => document.querySelector<HTMLButtonElement>('button[aria-label="Add workspace folder"]')?.disabled).toBe(false)
      expect(req).not.toHaveBeenCalled()

      await screen.getByRole('button', { name: 'Add workspace folder' }).click()
      useKoma.setState((state) => ({ remoteState: { ...state.remoteState, hostId: 'ssh-host', state: 'connected' } }))
      reply('/local/path')
      await expect.poll(() => document.querySelector('button[aria-label="Add workspace folder"]')).toBeNull()
      expect(document.querySelector('button[aria-label="Choose folder for External skill root 1"]')).toBeNull()
      expect(workdir.value).toBe('/project\n/picked/project')
      expect(req.mock.calls.some(([request]) => request.r === 'SetPrefs')).toBe(false)
      expect(document.querySelector('input[aria-label="External skill root 1"]')).toBeTruthy()
    } finally {
      window.ipc = originalIpc
      useKoma.setState({ remoteState: originalRemote })
    }
  })

  it('keeps enabled Settings controls opaque in dark and light palettes', async () => {
    await render(<SettingsTab />)
    const input = await waitForSelector<HTMLInputElement>('.settings-content input[inputmode="numeric"]')
    const options = [...document.querySelectorAll<HTMLButtonElement>('.settings-content button[aria-pressed]')]
    expect(options.length).toBeGreaterThan(0)
    const save = [...document.querySelectorAll<HTMLButtonElement>('.settings-content button')].find((button) => button.textContent?.trim() === 'Save')!
    expect(save.disabled).toBe(true)
    for (const theme of [DARK, { bg: '#fafafa', fg: '#202020', dim: '#555555', accent: '#2555aa' }]) {
      setTheme(theme)
      const probe = document.createElement('span')
      document.body.append(probe)
      probe.style.color = theme.fg
      expect(getComputedStyle(input).color).toBe(getComputedStyle(probe).color)
      probe.style.color = theme.bg
      expect(getComputedStyle(input).backgroundColor).toBe(getComputedStyle(probe).color)
      expect(getComputedStyle(input).opacity).toBe('1')
      expect(getComputedStyle(input, '::placeholder').opacity).toBe('1')
      for (const button of document.querySelectorAll<HTMLButtonElement>('.settings-content button:enabled, .settings-nav button:enabled')) {
        expect(getComputedStyle(button).opacity).toBe('1')
      }
      expect(Number(getComputedStyle(save).opacity)).toBeLessThan(1)
      const selected = options.find((button) => button.getAttribute('aria-pressed') === 'true')!
      const unselected = options.find((button) => button.getAttribute('aria-pressed') === 'false')!
      expect(getComputedStyle(selected).backgroundColor).not.toBe(getComputedStyle(unselected).backgroundColor)
      probe.remove()
    }
  })

  it('keeps ZIP guidance concise while exposing every enforced package requirement', async () => {
    useKoma.setState((state) => ({ session: { ...state.session, id: null } }))
    const screen = await render(<UploadSkillTab />)
    expect(screen.getByRole('heading', { name: 'Upload skill (.zip)' })).toBeTruthy()
    const project = [...document.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent === 'Project')
    const choose = [...document.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent?.includes('Choose .zip'))
    expect(project?.disabled).toBe(true)
    expect(choose?.disabled).toBe(true)
    expect(screen.getByText('ZIP up to 64 MiB. Must contain a valid SKILL.md.')).toBeTruthy()

    const summary = document.querySelector<HTMLElement>('summary')!
    const details = summary.closest('details')!
    expect(summary.textContent).toBe('Package requirements')
    expect(details.open).toBe(false)

    summary.focus()
    await userEvent.keyboard('{Enter}')
    expect(document.activeElement).toBe(summary)
    expect(details.open).toBe(true)
    const requirements = details.textContent ?? ''
    expect(requirements).toContain('Maximum 1,000 files and folder depth 16.')
    expect(requirements).toContain('Maximum 8 MiB per companion file.')
    expect(requirements).toContain('Maximum 2 MiB for SKILL.md.')
    expect(requirements).toContain('Maximum 64 MiB compressed and 64 MiB uncompressed total.')
    expect(requirements).toContain('No symlinks, special files, path traversal, collisions, or malformed SKILL.md.')

    const root = summary.closest<HTMLElement>('.bg-koma-bg')!
    expect(getComputedStyle(summary).color).toBe('rgb(241, 220, 167)')
    expect(getComputedStyle(root).backgroundColor).toBe('rgb(46, 42, 32)')
    setTheme(DARK)
    expect(getComputedStyle(summary).color).toBe('rgb(179, 177, 173)')
    expect(getComputedStyle(root).backgroundColor).toBe('rgb(15, 20, 25)')
  })

  it('retains the client-side 64 MiB ZIP rejection', async () => {
    useKoma.setState((state) => ({ session: { ...state.session, id: 'session-a' } }))
    await render(<UploadSkillTab />)
    const input = document.querySelector<HTMLInputElement>('input[type="file"]')!
    const file = new File(['zip'], 'oversized.zip', { type: 'application/zip' })
    Object.defineProperty(file, 'size', { value: 64 * 1024 * 1024 + 1 })
    const transfer = new DataTransfer()
    transfer.items.add(file)
    Object.defineProperty(input, 'files', { configurable: true, value: transfer.files })
    input.dispatchEvent(new Event('change', { bubbles: true }))
    await new Promise((resolve) => requestAnimationFrame(resolve))
    expect(useKoma.getState().ui.toast?.text).toBe('ZIP package exceeds the 64 MiB upload limit.')
    expect(useKoma.getState().ui.toast?.kind).toBe('error')
  })

  it('keeps two visible skill tabs when Edit with Koma focuses an active chat', async () => {
    const alpha: SkillCatalogueEntry = {
      skillId: 'opaque-alpha', generation: 'gen-alpha', name: 'alpha', description: 'Alpha skill', triggers: '',
      sourceTier: 'global', scope: 'global', sourcePath: '/skills/alpha/SKILL.md',
    }
    const beta: SkillCatalogueEntry = {
      skillId: 'opaque-beta', generation: 'gen-beta', name: 'beta', description: 'Beta skill', triggers: '',
      sourceTier: 'global', scope: 'global', sourcePath: '/skills/beta/SKILL.md',
    }
    const alphaTab = { id: 'skill:opaque-alpha', kind: 'skill' as const, skillId: alpha.skillId, title: alpha.name }
    const betaTab = { id: 'skill:opaque-beta', kind: 'skill' as const, skillId: beta.skillId, title: beta.name }
    const tabs: Tab[] = [{ id: 'chat', kind: 'chat' }, alphaTab, betaTab]
    const detail: SkillDetail = {
      ...alpha, declaredTools: [], instruction: 'Alpha instructions', companionFiles: [], editable: true,
      structuredSaveSupported: true,
    }
    useKoma.setState((state) => ({
      session: { ...state.session, id: 'session-a' },
      skills: [alpha, beta],
      skillDetails: { [alphaTab.id]: detail },
      skillDetailPending: {},
      skillDetailErrors: {},
      ui: {
        ...state.ui,
        tabs,
        activeTabId: alphaTab.id,
        groups: ['g0'],
        tabGroup: { chat: 'g0', [alphaTab.id]: 'g0', [betaTab.id]: 'g0' },
        groupActive: { g0: alphaTab.id },
        activeGroupId: 'g0',
        groupSizes: { g0: 1 },
        preserveTabsOnNextSession: false,
      },
    }))

    const screen = await render(<div><TabBar groupId="g0" focused /><SkillTab tab={alphaTab} /></div>)
    await screen.getByRole('button', { name: 'Edit with Koma' }).click()
    await new Promise((resolve) => requestAnimationFrame(resolve))

    expect(req.mock.calls.some(([request]) => request.r === 'NewSession')).toBe(false)
    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual(['chat', alphaTab.id, betaTab.id])
    expect(useKoma.getState().ui.activeTabId).toBe('chat')
    const visibleTabTitles = [...document.querySelectorAll<HTMLElement>('[role="tab"][title]')].map((element) => element.title)
    expect(visibleTabTitles).toContain('alpha')
    expect(visibleTabTitles).toContain('beta')
  })

  it('restores visible skill tabs after detached Edit with Koma creates the first chat', async () => {
    const alpha: SkillCatalogueEntry = {
      skillId: 'opaque-alpha', generation: 'gen-alpha', name: 'alpha', description: 'Alpha', triggers: '',
      sourceTier: 'global', scope: 'global', sourcePath: '/skills/alpha/SKILL.md',
    }
    const alphaTab = { id: 'skill:opaque-alpha', kind: 'skill' as const, skillId: alpha.skillId, title: alpha.name }
    const betaTab = { id: 'skill:opaque-beta', kind: 'skill' as const, skillId: 'opaque-beta', title: 'beta' }
    const detail: SkillDetail = {
      ...alpha, declaredTools: [], instruction: 'Alpha instructions', companionFiles: [], editable: true,
      structuredSaveSupported: true,
    }
    useKoma.setState((state) => ({
      session: { ...state.session, id: null },
      skills: [alpha],
      skillDetails: { [alphaTab.id]: detail },
      ui: {
        ...state.ui,
        tabs: [{ id: 'chat', kind: 'chat' }, alphaTab, betaTab, { id: 'upload-skill', kind: 'uploadSkill' }],
        activeTabId: alphaTab.id,
        groups: ['g0'],
        tabGroup: { chat: 'g0', [alphaTab.id]: 'g0', [betaTab.id]: 'g0', 'upload-skill': 'g0' },
        groupActive: { g0: alphaTab.id },
        activeGroupId: 'g0',
        groupSizes: { g0: 1 },
        preserveTabsOnNextSession: false,
        preservedTabLayout: null,
      },
    }))

    const screen = await render(<div><TabBar groupId="g0" focused /><SkillTab tab={alphaTab} /></div>)
    await screen.getByRole('button', { name: 'Edit with Koma' }).click()
    expect(req.mock.calls.filter(([request]) => request.r === 'NewSession')).toHaveLength(1)
    expect(useKoma.getState().ui.activeTabId).toBe('chat')
    useKoma.getState().push({ k: 'Switching', to: 'first-chat' })
    useKoma.getState().detachSession()
    const firstChatSnapshot = {
      k: 'Snapshot', session: 'first-chat', state: 'idle', messages: [], title: '', subagents: [], bash: [],
      fileChanges: [], attachments: [], pendingSteer: [], awaitingApproval: false,
      palette: useKoma.getState().palette, loadedSkillNames: [],
    } as const
    useKoma.getState().push({ ...firstChatSnapshot, session: 'stale-in-flight-chat' } as any)
    expect(useKoma.getState().ui.preserveTabsOnNextSession).toBe(true)
    useKoma.getState().push(firstChatSnapshot as any)
    await new Promise((resolve) => requestAnimationFrame(resolve))

    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual(['chat', alphaTab.id, betaTab.id, 'upload-skill'])
    expect(useKoma.getState().ui.activeTabId).toBe('chat')
    const visibleTabTitles = [...document.querySelectorAll<HTMLElement>('[role="tab"][title]')].map((element) => element.title)
    expect(visibleTabTitles).toContain('alpha')
    expect(visibleTabTitles).toContain('beta')
    expect(visibleTabTitles).toContain('Upload skill (.zip)')
  })

  it('makes automatic locations scannable without changing External-root controls', async () => {
    await render(<SettingsTab />)
    expect(document.body.textContent).toContain('No additional External skill locations configured.')
    expect(document.body.textContent).toContain('Existing directories only. External locations are read-only.')

    const summary = [...document.querySelectorAll<HTMLElement>('summary')].find((item) => item.textContent === 'Automatically searched locations')!
    const details = summary.closest('details')!
    expect(details.open).toBe(false)
    summary.focus()
    await userEvent.keyboard('{Enter}')
    expect(document.activeElement).toBe(summary)
    expect(details.open).toBe(true)

    const content = details.textContent ?? ''
    expect(content).toContain('Global — ~/.koma/skills')
    expect(content).toContain('Compatibility (Claude project folder) — <project>/.claude/skills')
    expect(content).toContain('Project — <project>/.agent/skills')
    expect(content).toContain('Project — <project>/.agents/skills')
    expect(content).toContain('External — locations configured above')
    expect(content).toContain('Locations are scanned when the Skills panel opens. To scan again, use the circular-arrow button in the Skills header (Rescan skill locations). Project locations require an active chat.')
    expect(content).toContain('If skills have the same name, the location listed later wins.')

    useKoma.setState({ settingsValues: settingsValues(['/opt/shared-skills']) })
    const firstRoot = await waitForSelector<HTMLInputElement>('input[aria-label="External skill root 1"]')
    expect(firstRoot.value).toBe('/opt/shared-skills')
    expect(document.querySelector('button[aria-label="Remove External skill root 1"]')).toBeTruthy()

    await userEvent.click([...document.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent?.includes('Add location'))!)
    const secondRoot = document.querySelector<HTMLInputElement>('input[aria-label="External skill root 2"]')!
    await userEvent.fill(secondRoot, '/opt/team-skills')
    const save = [...document.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent?.includes('Save'))!
    expect(save.disabled).toBe(false)
    req.mockClear()
    await userEvent.click(save)
    expect(req.mock.calls.some(([request]) => request.r === 'SetExtraSkillRoots' && request.roots.join(',') === '/opt/shared-skills,/opt/team-skills')).toBe(true)

    const root = summary.closest<HTMLElement>('.bg-koma-bg')!
    expect(getComputedStyle(summary).color).toBe('rgb(241, 220, 167)')
    expect(getComputedStyle(root).backgroundColor).toBe('rgb(46, 42, 32)')
    setTheme(DARK)
    expect(getComputedStyle(summary).color).toBe('rgb(179, 177, 173)')
    expect(getComputedStyle(root).backgroundColor).toBe('rgb(15, 20, 25)')
  })
})
