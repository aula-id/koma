import { create } from 'zustand'
import { HELP_MANIFEST } from './helpKnowledge'
// Guided product tours for the GUI Tutorial tab (driver.js 1.3.5).
// One driver instance + full steps array — native Next/Done/progress.
// DOM prep: await work in onNextClick, then driver.moveNext(). Never
// destroy/rebuild per step.

import { driver, type DriveStep, type Config, type Driver } from 'driver.js'
import 'driver.js/dist/driver.css'
import { useKoma } from '../store/koma'

export type TourId =
  | 'oauth-setup'
  | 'provider-setup'
  | 'activity-bar'
  | 'sessions-hub'
  | 'composer'
  | 'agents'
  | 'skills'
  | 'git'
  | 'mcp'
  | 'remote'
  | 'store'
  | 'settings'
  | 'connector'
  | 'web-search-setup'
  | 'notification-history'
  | 'coding-file-saves'
  | 'terminal-selection'

export type TourMeta = {
  id: TourId
  title: string
  blurb: string
  kind: 'setup' | 'spotlight'
}

export const TOUR_CATALOGUE: TourMeta[] = [
  {
    id: 'oauth-setup',
    title: 'OAuth → model',
    blurb: 'Open Connector, connect an account, add a model, pick it in chat.',
    kind: 'setup',
  },
  {
    id: 'provider-setup',
    title: 'API provider → model',
    blurb: 'Open Connector, add provider + key, add model, select it.',
    kind: 'setup',
  },
  {
    id: 'activity-bar',
    title: 'Activity bar',
    blurb: 'Left strip of panels — Explore, Git, Agents, and more.',
    kind: 'spotlight',
  },
  {
    id: 'sessions-hub',
    title: 'Sessions hub',
    blurb: 'Start a new session or resume an existing one.',
    kind: 'spotlight',
  },
  {
    id: 'composer',
    title: 'Composer',
    blurb: 'Message box, model picker, attachments, shell with !.',
    kind: 'spotlight',
  },
  {
    id: 'connector',
    title: 'Connector',
    blurb: 'Providers, OAuth accounts, and models in one panel.',
    kind: 'spotlight',
  },
  {
    id: 'agents',
    title: 'Agents',
    blurb: 'Built-in and custom sub-agents.',
    kind: 'spotlight',
  },
  {
    id: 'skills',
    title: 'Skills',
    blurb: 'Discover, create, load, edit, duplicate, and package reusable instructions.',
    kind: 'spotlight',
  },
  {
    id: 'git',
    title: 'Source control',
    blurb: 'Git status, diffs, branches, and remotes.',
    kind: 'spotlight',
  },
  {
    id: 'mcp',
    title: 'MCP',
    blurb: 'Model Context Protocol servers.',
    kind: 'spotlight',
  },
  {
    id: 'remote',
    title: 'Remote',
    blurb: 'SSH hosts and remote sessions.',
    kind: 'spotlight',
  },
  {
    id: 'store',
    title: 'Extensions',
    blurb: 'Browse and install extensions from koma.run.',
    kind: 'spotlight',
  },
  {
    id: 'settings',
    title: 'Settings',
    blurb: 'Theme, keys, appearance, and more.',
    kind: 'spotlight',
  },
]

TOUR_CATALOGUE.push(
  { id: 'web-search-setup', title: 'Set up web search', blurb: 'Choose a search provider in Settings and activate it yourself.', kind: 'setup' },
  { id: 'notification-history', title: 'Notification history', blurb: 'Select a scope, search notices and manage read state.', kind: 'spotlight' },
  { id: 'coding-file-saves', title: 'Edit and save a file', blurb: 'Open a workspace file, edit it and confirm the save outcome.', kind: 'setup' },
  { id: 'terminal-selection', title: 'Terminal selection', blurb: 'Select the terminal tab before copying text or typing.', kind: 'spotlight' },
)
export const useGuide = create<{ id: TourId | null; step: number; blocked: string | null }>(() => ({ id: null, step: 0, blocked: null }))
let guideCleanup: (() => void) | null = null
let initialCounts = { providers: 0, models: 0, accounts: 0 }
const dirtyDuringGuide = new Map<string, string>()
let terminalSelected = false
const SESSION_GUIDES = new Set(['composer', 'agents', 'skills', 'git', 'coding-file-saves', 'terminal-selection', 'notification-history'])

/** Only registered first-party navigation. Never accept model-supplied selectors. */
export async function openHelpView(id: string): Promise<boolean> {
  if (!HELP_MANIFEST.navigation.includes(id)) return false
  const state = useKoma.getState()
  if (id === 'help') { state.openHelpTab(); return true }
  if (id === 'settings') { state.openSettingsTab(); return true }
  if (id === 'notifications') { state.openNotificationsTab(); return true }
  if (id === 'analytics') { state.openAnalyticsTab(); return true }
  if (id === 'graph') { state.openGraphTab(); return true }
  if (id === 'terminal') { const tab = state.ui.tabs.find(t => t.kind === 'terminal'); if (!tab) return false; state.activateTab(tab.id); return true }
  if (id === 'chat') { state.activateTab('chat'); return true }
  if (id === 'sessions') return click('[data-tour="change-session"]') || click('[data-tour="sessions-hub-open"]')
  return ensureSidebarView(id)
}

// ─── DOM helpers ────────────────────────────────────────────────────────────

function qs<T extends Element = Element>(sel: string): T | null {
  try {
    return document.querySelector(sel) as T | null
  } catch {
    return null
  }
}

function isVisible(el: Element | null): boolean {
  if (!el || !(el instanceof HTMLElement)) return false
  const st = getComputedStyle(el)
  if (st.display === 'none' || st.visibility === 'hidden') return false
  const r = el.getBoundingClientRect()
  return r.width > 0 && r.height > 0
}

/** First visible match among selectors (in order). Never a comma-selector. */
function firstVisible(...sels: string[]): Element | undefined {
  for (const sel of sels) {
    const el = Array.from(document.querySelectorAll(sel)).find(isVisible)
    if (el) return el
  }
  return undefined
}

function sleep(ms: number) {
  const gen = runGen
  return new Promise<void>((resolve, reject) => window.setTimeout(() => gen === runGen ? resolve() : reject(new Error('Guide cancelled')), ms))
}

async function waitFor(sel: string, timeoutMs = 4000): Promise<Element | null> {
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    const el = qs(sel)
    if (el && isVisible(el)) return el
    await sleep(40)
  }
  const el = qs(sel)
  return isVisible(el) ? el : null
}

function click(sel: string): boolean {
  const el = qs<HTMLElement>(sel)
  if (!el || !isVisible(el)) return false
  el.click()
  return true
}

async function ensureSidebarView(view: string, panelSel?: string): Promise<boolean> {
  if (panelSel) {
    const already = qs(panelSel)
    if (already && isVisible(already)) return true
  }

  const barBtn = qs<HTMLElement>(`[data-tour-view="${view}"]`)
  if (barBtn) {
    if (panelSel && qs(panelSel) && isVisible(qs(panelSel))) return true
    barBtn.click()
    await sleep(120)
    if (panelSel) {
      const el = await waitFor(panelSel, 1500)
      if (el && isVisible(el)) return true
      barBtn.click()
      await sleep(120)
      return !!(await waitFor(panelSel, 2000))
    }
    return true
  }

  const more = qs<HTMLElement>('[aria-label="Additional Views"], [title="Additional Views"]')
  if (more) {
    more.click()
    await sleep(80)
    const labels: Record<string, string> = {
      connector: 'Connector',
      agents: 'Agents',
      skills: 'Skills',
      git: 'Source Control',
      mcp: 'MCP',
      remote: 'Remote',
      store: 'Extensions',
      explore: 'Explore',
      coding: 'Coding',
      usage: 'Usage',
      importGraph: 'Import Graph',
    }
    const label = labels[view] ?? view
    const hit = (Array.from(document.querySelectorAll('button')) as HTMLElement[]).find(
      (b) => b.textContent?.trim() === label,
    )
    if (hit) {
      hit.click()
      await sleep(120)
      if (panelSel) return !!(await waitFor(panelSel, 2000))
      return true
    }
  }
  return panelSel ? !!(await waitFor(panelSel, 400)) : false
}

async function ensureConnectorList(): Promise<boolean> {
  const ok = await ensureSidebarView('connector', '[data-tour="connector-panel"]')
  if (!ok) return false
  if (!qs('[data-tour="connector-list"]') && qs('[data-tour="connector-back"]')) {
    click('[data-tour="connector-back"]')
    await waitFor('[data-tour="connector-list"]', 2000)
  }
  return !!qs('[data-tour="connector-list"]')
}

async function openConnectorAdd(which: 'provider' | 'oauth' | 'model'): Promise<boolean> {
  if (!(await ensureConnectorList())) return false
  const sel =
    which === 'provider'
      ? '[data-tour="connector-add-provider"]'
      : which === 'oauth'
        ? '[data-tour="connector-add-oauth"]'
        : '[data-tour="connector-add-model"]'
  const btn = await waitFor(sel, 2000)
  if (!btn) return false
  ;(btn as HTMLElement).click()
  await sleep(300)
  if (which === 'provider') {
    return !!(await waitFor('[data-tour="provider-form-pick"]', 2500)) ||
      !!(await waitFor('[data-tour="provider-form"]', 500))
  }
  if (which === 'oauth') {
    return !!(await waitFor('[data-tour="oauth-picker"]', 2500))
  }
  return !!(await waitFor('[data-tour="model-form"]', 2500))
}

// ─── Step factory ───────────────────────────────────────────────────────────

type Side = 'top' | 'right' | 'bottom' | 'left'

/**
 * DriveStep with live element resolver.
 * `onNext` (if set) owns the Next button: run prep for the *following* step,
 * then moveNext(). Without onNext, driver advances normally.
 */
export type WorkflowStep = { targets: string[]; title: string; description: string; completion: 'manual' | 'provider-added' | 'account-connected' | 'model-added' | 'file-opened' | 'file-saved' | 'terminal-selected'; blocked: string }
function stepComplete(completion: WorkflowStep['completion']): boolean {
  const st = useKoma.getState()
  if (completion === 'manual') return true
  if (completion === 'model-added') return st.config.models.length > initialCounts.models
  if (completion === 'provider-added') return st.config.providers.length > initialCounts.providers
  if (completion === 'account-connected') return st.oauth.conns.length > initialCounts.accounts
  if (completion === 'file-opened') return st.ui.tabs.some(t => t.id === st.ui.activeTabId && t.kind === 'codingFile')
  if (completion === 'file-saved') return [...dirtyDuringGuide].some(([key, fingerprint]) => { const f = st.coding.files[key]; return f && f.fingerprint !== fingerprint && !f.dirty && !f.saving && !f.error && !f.conflict })
  return terminalSelected
}
const stepMetadata = new WeakMap<DriveStep, WorkflowStep>()
function step(opts: {
  targets: string[]
  title: string
  description: string
  side?: Side
  onNext?: () => void | Promise<void>
  disableActiveInteraction?: boolean
}): DriveStep {
  const resolve = () => firstVisible(...opts.targets) as Element

  const popover: NonNullable<DriveStep['popover']> = {
    title: opts.title,
    description: opts.description,
    side: opts.side ?? 'right',
    align: 'start',
  }

  const completion: WorkflowStep['completion'] = ['Model form', 'Configure the model'].includes(opts.title) ? 'model-added' : ['Provider form', 'Endpoint + API key'].includes(opts.title) ? 'provider-added' : opts.title === 'Choose a provider' && opts.targets.some(t => t.includes('oauth-picker')) ? 'account-connected' : opts.title === 'Open a file' ? 'file-opened' : opts.title === 'Save your edit' ? 'file-saved' : opts.title === 'Terminal selection' ? 'terminal-selected' : 'manual'
  let advancing = false
  popover.onNextClick = (_el, _s, { driver: d }) => {
    if (advancing) return
    const gen = runGen
    void (async () => {
      const st = useKoma.getState()
      const completed = stepComplete(completion)
      if (completion === 'manual' && !firstVisible(...opts.targets)) { useGuide.setState({ blocked: 'Required control is unavailable or hidden.' }); return }
      if (!completed) { useGuide.setState({ blocked: 'Complete the displayed action yourself before continuing.' }); return }
      advancing = true
      try {
        await opts.onNext?.()
        if (completion === 'model-added') st.activateTab('chat')
        await sleep(60)
        if (gen !== runGen) return
        const next = d.getConfig().steps?.[(d.getActiveIndex() ?? 0) + 1]
        if (next && typeof next.element === 'function' && !isVisible(next.element())) { useGuide.setState({ blocked: 'The next target is unavailable or hidden. Open the required view, or cancel this guide.' }); return }
        useGuide.setState({ blocked: null })
        if (d.isLastStep()) d.destroy(); else d.moveNext()
      } catch (error) {
        if (gen === runGen) useGuide.setState({ blocked: error instanceof Error ? error.message : 'Navigation is unavailable.' })
      } finally { advancing = false }
    })()
  }

  const result: DriveStep = {
    element: resolve,
    disableActiveInteraction: opts.disableActiveInteraction,
    popover,
    onHighlightStarted: (_el, _s, { driver: d }) => {
      useGuide.setState({ step: d.getActiveIndex() ?? 0, blocked: firstVisible(...opts.targets) ? null : 'This target is unavailable or hidden. Check prerequisites and panel visibility.' })
    },
    onHighlighted: (_el, _s, { driver: d }) => {
      const visible = !!firstVisible(...opts.targets)
      useGuide.setState({ step: d.getActiveIndex() ?? 0, blocked: visible ? null : 'This target is unavailable or hidden. Check prerequisites and panel visibility.' })
      if (!visible) { const description = document.querySelector('.driver-popover-description'); if (description) description.textContent = 'Paused: the required control is unavailable or hidden. Check the guide prerequisites and Sidebar visibility, or cancel the guide.' }
      window.setTimeout(() => {
        try {
          d.refresh()
        } catch {
          /* destroyed */
        }
      }, 100)
    },
  }
  stepMetadata.set(result, { targets: opts.targets, title: opts.title, description: opts.description, completion, blocked: 'Required control is unavailable or hidden.' })
  return result
}

type BuiltTour = {
  /** Run before drive(0) so step 0's target exists. */
  bootstrap: () => Promise<void>
  steps: DriveStep[]
}

function buildTour(id: TourId): BuiltTour | null {
  switch (id) {
    case 'web-search-setup': return {
      bootstrap: async () => { useKoma.getState().openSettingsTab(); await sleep(100); qs('[data-tour="web-search-settings"]')?.scrollIntoView() },
      steps: [step({ targets: ['[data-tour="web-search-settings"]'], title: 'Web search', description: 'Built-in DuckDuckGo needs no key. For Firecrawl, Tavily or Exa, enter its key and activate the provider yourself. Check the displayed active state.' })],
    }
    case 'notification-history': return {
      bootstrap: async () => { useKoma.getState().openNotificationsTab(); await waitFor('[data-tour="notification-history"]') },
      steps: [step({ targets: ['[data-tour="notification-history"]'], title: 'History scopes', description: 'Select Session or App. Search and filter severity. Selecting a row marks it read. Mark all read and confirmed Clear affect the selected scope; popup expiry leaves history intact.' })],
    }
    case 'coding-file-saves': return {
      bootstrap: async () => { await ensureSidebarView('coding'); },
      steps: [step({ targets: ['[data-tour="coding-panel"]'], title: 'Open a file', description: 'Open a workspace file from the Coding tree yourself. The guide continues when an editor tab is selected.' }), step({ targets: ['[data-tour="code-editor"]'], title: 'Save your edit', description: 'Edit this file, then use Save or Ctrl+S yourself. Resolve any unsaved-change or conflict prompts. The guide recognizes a successful save after an edit.' })],
    }
    case 'terminal-selection': return {
      bootstrap: async () => { const tab = useKoma.getState().ui.tabs.find(t => t.kind === 'terminal'); if (!tab) throw new Error('Open a terminal tab yourself before starting this guide.'); useKoma.getState().activateTab(tab.id); await sleep(100) },
      steps: [step({ targets: ['[data-tour="terminal-content"]'], title: 'Terminal selection', description: 'Select text in this terminal before copying. Input is sent to the shell. A read-only Bash job stream is a different view.' })],
    }
    case 'oauth-setup':
      return {
        bootstrap: async () => {
          await ensureSidebarView('connector', '[data-tour="connector-panel"]')
          await ensureConnectorList()
        },
        steps: [
          step({
            targets: ['[data-tour-view="connector"]', '[data-tour="activity-bar"]'],
            title: 'Connector',
            description:
              'Providers, OAuth accounts, and models live here. Next opens Connect account (+).',
            side: 'right',
            onNext: async () => {
              await openConnectorAdd('oauth')
            },
          }),
          step({
            targets: ['[data-tour="oauth-picker"]', '[data-tour="connector-add-oauth"]'],
            title: 'Choose a provider',
            description:
              'Pick Codex, Claude, koma.run, etc. Browser sign-in — koma never sees your password. Finish login, then Next.',
            side: 'left',
            onNext: async () => {
              // Back to list for the model step.
              if (qs('[data-tour="connector-back"]') && !qs('[data-tour="connector-list"]')) {
                click('[data-tour="connector-back"]')
                await waitFor('[data-tour="connector-list"]', 2000)
              }
              await ensureConnectorList()
            },
          }),
          step({
            targets: ['[data-tour="connector-add-model"]', '[data-tour="connector-list"]'],
            title: 'Add a model',
            description:
              'After the account shows Connected, Next opens Models → + Add model.',
            side: 'left',
            onNext: async () => {
              await openConnectorAdd('model')
            },
          }),
          step({
            targets: ['[data-tour="model-form"]', '[data-tour="form-save"]'],
            title: 'Model form',
            description:
              'Name it, pick the OAuth connection as Provider, set model id, enable Main, Save.',
            side: 'left',
          }),
          step({
            targets: [
              '[data-tour="model-picker"]',
              '[data-tour="composer"]',
              '[data-tour="start-screen"]',
            ],
            title: 'Select in the composer',
            description:
              'With a session open, use the composer model picker. Start a session from the hub if needed.',
            side: 'top',
          }),
        ],
      }

    case 'provider-setup':
      return {
        bootstrap: async () => {
          await ensureSidebarView('connector', '[data-tour="connector-panel"]')
          await ensureConnectorList()
        },
        steps: [
          step({
            targets: ['[data-tour-view="connector"]', '[data-tour="activity-bar"]'],
            title: 'Connector',
            description: 'API-key providers live under Providers. Next opens + Add provider.',
            side: 'right',
            onNext: async () => {
              await openConnectorAdd('provider')
            },
          }),
          step({
            targets: ['[data-tour="provider-form-pick"]', '[data-tour="provider-form"]'],
            title: 'Choose a preset',
            description:
              'Pick OpenRouter, OpenAI, Groq, … or Custom. Next opens a common preset form.',
            side: 'left',
            onNext: async () => {
              if (qs('[data-tour="provider-form-pick"]')) {
                const pref =
                  qs<HTMLElement>('[data-tour="provider-preset-openrouter"]') ||
                  qs<HTMLElement>('[data-tour="provider-preset-openai"]') ||
                  qs<HTMLElement>('[data-tour="provider-preset-custom"]')
                pref?.click()
                await waitFor('[data-tour="provider-form"]', 2500)
              }
            },
          }),
          step({
            targets: [
              '[data-tour="provider-api-key"]',
              '[data-tour="provider-form"]',
              '[data-tour="form-save"]',
            ],
            title: 'Endpoint + API key',
            description:
              'Confirm Name and Endpoint, paste your API key, Save. Next returns to the list for Add model.',
            side: 'left',
            onNext: async () => {
              if (!qs('[data-tour="connector-list"]') && qs('[data-tour="connector-back"]')) {
                click('[data-tour="connector-back"]')
                await waitFor('[data-tour="connector-list"]', 2000)
              }
              await ensureConnectorList()
            },
          }),
          step({
            targets: ['[data-tour="connector-add-model"]', '[data-tour="connector-list"]'],
            title: 'Add a model',
            description: 'Next opens Models → +.',
            side: 'left',
            onNext: async () => {
              await openConnectorAdd('model')
            },
          }),
          step({
            targets: ['[data-tour="model-form"]', '[data-tour="model-id"]', '[data-tour="form-save"]'],
            title: 'Configure the model',
            description:
              'Name, Provider, Model id (catalogue search when supported), Roles → Main, Save.',
            side: 'left',
          }),
          step({
            targets: [
              '[data-tour="model-picker"]',
              '[data-tour="composer"]',
              '[data-tour="start-screen"]',
            ],
            title: 'Select in the composer',
            description: 'Open a session and pick the new model as session Main.',
            side: 'top',
          }),
        ],
      }

    case 'activity-bar':
      return {
        bootstrap: async () => {},
        steps: [
          step({
            targets: ['[data-tour="activity-bar"]'],
            title: 'Activity bar',
            description:
              'Switches sidebar panels. Drag to reorder; hide in Settings → Sidebar. Overflow → ⋯.',
            side: 'right',
          }),
          step({
            targets: ['[data-tour-open="help"]'],
            title: 'Help and Notifications',
            description: 'Pinned at the bottom — always available.',
            side: 'right',
          }),
        ],
      }

    case 'sessions-hub':
      return {
        bootstrap: async () => { click('[data-tour="change-session"]'); await sleep(100) },
        steps: [
          step({
            targets: ['[data-tour="session-hub"]', '[data-tour="start-screen"]'],
            title: 'Sessions hub',
            description:
              'New session, open folder, resume, remote. Resume also lives in the titlebar search.',
            side: 'bottom',
          }),
        ],
      }

    case 'composer':
      return {
        bootstrap: async () => { useKoma.getState().activateTab('chat'); await sleep(100) },
        steps: [
          step({
            targets: ['[data-tour="composer"]'],
            title: 'Composer',
            description:
              'Type to chat. ! runs a local shell line. Attachments and model picker on the footer.',
            side: 'top',
          }),
          step({
            targets: ['[data-tour="model-picker"]', '[data-tour="composer"]'],
            title: 'Model picker',
            description: 'Switch session Main here (including koma free).',
            side: 'top',
          }),
        ],
      }

    case 'connector':
      return {
        bootstrap: async () => {
          await ensureSidebarView('connector', '[data-tour="connector-panel"]')
          await ensureConnectorList()
        },
        steps: [
          step({
            targets: ['[data-tour="connector-panel"]', '[data-tour-view="connector"]'],
            title: 'Connector',
            description: 'Providers, OAuth, and models in one panel.',
            side: 'left',
          }),
          step({
            targets: ['[data-tour="connector-list"]'],
            title: 'Three catalogues',
            description:
              'Providers (+), OAuth / Connect account (+), Models (+). Setup tours walk the full recipes.',
            side: 'left',
          }),
        ],
      }

    case 'agents':
      return {
        bootstrap: async () => {
          await ensureSidebarView('agents')
        },
        steps: [
          step({
            targets: ['[data-tour-view="agents"]'],
            title: 'Agents',
            description: 'Built-in and custom sub-agents.',
            side: 'right',
          }),
        ],
      }

    case 'skills':
      return {
        bootstrap: async () => {
          useKoma.getState().setSkillSelection([])
          await ensureSidebarView('skills', '[data-tour="skills-panel"]')
          if (qs('[data-tour="skills-add-back"]')) {
            click('[data-tour="skills-add-back"]')
            await waitFor('[data-tour="skills-list"]', 1500)
          }
        },
        steps: [
          step({
            targets: ['[data-tour-view="skills"]'],
            title: 'Skills panel',
            description: 'Open the dedicated Skills catalogue. Skills stay separate from sub-agents.',
            side: 'right',
          }),
          step({
            targets: ['[data-tour="skills-search"]', '[data-tour="skills-panel"]'],
            title: 'Search skills',
            description: 'Search by name, description, or triggers. Ctrl/Cmd+F focuses this field while the panel is open.',
            side: 'right',
          }),
          step({
            targets: ['[data-tour="skills-filters"]', '[data-tour="skills-panel"]'],
            title: 'Global and Project',
            description: 'Global lists skills from ~/.koma/skills. Project lists skills from the active chat workspace. A skill loaded into the chat is pinned to the top of its tab, marked with an accent bar, and labeled active. Use the refresh button in the Skills header to scan again.',
            side: 'right',
          }),
          step({
            targets: ['[data-tour="skills-list"]'],
            title: 'Ownership and selection',
            description: 'Global and Project skills are Koma-owned. External skills are read-only. Click or Enter opens a skill. Right-click a row for Load, Remove, Reload, Duplicate, and Delete. Ctrl/Cmd-click and Shift-click select several skills for that menu.',
            side: 'right',
            onNext: async () => {
              const first = qs<HTMLElement>('[data-tour="skills-list"] [role="option"]')
              const rect = first?.getBoundingClientRect()
              first?.dispatchEvent(new MouseEvent('contextmenu', {
                bubbles: true,
                cancelable: true,
                clientX: rect ? rect.left + 8 : 24,
                clientY: rect ? rect.top + 12 : 24,
              }))
              await waitFor('[data-tour="skills-context-menu"]', 1200)
            },
          }),
          step({
            targets: ['[data-tour="skills-context-menu"]', '[data-tour="skills-list"]'],
            title: 'Session actions',
            description: 'Right-click a skill, or a Ctrl/Cmd selection, for chat actions. Load into chat and Remove from chat change only the active chat. Save writes disk; Save & Reload also updates the current chat when the owned skill is loaded. Reload from disk remains for edits made outside the editor. Duplicate to Koma is for External skills; Delete is only for owned skills.',
            side: 'right',
            onNext: async () => {
              window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
              useKoma.getState().setSkillSelection([])
              await waitFor('[data-tour="skills-add"]', 1200)
            },
          }),
          step({
            targets: ['[data-tour="skills-add"]', '[data-tour="skills-panel"]'],
            title: 'Add skill',
            description: 'Add skill is available while a project is open. Choose Create new, upload a bounded ZIP package, or start a guided Create with Koma chat. Global skills stay editable without a project.',
            side: 'right',
            onNext: async () => {
              const add = qs<HTMLButtonElement>('[data-tour="skills-add"]')
              if (!add || add.disabled) return
              click('[data-tour="skills-add"]')
              await waitFor('[data-tour="skills-add-methods"]', 1200)
            },
          }),
          step({
            targets: ['[data-tour="skills-add-methods"]', '[data-tour="skills-panel"]'],
            title: 'Three creation methods',
            description: 'Create new opens the structured editor. Upload .zip installs a packaged Global/Project skill. Create with Koma starts an editable guided-chat draft.',
            side: 'right',
          }),
        ],
      }

    case 'git':
      return {
        bootstrap: async () => {
          await ensureSidebarView('git')
        },
        steps: [
          step({
            targets: ['[data-tour-view="git"]'],
            title: 'Source control',
            description: 'Status, stage, commit, diffs, branches, stash, remotes.',
            side: 'right',
          }),
        ],
      }

    case 'mcp':
      return {
        bootstrap: async () => {
          await ensureSidebarView('mcp')
        },
        steps: [
          step({
            targets: ['[data-tour-view="mcp"]'],
            title: 'MCP',
            description: 'Model Context Protocol servers. Tools appear once connected.',
            side: 'right',
          }),
        ],
      }

    case 'remote':
      return {
        bootstrap: async () => {
          await ensureSidebarView('remote')
        },
        steps: [
          step({
            targets: ['[data-tour-view="remote"]'],
            title: 'Remote',
            description: 'SSH hosts, connect, remote cwd, attach. Keys: Settings → SSH Keys.',
            side: 'right',
          }),
        ],
      }

    case 'store':
      return {
        bootstrap: async () => {
          await ensureSidebarView('store')
        },
        steps: [
          step({
            targets: ['[data-tour-view="store"]'],
            title: 'Extensions',
            description: 'Browse the koma.run store and manage installed extensions.',
            side: 'right',
          }),
        ],
      }

    case 'settings':
      return {
        bootstrap: async () => {
          click('[data-tour-open="settings"]')
          await sleep(150)
        },
        steps: [
          step({
            targets: ['[data-tour-open="settings"]'],
            title: 'Settings',
            description: 'Theme, appearance, coding, SSH keys, activity-bar layout, account.',
            side: 'right',
          }),
        ],
      }

    default:
      return null
  }
}

// ─── Runner ─────────────────────────────────────────────────────────────────

const BASE: Config = {
  animate: true,
  overlayOpacity: 0.55,
  stagePadding: 8,
  stageRadius: 6,
  allowClose: true,
  smoothScroll: true,
  popoverClass: 'koma-driver-theme',
  nextBtnText: 'Next',
  prevBtnText: 'Back',
  doneBtnText: 'Done',
  progressText: '{{current}} / {{total}}',
  showProgress: true,
  showButtons: ['next', 'previous', 'close'],
  allowKeyboardControl: true,
  overlayClickBehavior: 'close',
}

let active: Driver | null = null
let runGen = 0

/** Start a named tour. One driver; native Next/Done/progress. */
export function startTour(id: TourId | string): boolean {
  const tourId = TOUR_CATALOGUE.some((t) => t.id === id) ? (id as TourId) : null
  if (!tourId) return false

  const built = buildTour(tourId)
  if (!built || built.steps.length === 0) return false

  const gen = ++runGen
  active?.destroy()
  active = null

  guideCleanup?.(); guideCleanup = null
  useGuide.setState({ id: tourId, step: 0, blocked: null })
  const origin = useKoma.getState().session.id
  if (SESSION_GUIDES.has(tourId) && !origin) { useGuide.setState({ blocked: 'Attach a session before starting this guide.' }); return false }
  dirtyDuringGuide.clear(); terminalSelected = false
  initialCounts = { providers: useKoma.getState().config.providers.length, models: useKoma.getState().config.models.length, accounts: useKoma.getState().oauth.conns.length }
  const unsubscribe = useKoma.subscribe(s => { for (const [key, f] of Object.entries(s.coding.files)) { if (f.dirty && !dirtyDuringGuide.has(key)) dirtyDuringGuide.set(key, f.fingerprint) }; if (SESSION_GUIDES.has(tourId) && s.session.id !== origin) { stopTour(); useGuide.setState({ blocked: 'Guide cancelled because the session changed.' }) } })
  const observeSelection = (event: Event) => { terminalSelected = (event as CustomEvent<{ selected: boolean }>).detail?.selected === true }
  document.addEventListener('koma-terminal-selection', observeSelection)
  const cleanupObservers = () => { unsubscribe(); document.removeEventListener('koma-terminal-selection', observeSelection) }
  guideCleanup = cleanupObservers
  // Help stays open; guide navigation may focus another view.
  void (async () => {
    try {
      await built.bootstrap()
    } catch (error) {
      if (gen === runGen) useGuide.setState({ blocked: error instanceof Error ? error.message : 'Required view is unavailable.' })
      return
    }
    if (gen !== runGen) return

    try { await sleep(60) } catch { return }
    if (gen !== runGen) return

    const d = driver({
      ...BASE,
      steps: built.steps,
      onDestroyed: () => {
        if (active === d) { active = null; guideCleanup?.(); guideCleanup = null; useGuide.setState({ id: null, blocked: null }) }
      },
    })
    active = d
    d.drive(0)
    const timer = window.setInterval(() => {
      if (gen !== runGen || active !== d) return
      const meta = stepMetadata.get(d.getConfig().steps?.[d.getActiveIndex() ?? 0]!)
      const st = useKoma.getState()
      const complete = meta && meta.completion !== 'manual' && stepComplete(meta.completion)
      if (complete) d.getActiveStep()?.popover?.onNextClick?.(d.getActiveElement(), d.getActiveStep()!, { driver: d, config: d.getConfig(), state: d.getState() })
      else if (meta && !firstVisible(...meta.targets)) useGuide.setState({ blocked: meta.blocked })
    }, 250)
    guideCleanup = () => { cleanupObservers(); window.clearInterval(timer) }
    window.setTimeout(() => {
      try {
        d.refresh()
      } catch {
        /* destroyed */
      }
    }, 180)
  })()

  return true
}

export function stopTour() {
  runGen++
  guideCleanup?.(); guideCleanup = null
  useGuide.setState({ id: null, blocked: null })
  active?.destroy()
  active = null
}

export function tourMeta(id: string | null | undefined): TourMeta | undefined {
  if (!id) return undefined
  return TOUR_CATALOGUE.find((t) => t.id === id)
}

/** Registered steps for coverage tooling and Reference. */
export function workflowSteps(id: TourId): WorkflowStep[] {
  return buildTour(id)?.steps.map(s => stepMetadata.get(s)!).filter(Boolean) ?? []
}

export function workflowRegistry() {
  return TOUR_CATALOGUE.map(tour => ({ ...tour, prerequisites: SESSION_GUIDES.has(tour.id) ? ['attached-session'] : [], navigation: HELP_MANIFEST.articles.filter(a => a.workflows.includes(tour.id)).map(a => a.navigation), steps: workflowSteps(tour.id) }))
}

// Zoom changes the target bounds without replacing the running guide.
window.addEventListener('koma-ui-scale', () => active?.refresh())
