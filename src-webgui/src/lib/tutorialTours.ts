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
  | 'git'
  | 'mcp'
  | 'remote'
  | 'store'
  | 'settings'
  | 'connector'

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
    const el = qs(sel)
    if (el && isVisible(el)) return el
  }
  for (const sel of sels) {
    const el = qs(sel)
    if (el) return el
  }
  return undefined
}

function sleep(ms: number) {
  return new Promise<void>((r) => window.setTimeout(r, ms))
}

async function waitFor(sel: string, timeoutMs = 4000): Promise<Element | null> {
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    const el = qs(sel)
    if (el) return el
    await sleep(40)
  }
  return qs(sel)
}

function click(sel: string): boolean {
  const el = qs<HTMLElement>(sel)
  if (!el) return false
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

  if (opts.onNext) {
    popover.onNextClick = (_el, _s, { driver: d }) => {
      void (async () => {
        try {
          await opts.onNext?.()
        } catch {
          /* still advance */
        }
        await sleep(60)
        d.moveNext()
        window.setTimeout(() => {
          try {
            d.refresh()
          } catch {
            /* destroyed */
          }
        }, 140)
      })()
    }
  }

  return {
    element: resolve,
    disableActiveInteraction: opts.disableActiveInteraction,
    popover,
    onHighlighted: (_el, _s, { driver: d }) => {
      window.setTimeout(() => {
        try {
          d.refresh()
        } catch {
          /* destroyed */
        }
      }, 100)
    },
  }
}

type BuiltTour = {
  /** Run before drive(0) so step 0's target exists. */
  bootstrap: () => Promise<void>
  steps: DriveStep[]
}

function buildTour(id: TourId): BuiltTour | null {
  switch (id) {
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
            targets: ['[data-tour-open="tutorial"]'],
            title: 'Tutorial & Help',
            description: 'Pinned at the bottom — always available.',
            side: 'right',
          }),
        ],
      }

    case 'sessions-hub':
      return {
        bootstrap: async () => {},
        steps: [
          step({
            targets: ['[data-tour="start-screen"]', 'main'],
            title: 'Sessions hub',
            description:
              'New session, open folder, resume, remote. Resume also lives in the titlebar search.',
            side: 'bottom',
          }),
        ],
      }

    case 'composer':
      return {
        bootstrap: async () => {},
        steps: [
          step({
            targets: ['[data-tour="composer"]', 'main'],
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

  // Close Tutorial so chrome is visible, then bootstrap DOM, then drive.
  void (async () => {
    try {
      const st = useKoma.getState()
      if (st.ui.activeTabId === 'tutorial') {
        st.closeTab('tutorial')
        await sleep(100)
      }
    } catch {
      /* ignore */
    }

    try {
      await built.bootstrap()
    } catch {
      /* still try to drive */
    }
    if (gen !== runGen) return

    await sleep(60)
    if (gen !== runGen) return

    const d = driver({
      ...BASE,
      steps: built.steps,
      onDestroyed: () => {
        if (active === d) active = null
      },
    })
    active = d
    d.drive(0)
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
  active?.destroy()
  active = null
}

export function tourMeta(id: string | null | undefined): TourMeta | undefined {
  if (!id) return undefined
  return TOUR_CATALOGUE.find((t) => t.id === id)
}
