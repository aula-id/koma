// Guided product tours for the GUI Tutorial tab (driver.js).
// All popover chrome is themed via CSS vars (--koma-*) — see styles.css
// `.koma-driver-theme`. Tours open real surfaces when cheap; otherwise they
// spotlight whatever is already mounted.

import { driver, type DriveStep, type Config } from 'driver.js'
import 'driver.js/dist/driver.css'

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
    blurb: 'Connect a provider with browser sign-in, then pick a model.',
    kind: 'setup',
  },
  {
    id: 'provider-setup',
    title: 'API provider → model',
    blurb: 'Add a custom endpoint + key, create a model, select it.',
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
}

function step(
  sel: string,
  title: string,
  description: string,
  side: 'top' | 'right' | 'bottom' | 'left' = 'right',
): DriveStep {
  return {
    element: sel,
    popover: {
      title,
      description,
      side,
      align: 'start',
    },
  }
}

/** Best-effort open of a sidebar view before a tour that needs it. */
function openSidebarView(view: string) {
  try {
    // Activity bar buttons carry data-tour-view="<view>".
    const btn = document.querySelector(`[data-tour-view="${view}"]`) as HTMLElement | null
    btn?.click()
  } catch {
    /* ignore */
  }
}

function openSingleton(kind: 'settings' | 'help' | 'tutorial') {
  try {
    const btn = document.querySelector(`[data-tour-open="${kind}"]`) as HTMLElement | null
    btn?.click()
  } catch {
    /* ignore */
  }
}

function stepsFor(id: TourId): DriveStep[] {
  switch (id) {
    case 'oauth-setup':
      return [
        step(
          '[data-tour-view="connector"]',
          'Open Connector',
          'Providers, OAuth, and models live here. Click Connector on the activity bar.',
        ),
        step(
          '[data-tour="connector-panel"], [data-tour-view="connector"]',
          'OAuth section',
          'In Connector, expand OAuth and pick a provider (Codex, Claude, koma.run, …). Sign-in opens your browser — koma never sees your password.',
        ),
        step(
          '[data-tour="connector-panel"], [data-tour-view="connector"]',
          'Add or pick a model',
          'After the account shows Connected, add a model (or use one the provider exposes) and assign the Main role.',
        ),
        step(
          '[data-tour="model-picker"], [data-tour="composer"]',
          'Select in the composer',
          'Open the model picker on the chat composer and choose the new model for this session.',
          'top',
        ),
      ]
    case 'provider-setup':
      return [
        step(
          '[data-tour-view="connector"]',
          'Open Connector',
          'Custom API-key providers are managed in Connector → Providers.',
        ),
        step(
          '[data-tour="connector-panel"], [data-tour-view="connector"]',
          'Add provider',
          'Choose Add provider, pick a preset or Custom, enter endpoint + API key, then save.',
        ),
        step(
          '[data-tour="connector-panel"], [data-tour-view="connector"]',
          'Add model',
          'Under Models, add a global Main model bound to that provider (live catalogue search when the endpoint supports it).',
        ),
        step(
          '[data-tour="model-picker"], [data-tour="composer"]',
          'Select in the composer',
          'Use the composer model picker so the session Main points at your new model.',
          'top',
        ),
      ]
    case 'activity-bar':
      return [
        step(
          '[data-tour="activity-bar"]',
          'Activity bar',
          'This strip switches sidebar panels. Drag to reorder; hide items from Settings → Sidebar. Overflow goes into ⋯.',
        ),
        step(
          '[data-tour-open="tutorial"]',
          'Tutorial & Help',
          'Tutorial (this tab) and Help sit pinned at the bottom — always available, not reorderable.',
        ),
      ]
    case 'sessions-hub':
      return [
        step(
          '[data-tour="sessions-hub"], [data-tour="start-screen"], main',
          'Sessions hub',
          'With no session attached you get the start screen: new session, open folder, resume, and remote. Resume also lives in the titlebar search.',
        ),
      ]
    case 'composer':
      return [
        step(
          '[data-tour="composer"], main',
          'Composer',
          'Type to chat. ! runs a local shell line. @ references files. Attachments and the model picker sit on the footer row.',
          'top',
        ),
      ]
    case 'connector':
      return [
        step(
          '[data-tour-view="connector"]',
          'Connector',
          'Single place for providers, OAuth accounts, and models. The free tier is picker-only — it does not appear as an editable provider row.',
        ),
      ]
    case 'agents':
      return [
        step(
          '[data-tour-view="agents"]',
          'Agents',
          'Browse built-in and custom sub-agents. Open a row to edit prompts and tools; spawn from chat with the task tool.',
        ),
      ]
    case 'git':
      return [
        step(
          '[data-tour-view="git"]',
          'Source control',
          'Status, stage, commit, diffs, branches, stash, and remotes — host-local git, same repo as your session workdir.',
        ),
      ]
    case 'mcp':
      return [
        step(
          '[data-tour-view="mcp"]',
          'MCP',
          'Add Model Context Protocol servers (stdio or HTTP). Tools show up for the agent once the server is connected.',
        ),
      ]
    case 'remote':
      return [
        step(
          '[data-tour-view="remote"]',
          'Remote',
          'Save SSH hosts, connect, pick a remote cwd, and attach a remote session. Keys live under Settings → SSH Keys.',
        ),
      ]
    case 'store':
      return [
        step(
          '[data-tour-view="store"]',
          'Extensions',
          'Browse the koma.run store and manage installed extensions. Some contribute activity-bar panels of their own.',
        ),
      ]
    case 'settings':
      return [
        step(
          '[data-tour-open="settings"]',
          'Settings',
          'Theme, appearance, coding autosave, SSH keys, activity-bar layout, and account links.',
        ),
      ]
    default:
      return []
  }
}

let active: ReturnType<typeof driver> | null = null

/** Start a named tour. Destroys any previous driver instance first. */
export function startTour(id: TourId | string): boolean {
  const tourId = TOUR_CATALOGUE.some((t) => t.id === id) ? (id as TourId) : null
  if (!tourId) return false

  // Nudge UI toward the right surface before highlighting.
  switch (tourId) {
    case 'oauth-setup':
    case 'provider-setup':
    case 'connector':
      openSidebarView('connector')
      break
    case 'agents':
      openSidebarView('agents')
      break
    case 'git':
      openSidebarView('git')
      break
    case 'mcp':
      openSidebarView('mcp')
      break
    case 'remote':
      openSidebarView('remote')
      break
    case 'store':
      openSidebarView('store')
      break
    case 'settings':
      openSingleton('settings')
      break
    default:
      break
  }

  active?.destroy()
  const steps = stepsFor(tourId).filter((s) => {
    if (typeof s.element === 'string') {
      // Keep steps whose selector might match later; driver handles missing lightly.
      return true
    }
    return true
  })
  if (steps.length === 0) return false

  active = driver({
    ...BASE,
    steps,
    onDestroyStarted: () => {
      active?.destroy()
      active = null
    },
  })
  // Small delay so sidebar panel mount can paint.
  window.setTimeout(() => active?.drive(), 120)
  return true
}

export function stopTour() {
  active?.destroy()
  active = null
}

export function tourMeta(id: string | null | undefined): TourMeta | undefined {
  if (!id) return undefined
  return TOUR_CATALOGUE.find((t) => t.id === id)
}
