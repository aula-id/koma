// Guided product tours for the GUI Tutorial tab (driver.js).
// Tours DRIVE the real UI: open sidebar → click + → highlight form fields.
// Popover chrome is themed via CSS vars (--koma-*) — see styles.css
// `.koma-driver-theme`.

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
  /** Setup recipe (multi-step) vs one-shot spotlight. */
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

const BASE: Config = {
  animate: true,
  overlayOpacity: 0.55,
  stagePadding: 6,
  stageRadius: 6,
  allowClose: true,
  smoothScroll: true,
  popoverClass: 'koma-driver-theme',
  nextBtnText: 'Next',
  prevBtnText: 'Back',
  doneBtnText: 'Done',
  progressText: '{{current}} / {{total}}',
  showProgress: true,
  // Disable click-outside advance; we own navigation via onNextClick.
  allowKeyboardControl: true,
}

type Side = 'top' | 'right' | 'bottom' | 'left'

type TourStep = {
  /** Element to highlight (CSS selector). Optional for center popovers. */
  element?: string
  title: string
  description: string
  side?: Side
  /**
   * Run BEFORE this step is shown. Use to open panels / click + / wait for DOM.
   * Throw/reject to abort the tour.
   */
  prepare?: () => void | Promise<void>
  /** If set, Next runs this instead of only advancing (still advances after). */
  onNext?: () => void | Promise<void>
}

// ─── DOM helpers ────────────────────────────────────────────────────────────

function qs<T extends Element = Element>(sel: string): T | null {
  try {
    return document.querySelector(sel) as T | null
  } catch {
    return null
  }
}

function click(sel: string): boolean {
  const el = qs<HTMLElement>(sel)
  if (!el) return false
  el.click()
  return true
}

function sleep(ms: number) {
  return new Promise<void>((r) => window.setTimeout(r, ms))
}

/** Poll until selector matches or timeout. */
async function waitFor(sel: string, timeoutMs = 4000, intervalMs = 50): Promise<Element | null> {
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    const el = qs(sel)
    if (el) return el
    await sleep(intervalMs)
  }
  return qs(sel)
}

/**
 * Open a sidebar view without the toggle-close bug.
 * Strategy:
 * 1. If panel root for that view is already mounted and visible, done.
 * 2. Else click activity-bar button; if that closed the panel, click again.
 * 3. Overflow menu: open ⋯ and click matching label.
 */
async function ensureSidebarView(view: string, panelSel?: string): Promise<boolean> {
  const mounted = panelSel ? qs(panelSel) : qs(`[data-tour-view="${view}"]`)
  // Connector / others: if the panel content is already in DOM, stay.
  if (panelSel && qs(panelSel) && isVisible(qs(panelSel))) {
    return true
  }

  const barBtn = qs<HTMLElement>(`[data-tour-view="${view}"]`)
  if (barBtn) {
    // If aria / class suggests already active AND sidebar open, don't click
    // (would toggle closed). Heuristic: panel exists → open.
    if (panelSel && qs(panelSel) && isVisible(qs(panelSel))) {
      return true
    }
    barBtn.click()
    await sleep(80)
    if (panelSel) {
      const el = await waitFor(panelSel, 1500)
      if (el && isVisible(el)) return true
      // Might have toggled closed — click again to open.
      barBtn.click()
      await sleep(80)
      return !!(await waitFor(panelSel, 2000))
    }
    return true
  }

  // Overflow "Additional Views" menu
  const more = qs<HTMLElement>('[aria-label="Additional Views"], [title="Additional Views"]')
  if (more) {
    more.click()
    await sleep(60)
    // Menu items are buttons with the view label text
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
    const items = Array.from(document.querySelectorAll('button')) as HTMLElement[]
    const hit = items.find((b) => b.textContent?.trim() === label)
    if (hit) {
      hit.click()
      await sleep(100)
      if (panelSel) return !!(await waitFor(panelSel, 2000))
      return true
    }
  }

  void mounted
  return panelSel ? !!(await waitFor(panelSel, 500)) : false
}

function isVisible(el: Element | null): boolean {
  if (!el || !(el instanceof HTMLElement)) return false
  const st = getComputedStyle(el)
  if (st.display === 'none' || st.visibility === 'hidden' || st.opacity === '0') return false
  const r = el.getBoundingClientRect()
  return r.width > 0 && r.height > 0
}

async function openSingleton(kind: 'settings' | 'help' | 'tutorial') {
  click(`[data-tour-open="${kind}"]`)
  await sleep(120)
}

async function ensureConnectorList(): Promise<boolean> {
  const ok = await ensureSidebarView('connector', '[data-tour="connector-panel"]')
  if (!ok) return false
  // If a detail screen is open, go back to list.
  if (!qs('[data-tour="connector-list"]')) {
    if (qs('[data-tour="connector-back"]')) {
      click('[data-tour="connector-back"]')
      await waitFor('[data-tour="connector-list"]', 2000)
    }
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
  await sleep(220) // slide animation
  if (which === 'provider') {
    return !!(await waitFor('[data-tour="provider-form-pick"], [data-tour="provider-form"]', 2000))
  }
  if (which === 'oauth') {
    return !!(await waitFor('[data-tour="oauth-picker"]', 2000))
  }
  return !!(await waitFor('[data-tour="model-form"]', 2000))
}

// ─── Step builders ──────────────────────────────────────────────────────────

function pop(
  title: string,
  description: string,
  element: string | undefined,
  side: Side,
  prepare?: TourStep['prepare'],
  onNext?: TourStep['onNext'],
): TourStep {
  return { title, description, element, side, prepare, onNext }
}

function stepsFor(id: TourId): TourStep[] {
  switch (id) {
    case 'oauth-setup':
      return [
        pop(
          'Open Connector',
          'Connector holds OAuth accounts, API providers, and models. Opening it now…',
          '[data-tour-view="connector"], [data-tour="activity-bar"]',
          'right',
          async () => {
            await ensureSidebarView('connector', '[data-tour="connector-panel"]')
          },
        ),
        pop(
          'Connect an account',
          'Click the + next to OAuth (highlighted). That opens the provider picker.',
          '[data-tour="connector-add-oauth"], [data-tour="connector-list"]',
          'left',
          async () => {
            await ensureConnectorList()
          },
          async () => {
            await openConnectorAdd('oauth')
          },
        ),
        pop(
          'Choose a provider',
          'Pick Codex, Claude, koma.run, etc. Sign-in opens your browser — koma never sees your password. Complete login there, then come back.',
          '[data-tour="oauth-picker"], [data-tour="oauth-starting"], [data-tour="oauth-waiting-url"]',
          'left',
          async () => {
            if (!qs('[data-tour="oauth-picker"]') && !qs('[data-tour="oauth-starting"]') && !qs('[data-tour="oauth-waiting-url"]')) {
              await openConnectorAdd('oauth')
            }
          },
        ),
        pop(
          'Add a model',
          'After the account shows Connected, open Models → + Add model. Assign the Main role so chat can use it.',
          '[data-tour="connector-add-model"], [data-tour="model-form"], [data-tour="connector-list"]',
          'left',
          async () => {
            // Return to list if still on oauth detail
            if (qs('[data-tour="connector-back"]') && !qs('[data-tour="connector-list"]')) {
              click('[data-tour="connector-back"]')
              await waitFor('[data-tour="connector-list"]', 2000)
            }
            await ensureConnectorList()
          },
          async () => {
            await openConnectorAdd('model')
          },
        ),
        pop(
          'Model form',
          'Name the model, pick the OAuth connection as Provider, set the model id, enable Main, then Save.',
          '[data-tour="model-form"], [data-tour="form-save"]',
          'left',
          async () => {
            if (!qs('[data-tour="model-form"]')) await openConnectorAdd('model')
          },
        ),
        pop(
          'Select in the composer',
          'With a session open, use the model picker on the chat composer to make this the session Main. (Start a session from the hub if you are still on the start screen.)',
          '[data-tour="model-picker"], [data-tour="composer"], [data-tour="start-screen"]',
          'top',
        ),
      ]

    case 'provider-setup':
      return [
        pop(
          'Open Connector',
          'API-key providers live under Connector → Providers. Opening it…',
          '[data-tour-view="connector"], [data-tour="activity-bar"]',
          'right',
          async () => {
            await ensureSidebarView('connector', '[data-tour="connector-panel"]')
          },
        ),
        pop(
          'Add provider',
          'Click + next to Providers. That opens the marketplace picker.',
          '[data-tour="connector-add-provider"], [data-tour="connector-list"]',
          'left',
          async () => {
            await ensureConnectorList()
          },
          async () => {
            await openConnectorAdd('provider')
          },
        ),
        pop(
          'Choose a preset',
          'Pick OpenRouter, OpenAI, Groq, … or Custom. Selecting one opens the form with endpoint prefilled.',
          '[data-tour="provider-form-pick"], [data-tour="provider-form"]',
          'left',
          async () => {
            if (!qs('[data-tour="provider-form-pick"]') && !qs('[data-tour="provider-form"]')) {
              await openConnectorAdd('provider')
            }
          },
          async () => {
            // If still on pick, open a common preset so the form appears for the next step.
            if (qs('[data-tour="provider-form-pick"]')) {
              const pref =
                qs<HTMLElement>('[data-tour="provider-preset-openrouter"]') ||
                qs<HTMLElement>('[data-tour="provider-preset-openai"]') ||
                qs<HTMLElement>('[data-tour="provider-preset-custom"]')
              pref?.click()
              await waitFor('[data-tour="provider-form"]', 2000)
            }
          },
        ),
        pop(
          'Fill endpoint + API key',
          'Confirm Name and Endpoint, paste your API key, then click Save. Leave key blank only when editing an existing provider to keep the stored key.',
          '[data-tour="provider-form"], [data-tour="provider-api-key"], [data-tour="form-save"]',
          'left',
          async () => {
            if (!qs('[data-tour="provider-form"]')) {
              if (qs('[data-tour="provider-form-pick"]')) {
                qs<HTMLElement>('[data-tour="provider-preset-custom"]')?.click()
                await waitFor('[data-tour="provider-form"]', 2000)
              } else {
                await openConnectorAdd('provider')
              }
            }
          },
        ),
        pop(
          'Add a model',
          'Back on the list, click + next to Models. Bind the model to the provider you just saved and assign Main.',
          '[data-tour="connector-add-model"], [data-tour="model-form"], [data-tour="connector-list"]',
          'left',
          async () => {
            if (qs('[data-tour="connector-back"]') && !qs('[data-tour="connector-list"]')) {
              // Don't auto-save; go back if user already saved, else stay.
              // Prefer list: click back only if form was cancelled/saved.
            }
            // Try to reach list without destroying unsaved form: only back if save not needed for demo
            if (!qs('[data-tour="connector-list"]') && qs('[data-tour="connector-back"]')) {
              click('[data-tour="connector-back"]')
              await waitFor('[data-tour="connector-list"]', 2000)
            }
            await ensureConnectorList()
          },
          async () => {
            await openConnectorAdd('model')
          },
        ),
        pop(
          'Configure the model',
          'Set Name, Provider, Model id (search works for catalogue endpoints), Roles → Main, then Save.',
          '[data-tour="model-form"], [data-tour="model-id"], [data-tour="form-save"]',
          'left',
          async () => {
            if (!qs('[data-tour="model-form"]')) await openConnectorAdd('model')
          },
        ),
        pop(
          'Select in the composer',
          'Open a session, then use the composer model picker to select your new model as session Main.',
          '[data-tour="model-picker"], [data-tour="composer"], [data-tour="start-screen"]',
          'top',
        ),
      ]

    case 'activity-bar':
      return [
        pop(
          'Activity bar',
          'This strip switches sidebar panels. Drag icons to reorder; hide items in Settings → Sidebar. Overflow goes into ⋯.',
          '[data-tour="activity-bar"]',
          'right',
        ),
        pop(
          'Tutorial & Help',
          'Tutorial (coach + tours) and Help sit pinned at the bottom — always available.',
          '[data-tour-open="tutorial"]',
          'right',
        ),
      ]

    case 'sessions-hub':
      return [
        pop(
          'Sessions hub',
          'With no session attached you get the start screen: new session, open folder, resume, remote. Resume also lives in the titlebar search.',
          '[data-tour="start-screen"], main',
          'bottom',
        ),
      ]

    case 'composer':
      return [
        pop(
          'Composer',
          'Type to chat. ! runs a local shell line. Attachments and the model picker sit on the footer row.',
          '[data-tour="composer"], main',
          'top',
          async () => {
            // Best-effort: if start screen, just highlight that.
          },
        ),
        pop(
          'Model picker',
          'Switch the session Main model here (including koma free).',
          '[data-tour="model-picker"], [data-tour="composer"]',
          'top',
        ),
      ]

    case 'connector':
      return [
        pop(
          'Connector',
          'Providers, OAuth, and models. Opening the panel…',
          '[data-tour="connector-panel"], [data-tour-view="connector"]',
          'left',
          async () => {
            await ensureSidebarView('connector', '[data-tour="connector-panel"]')
            await ensureConnectorList()
          },
        ),
        pop(
          'Three catalogues',
          'Providers (+), OAuth / Connect account (+), Models (+). Use the setup tours for full recipes.',
          '[data-tour="connector-list"]',
          'left',
        ),
      ]

    case 'agents':
      return [
        pop(
          'Agents',
          'Built-in and custom sub-agents. Opening the panel…',
          '[data-tour-view="agents"]',
          'right',
          async () => {
            await ensureSidebarView('agents')
          },
        ),
      ]

    case 'git':
      return [
        pop(
          'Source control',
          'Status, stage, commit, diffs, branches, stash, remotes.',
          '[data-tour-view="git"]',
          'right',
          async () => {
            await ensureSidebarView('git')
          },
        ),
      ]

    case 'mcp':
      return [
        pop(
          'MCP',
          'Add Model Context Protocol servers. Tools appear for the agent once connected.',
          '[data-tour-view="mcp"]',
          'right',
          async () => {
            await ensureSidebarView('mcp')
          },
        ),
      ]

    case 'remote':
      return [
        pop(
          'Remote',
          'SSH hosts, connect, remote cwd, attach. Keys live under Settings → SSH Keys.',
          '[data-tour-view="remote"]',
          'right',
          async () => {
            await ensureSidebarView('remote')
          },
        ),
      ]

    case 'store':
      return [
        pop(
          'Extensions',
          'Browse the koma.run store and manage installed extensions.',
          '[data-tour-view="store"]',
          'right',
          async () => {
            await ensureSidebarView('store')
          },
        ),
      ]

    case 'settings':
      return [
        pop(
          'Settings',
          'Theme, appearance, coding, SSH keys, activity-bar layout, account.',
          '[data-tour-open="settings"]',
          'right',
          async () => {
            await openSingleton('settings')
          },
        ),
      ]

    default:
      return []
  }
}

// ─── Runner ─────────────────────────────────────────────────────────────────

let active: Driver | null = null
let runToken = 0

function toDriveSteps(steps: TourStep[]): DriveStep[] {
  return steps.map((s) => ({
    element: s.element,
    popover: {
      title: s.title,
      description: s.description,
      side: s.side ?? 'right',
      align: 'start',
      // onNextClick / onPrevClick set in drive() config via hooks
    },
  }))
}

/** Start a named tour. Destroys any previous driver instance first. */
export function startTour(id: TourId | string): boolean {
  const tourId = TOUR_CATALOGUE.some((t) => t.id === id) ? (id as TourId) : null
  if (!tourId) return false

  const steps = stepsFor(tourId)
  if (steps.length === 0) return false

  // Leave the Tutorial tab so sidebar/composer chrome is visible under the tour.
  try {
    const st = useKoma.getState()
    if (st.ui.activeTabId === 'tutorial') {
      st.closeTab('tutorial')
    }
  } catch {
    /* store may be unavailable in isolation */
  }

  active?.destroy()
  active = null
  const token = ++runToken

  void runTour(token, steps)
  return true
}

async function runTour(token: number, steps: TourStep[]) {
  // Prepare step 0 before creating driver so the first highlight lands on real UI.
  try {
    await steps[0]?.prepare?.()
  } catch {
    /* continue anyway */
  }
  if (token !== runToken) return

  let index = 0

  const moveTo = async (next: number) => {
    if (token !== runToken) return
    if (next < 0 || next >= steps.length) {
      active?.destroy()
      active = null
      return
    }
    // When advancing forward, run previous step's onNext (user confirmed).
    if (next > index) {
      try {
        await steps[index]?.onNext?.()
      } catch {
        /* ignore */
      }
    }
    index = next
    try {
      await steps[index]?.prepare?.()
    } catch {
      /* ignore */
    }
    if (token !== runToken) return
    // Rebuild driver on the new step so element is re-queried after DOM changes.
    rebuild()
  }

  const rebuild = () => {
    if (token !== runToken) return
    active?.destroy()
    const slice = steps.slice(index)
    const driveSteps = toDriveSteps(slice)
    // Only show the current step as a 1-step driver, then we manually chain.
    // This avoids driver.js caching stale elements across React re-renders.
    const current: DriveStep = {
      element: driveSteps[0]?.element,
      popover: {
        ...driveSteps[0]?.popover,
        // Progress across the full tour
        // (driver only knows 1 step — we override progress text)
        description: `${driveSteps[0]?.popover?.description ?? ''}`,
      },
    }

    const isLast = index >= steps.length - 1
    const isFirst = index <= 0

    const buttons: Array<'next' | 'previous' | 'close'> = ['close']
    if (!isFirst) buttons.unshift('previous')
    buttons.push('next')

    active = driver({
      ...BASE,
      steps: [current],
      showButtons: buttons,
      nextBtnText: isLast ? 'Done' : 'Next',
      progressText: `${index + 1} / ${steps.length}`,
      showProgress: true,
      onNextClick: (_el, _step, { driver: d }) => {
        if (isLast) {
          d.destroy()
          active = null
          return
        }
        void moveTo(index + 1)
      },
      onPrevClick: () => {
        void moveTo(Math.max(0, index - 1))
      },
      onCloseClick: (_el, _step, { driver: d }) => {
        d.destroy()
        active = null
        runToken++
      },
    })
    active.drive(0)
  }

  rebuild()
}

export function stopTour() {
  runToken++
  active?.destroy()
  active = null
}

export function tourMeta(id: string | null | undefined): TourMeta | undefined {
  if (!id) return undefined
  return TOUR_CATALOGUE.find((t) => t.id === id)
}
