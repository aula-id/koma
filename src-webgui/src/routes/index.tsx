import { createRootRoute, createRoute, Outlet } from '@tanstack/react-router'
import { lazy, Suspense, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { ChatView } from '../components/ChatView'
import { TAB_DRAG_MIME, TabBar } from '../components/TabBar'
import { StartScreen } from '../components/StartScreen'
import { Onboarding } from '../components/Onboarding'
import { Titlebar, getPlatform } from '../components/Titlebar'
import { ResizeHandles } from '../components/ResizeHandles'
import { ActivityBar } from '../components/ActivityBar'
import { Sidebar, type SidebarView } from '../components/Sidebar'
import { ResumePalette } from '../components/ResumePalette'
import { RenameOverlay } from '../components/RenameOverlay'
import { OmniSearchPalette } from '../components/OmniSearchPalette'
import { SwitchingOverlay } from '../components/SwitchingOverlay'
import { RemotePasswordPrompt } from '../components/RemotePasswordPrompt'
import { RemotePathPicker } from '../components/RemotePathPicker'
import { ToastContainer } from '../components/ToastContainer'
import { UsageFooter } from '../components/UsageFooter'
import { ProblemsDrawer } from '../components/ProblemsDrawer'
import { LspDrawer } from '../components/LspDrawer'
import { useKoma } from '../store/koma'
import { BrailleSpinner } from '../components/BrailleSpinner'
import { ExtensionPanelFrame } from '../components/ExtensionPanelFrame'
import { GlobalContextMenu } from '../components/GlobalContextMenu'
import { installPanelBridgeListener } from '../lib/panelBridge'
import { hasCodingPathDrag, readCodingPathDragData } from '../lib/codingRef'
import {
  MAX_GROUPS,
  dropZoneFor,
  gridLayout,
  groupOf,
  isTabVisible,
  normalizeGroups,
  type DropZone,
  type EditorGroupId,
} from '../store/editorGroups'
import type { Tab } from '../store/koma'

function isEditorBodyDrag(dt: DataTransfer | null | undefined): boolean {
  if (!dt?.types) return false
  return dt.types.includes(TAB_DRAG_MIME) || hasCodingPathDrag(dt)
}

const SIDEBAR_MIN = 150
const SIDEBAR_MAX = 500

// Shared first-run gate — onboarding takes precedence over everything and must
// not be bypassable. Both RootLayout (to suppress ALL chrome + overlays) and
// IndexPage (content) read this same signal. Host's authoritative firstRun flag
// when present, else inferred from an unconfigured config (no provider, or no
// Main-role model). Gated on `loaded` so it never flashes before the first
// Config push.
function useNeedsOnboarding() {
  const sessionId = useKoma((s) => s.session.id)
  const loaded = useKoma((s) => s.config.loaded)
  const firstRun = useKoma((s) => s.config.firstRun)
  const providers = useKoma((s) => s.config.providers)
  const models = useKoma((s) => s.config.models)
  const oauthConns = useKoma((s) => s.oauth.conns)
  // A model's provider can be a real config provider OR an OAuth connection —
  // the daemon resolves `provider_uuid` against either catalogue (see
  // ConnectorPanel/Onboarding's providerOptions) — so an OAuth-only setup
  // (zero config.providers, one live connection) must count as "has a
  // provider" too. Without this, this FALLBACK-only check (only ever
  // consulted when the host omits its own authoritative `firstRun`) would
  // keep onboarding stuck forever for a user whose only "provider" is an
  // OAuth connection, even after they save a perfectly usable OAuth-backed
  // Main model.
  const configured =
    (providers.length > 0 || oauthConns.length > 0) && models.some((m) => m.roles.includes('main'))
  return loaded && sessionId === null && (firstRun ?? !configured)
}

function RootLayout() {
  // Resolved once — window.__komaOS is injected by the Rust host before the app
  // boots and never changes for the lifetime of the window.
  const [platform] = useState(getPlatform)
  const [overlay, setOverlay] = useState<'none' | 'resume' | 'rename'>('none')
  const [activeView, setActiveView] = useState<SidebarView>('explore')
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [sidebarWidth, setSidebarWidth] = useState(240)
  // Omnisearch is opened from the Composer, which lives under a different
  // route subtree than this layout — kept in the store (not local state) so
  // it's reachable without prop drilling; see koma.ts's ui slice.
  const omnisearchOpen = useKoma((s) => s.ui.omnisearchOpen)
  const closeOmniSearch = useKoma((s) => s.closeOmniSearch)
  const cancelSwitching = useKoma((s) => s.cancelSwitching)
  const remoteState = useKoma((s) => s.remoteState)
  const remotePathState = useKoma((s) => s.remotePath.state)
  const remotePathOpen =
    remotePathState === 'listing' ||
    remotePathState === 'ready' ||
    remotePathState === 'error'
  const req = useKoma((s) => s.req)
  const openSettingsTab = useKoma((s) => s.openSettingsTab)
  const openHelpTab = useKoma((s) => s.openHelpTab)
  const openTutorialTab = useKoma((s) => s.openTutorialTab)
  const openTerminalTab = useKoma((s) => s.openTerminalTab)
  // Counter for generating unique terminal IDs.
  const terminalCountRef = useRef(0)
  const needsOnboarding = useNeedsOnboarding()
  // Cross-tree signal from the UsageFooter PLAN badge click (see koma.ts's
  // `focusPlanTick`): switch the sidebar to the Explore view and ensure it's
  // open. ExplorePanel's own effect on the same tick expands its PLAN section.
  const focusPlanTick = useKoma((s) => s.ui.focusPlanTick)
  useEffect(() => {
    if (focusPlanTick === 0) return
    setActiveView('explore')
    setSidebarOpen(true)
  }, [focusPlanTick])

  // Terminal button handler: each click creates a new terminal tab with a unique ID.
  // Host opens a remote shell (ssh -t) when remote hub/session is live; local otherwise.
  const handleTerminal = () => {
    terminalCountRef.current += 1
    const n = terminalCountRef.current
    const rs = useKoma.getState().remoteState
    const remoteLive = rs.state === 'ready' || rs.state === 'connected'
    const hostLabel =
      remoteLive && rs.user && rs.host ? `${rs.user}@${rs.host}` : null
    const title = hostLabel
      ? n === 1
        ? hostLabel
        : `${hostLabel} ${n}`
      : n === 1
        ? 'Terminal'
        : `Terminal ${n}`
    const id = `t${Date.now()}`
    openTerminalTab(id, title)
  }

  // Wire the JS <-> Rust bridge: expose window.__komaClient.push so the host
  // can feed the koma store, then announce readiness so it sends the first
  // push (Hub if swapper else Snapshot). Also expose window.komaIpc for
  // fire-and-forget IPC calls (e.g., error logging).
  useEffect(() => {
    // Coalesce high-frequency Stream/Reasoning/Status envelopes to one store
    // apply per animation frame. Structural envelopes (Snapshot*, Hub, …) flush
    // immediately so UI never lags a commit behind a batched token tick.
    type Env = Parameters<NonNullable<typeof window.__komaClient>['push']> extends
      never
      ? never
      : any
    let raf = 0
    const pending: any[] = []
    let pacedRaf = 0
    const paced: any[] = []
    // Only these attach envelopes are frame-paced. Stream/Status/chat traffic
    // must never sit behind Snapshot — that jammed the composer after switch.
    const isPacedKind = (k: string) =>
      k === 'Switching' ||
      k === 'Loading' ||
      k === 'Snapshot' ||
      k === 'SnapshotTail' ||
      k === 'SnapshotHead' ||
      k === 'HistoryPage' ||
      k === 'Config' ||
      k === 'SettingsValues' ||
      k === 'RepoList'
    const isCoalesce = (k: string) =>
      k === 'StreamDelta' ||
      k === 'StreamMsg' ||
      k === 'ReasoningDelta' ||
      k === 'Reasoning' ||
      k === 'Status'

    const logPace = (msg: string) => {
      try {
        window.komaIpc?.({ r: 'WriteErrorLog', message: `[ui-pace] ${msg}` })
      } catch {
        /* fire-and-forget */
      }
    }

    const peekKind = (item: string | object): string | null => {
      if (item && typeof item === 'object' && 'k' in item) {
        const k = (item as { k?: unknown }).k
        return typeof k === 'string' ? k : null
      }
      if (typeof item === 'string') {
        const m = item.match(/"k"\s*:\s*"([^"]+)"/)
        return m?.[1] ?? null
      }
      return null
    }

    const flush = () => {
      raf = 0
      if (pending.length === 0) return
      // Collapse stream/reasoning deltas: apply in order but one React tick
      // by calling push back-to-back inside the rAF (zustand batches sync sets
      // in the same event loop turn when using the default path).
      const batch = pending.splice(0, pending.length)
      const push = useKoma.getState().push
      for (const env of batch) push(env)
    }

    // During a session attach, apply one *heavy* host envelope per browser
    // frame so Snapshot/Loading/Config don't become one giant React task.
    // Chat stream envelopes always take the coalesce path — never this queue.
    const flushPaced = () => {
      pacedRaf = 0
      const item = paced.shift()
      if (!item) return
      const t0 = performance.now()
      const env = typeof item === 'string' ? JSON.parse(item) : item
      const kind = typeof env?.k === 'string' ? env.k : '?'
      const rawLen = typeof item === 'string' ? item.length : 0
      useKoma.getState().push(env)
      const ms = performance.now() - t0
      if (
        ms >= 8 ||
        kind === 'Snapshot' ||
        kind === 'SnapshotHead' ||
        kind === 'HistoryPage' ||
        paced.length > 0
      ) {
        logPace(
          `apply k=${kind} parse+push_ms=${ms.toFixed(1)} raw_bytes=${rawLen} queue_left=${paced.length}`,
        )
      }
      if (paced.length > 0) pacedRaf = requestAnimationFrame(flushPaced)
    }

    const enqueuePaced = (item: string | object) => {
      if (pending.length) flush()
      paced.push(item)
      if (!pacedRaf) pacedRaf = requestAnimationFrame(flushPaced)
    }

    const enqueue = (env: any) => {
      if (env && typeof env === 'object' && typeof env.k === 'string' && isPacedKind(env.k)) {
        enqueuePaced(env)
        return
      }
      if (env && typeof env === 'object' && isCoalesce(env.k)) {
        pending.push(env)
        if (!raf) raf = requestAnimationFrame(flush)
        return
      }
      // Structural: drain any pending light envelopes first so order is preserved
      // (e.g. last StreamDelta before Snapshot clear).
      if (pending.length) flush()
      useKoma.getState().push(env)
    }

    window.__komaClient = {
      // Host may pass a JSON string (legacy double-encode) OR a pre-parsed
      // object (cheaper inject path).
      push: (j) => {
        const env = typeof j === 'string' ? JSON.parse(j) : j
        enqueue(env)
      },
      // Native GUI host path: one evaluate_script carries every envelope that
      // accumulated in its 16 ms frame window. Rust sends JSON strings so a fat
      // Snapshot is not parsed until its own paced frame.
      pushBatch: (items) => {
        logPace(`batch n=${items.length}`)
        for (const item of items) {
          const kind = peekKind(item)
          if (kind && isPacedKind(kind)) {
            enqueuePaced(item)
          } else {
            const env = typeof item === 'string' ? JSON.parse(item) : item
            enqueue(env)
          }
        }
      },
    }
    // komaIpc is for fire-and-forget requests that don't need a reply
    window.komaIpc = (g) => {
      const ipc = window.ipc
      if (ipc && typeof ipc.postMessage === 'function') {
        try {
          ipc.postMessage(JSON.stringify({ t: 'req', ...g }))
        } catch {
          // ignore IPC errors for fire-and-forget calls
        }
      }
    }
    useKoma.getState().req({ r: 'Ready' })
    // Defer host-local side fetches until after the first Hub/Config paint.
    requestAnimationFrame(() => {
      useKoma.getState().req({ r: 'GitStatus' })
      useKoma.getState().req({ r: 'GetRemoteHosts' })
      useKoma.getState().refreshInstalled()
    })
    // Extension panel bridge (W9): single window-level `message` listener
    // that attributes + forwards panel iframe traffic — see
    // lib/panelBridge.ts. Idempotent, but installed here alongside the rest
    // of the JS<->Rust bridge setup so exactly one listener exists.
    const uninstallPanelBridge = installPanelBridgeListener()
    return () => {
      if (raf) cancelAnimationFrame(raf)
      if (pacedRaf) cancelAnimationFrame(pacedRaf)
      window.__komaClient = undefined
      window.komaIpc = undefined
      uninstallPanelBridge()
    }
  }, [])

  // Global Ctrl+B (Cmd+B on mac): background EVERY eligible running sub-agent at
  // once — mirrors the TUI composer's Ctrl+B (`Action::BackgroundAllSubagents`).
  // Fires from anywhere in the app, including a focused composer textarea (that
  // matches the TUI, whose composer is exactly where Ctrl+B is bound), EXCEPT
  // inside a Monaco diff-tab editor, where Ctrl+B/Cmd+B are the editor's own
  // bindings. Reads eligibility fresh off the store (running && !detached &&
  // blocking) rather than subscribing, so this effect never needs to re-run;
  // `preventDefault` only fires when we actually act, so the combo is never
  // swallowed for nothing (e.g. no sub-agents at all).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== 'b' || e.altKey || e.shiftKey) return
      const wantsMod = platform === 'macos' ? e.metaKey : e.ctrlKey
      if (!wantsMod) return
      const target = e.target as HTMLElement | null
      if (target?.closest?.('.monaco-editor')) return
      const eligible = useKoma
        .getState()
        .session.subagents.some((a) => a.status === 'running' && !a.detached && a.blocking)
      if (!eligible) return
      e.preventDefault()
      req({ r: 'BackgroundAll' })
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [platform, req])

  // Global Ctrl+R (Cmd+R on mac): resend the last user turn — mirrors the TUI's
  // Ctrl+R (`Action::Resend`), idle-only (the daemon's own `handle_resend` guards
  // busy too, but gating here avoids firing a request it would just bounce off
  // a status line). `preventDefault` fires UNCONDITIONALLY on the combo (not just
  // when we act): Ctrl+R is the browser's page-reload shortcut, and reloading the
  // webview mid-session would nuke the whole client — that must never happen,
  // working or not. Same Monaco-editor exemption as the Ctrl+B handler above (the
  // diff tab's own Ctrl+R binding, if any, keeps working there).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== 'r' || e.altKey || e.shiftKey) return
      const wantsMod = platform === 'macos' ? e.metaKey : e.ctrlKey
      if (!wantsMod) return
      const target = e.target as HTMLElement | null
      if (target?.closest?.('.monaco-editor')) return
      e.preventDefault()
      if (useKoma.getState().session.working) return
      req({ r: 'Resend' })
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [platform, req])

  // Click the active view's icon to collapse/expand; click another to switch to
  // it (and ensure the sidebar is open).
  const selectView = (view: SidebarView) => {
    if (view === activeView) {
      setSidebarOpen((o) => !o)
    } else {
      setActiveView(view)
      setSidebarOpen(true)
    }
  }

  // Drag the divider between the sidebar and the terminal to resize. Tracks the
  // mouse on window (so the drag survives leaving the 5px handle) and clamps the
  // width. The terminal's own ResizeObserver refits xterm as space changes.
  const startResize = (e: ReactMouseEvent) => {
    e.preventDefault()
    const startX = e.clientX
    const startW = sidebarWidth
    const onMove = (ev: MouseEvent) => {
      const next = Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, startW + ev.clientX - startX))
      setSidebarWidth(next)
    }
    const onUp = () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    document.body.style.cursor = 'ew-resize'
    document.body.style.userSelect = 'none'
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }

  // Onboarding is un-bypassable: when first-run, render ONLY the onboarding flow
  // and mount NO other chrome — no activity bar, sidebar, session-switcher,
  // rename, or session overlays exist or are reachable. The window frame stays
  // (drag/traffic-lights/resize); its cmdbar is hidden via overlayOpen so the
  // change-session/rename pills disappear. The bridge useEffect above still runs,
  // so host Config pushes during setup keep flowing.
  if (needsOnboarding) {
    return (
      <div id="app" className={`os-${platform}`}>
        <Titlebar onSearch={() => {}} onRename={() => {}} onTerminal={handleTerminal} overlayOpen />
        <div className="absolute inset-x-0 top-8 bottom-0 overflow-hidden">
          <Onboarding />
        </div>
        <ToastContainer />
        <ResizeHandles />
      </div>
    )
  }

  return (
    <div id="app" className={`os-${platform}`}>
      <Titlebar
        onSearch={() => setOverlay('resume')}
        onRename={() => setOverlay('rename')}
        onTerminal={handleTerminal}
        // Hide cmdbar pills while resume/rename OR remote cwd picker is open —
        // same morph handoff rename uses (layoutId stays free for the overlay).
        overlayOpen={overlay !== 'none' || remotePathOpen}
      />
      <div className="absolute inset-x-0 top-8 bottom-0 flex overflow-hidden">
        <ActivityBar
          activeView={activeView}
          sidebarOpen={sidebarOpen}
          onSelect={selectView}
          onSettings={openSettingsTab}
          onHelp={openHelpTab}
          onTutorial={openTutorialTab}
        />
        {sidebarOpen && (
          <>
            <Sidebar width={sidebarWidth} view={activeView} />
            <div
              onMouseDown={startResize}
              className="w-[5px] flex-none cursor-ew-resize hover:bg-koma-grip"
            />
          </>
        )}
        <main className="relative flex min-w-0 flex-1 items-stretch justify-center">
          <Outlet />
        </main>
      </div>
      {overlay === 'resume' && (
        <ResumePalette onClose={() => setOverlay('none')} />
      )}
      {overlay === 'rename' && <RenameOverlay onClose={() => setOverlay('none')} />}
      {omnisearchOpen && <OmniSearchPalette onClose={closeOmniSearch} />}
      <SwitchingOverlay
        onCancel={() => {
          // Best-effort bail: the in-flight swap can't be interrupted, so
          // tell the host to drop back to the swapper once the target lands
          // (Rust GuiReq::CancelSwitch), then drop the loader and reopen the
          // hub locally so the user can pick again.
          req({ r: 'CancelSwitch' })
          cancelSwitching()
          setOverlay('resume')
        }}
      />
      <RemotePasswordPrompt
        active={remoteState.state === 'auth_required'}
        target={remoteState.user && remoteState.host ? `${remoteState.user}@${remoteState.host}` : null}
        onSubmit={(password) => req({ r: 'SubmitRemotePassword', password })}
        onCancel={() => {
          req({ r: 'CancelRemoteConnect' })
          if (useKoma.getState().ui.switchingTo) {
            req({ r: 'CancelSwitch' })
            cancelSwitching()
          }
        }}
      />
      <RemotePathPicker />
      <ToastContainer />
      <ResizeHandles />
      {/* Hide while remote cwd picker is open — Ctrl+Enter / right-click must
          not surface the global copy/paste/resume menu over the path dialog. */}
      <GlobalContextMenu
        onResume={() => setOverlay('resume')}
        hidden={needsOnboarding || remotePathOpen}
      />
    </div>
  )
}

// Monaco DiffEditor is HEAVY — lazy so its chunk never loads until the first
// diff tab is opened (a tiny spinner covers the one-time chunk fetch).
const DiffTab = lazy(() => import('../components/DiffTab'))

// Settings page — lazy so its chunk only loads when the gear is first clicked.
const SettingsTab = lazy(() => import('../components/SettingsTab'))

// Help page — lazy so its chunk only loads when the (?) button is first clicked.
const HelpTab = lazy(() => import('../components/HelpTab'))

// Tutorial coach — lazy so driver.js + chat UI only load when first opened.
const TutorialTab = lazy(() => import('../components/TutorialTab'))

// Per-agent editor — lazy so its chunk only loads when the Agents panel's
// first row (or "+ Add agent") is clicked.
const AgentTab = lazy(() => import('../components/AgentTab'))

// Read-only stream tab (sub-agent transcript / bash output) — lazy so its chunk only
// loads when the first stream tab is opened from the Explorer.
const StreamTab = lazy(() => import('../components/StreamTab'))

// GitKraken-style commit-graph tab — lazy so its chunk (layout engine + the
// virtualized SVG gutter) only loads when the graph is first opened.
const GraphTab = lazy(() => import('../components/GraphTab'))

// Import-graph tab — lazy so its chunk (layout engine + SVG canvas) only loads
// when the import graph is first opened.
const ImportGraphTab = lazy(() => import('../components/ImportGraphTab'))

// Analytics dashboard tab — lazy so its chunk only loads when first opened.
const AnalyticsTab = lazy(() => import('../components/AnalyticsTab'))

// Extension STORE tab — lazy so its chunk only loads when the store is first
// opened from the ActivityBar.
const StoreTab = lazy(() => import('../components/StoreTab'))
const InstalledExtensionTab = lazy(() => import('../components/InstalledExtensionTab'))

// Coding panel Monaco editor — lazy so its chunk only loads when a file is opened.
const CodeEditorTab = lazy(() => import('../components/CodeEditorTab'))

// Interactive terminal tab — lazy so its chunk (xterm.js) only loads when the
// first terminal is opened from the Titlebar.
const TerminalTab = lazy(() => import('../components/TerminalTab').then(m => ({ default: m.TerminalTab })))

function DiffFallback() {
  return (
    <div className="flex h-full w-full items-center justify-center text-koma-dim">
      <BrailleSpinner size={18} className="opacity-70" />
    </div>
  )
}

function TabBody({ tab }: { tab: Exclude<Tab, { kind: 'chat' }> }) {
  return (
    <Suspense fallback={<DiffFallback />}>
      {tab.kind === 'diff' ? (
        <DiffTab tab={tab} />
      ) : tab.kind === 'settings' ? (
        <SettingsTab />
      ) : tab.kind === 'help' ? (
        <HelpTab />
      ) : tab.kind === 'tutorial' ? (
        <TutorialTab />
      ) : tab.kind === 'agent' ? (
        <AgentTab tab={tab} />
      ) : tab.kind === 'subagent' || tab.kind === 'bash' ? (
        <StreamTab tab={tab} />
      ) : tab.kind === 'graph' ? (
        <GraphTab />
      ) : tab.kind === 'importGraph' ? (
        <ImportGraphTab />
      ) : tab.kind === 'analytics' ? (
        <AnalyticsTab />
      ) : tab.kind === 'store' ? (
        <StoreTab />
      ) : tab.kind === 'installedExtension' ? (
        <InstalledExtensionTab extId={tab.extId} />
      ) : tab.kind === 'extension' ? (
        <ExtensionPanelFrame extId={tab.extId} panelId={tab.panelId} title={tab.title} />
      ) : tab.kind === 'codingFile' ? (
        <CodeEditorTab tab={tab} />
      ) : tab.kind === 'terminal' ? (
        <TerminalTab tab={tab} />
      ) : null}
    </Suspense>
  )
}

type DropHover = { groupId: EditorGroupId; zone: DropZone }

function zoneHighlightClass(zone: DropZone): string {
  switch (zone) {
    case 'left':
      return 'inset-y-2 left-2 w-[48%]'
    case 'right':
      return 'inset-y-2 right-2 w-[48%]'
    case 'top':
      return 'inset-x-2 top-2 h-[48%]'
    case 'bottom':
      return 'inset-x-2 bottom-2 h-[48%]'
    default:
      return 'inset-2'
  }
}

/** Paint-only drop affordance — never receives pointer events. */
function EditorDropHighlight({
  groupId,
  hover,
}: {
  groupId: EditorGroupId
  hover: DropHover | null
}) {
  if (hover?.groupId !== groupId) return null
  return (
    <div
      className={`pointer-events-none absolute ${zoneHighlightClass(hover.zone)} rounded border border-koma-accent bg-koma-accent/15`}
    />
  )
}

// A single CSS grid hosts every group strip, every tab body, and every divider.
// Tab bodies stay siblings even when moved: only their grid coordinates change,
// so React never remounts chat, Monaco, xterm, streams, or extension iframes.
function TabbedMain() {
  const rawUi = useKoma((s) => s.ui)
  const ui = useMemo(() => normalizeGroups(rawUi), [rawUi])
  const sessionId = useKoma((s) => s.session.id)
  const focusGroup = useKoma((s) => s.focusEditorGroup)
  const splitTab = useKoma((s) => s.splitTab)
  const toggleSplitDir = useKoma((s) => s.toggleSplitDir)
  const resizeGroups = useKoma((s) => s.resizeEditorGroups)
  const gridRef = useRef<HTMLDivElement>(null)
  // Pane content boxes registered by group id — used for document-level hit
  // testing so we never mount a pointer-events layer over Monaco/xterm.
  const paneEls = useRef(new Map<EditorGroupId, HTMLElement>())
  const [dropHover, setDropHover] = useState<DropHover | null>(null)
  const dropHoverRef = useRef<DropHover | null>(null)
  const sessionRef = useRef(false)
  const moveTab = useKoma((s) => s.moveTabToGroup)
  const openCodingFile = useKoma((s) => s.openCodingFile)
  // splitTab already selected above for the keyboard shortcut.
  const layout = useMemo(
    () => gridLayout(ui.groups, ui.groupSizes, ui.splitDir),
    [ui.groupSizes, ui.groups, ui.splitDir],
  )
  const cells = useMemo(
    () => new Map(layout.cells.map((cell) => [cell.id, cell])),
    [layout.cells],
  )
  const chatGroupId = groupOf(ui, 'chat')

  const setPaneEl = (id: EditorGroupId, el: HTMLElement | null) => {
    if (el) paneEls.current.set(id, el)
    else paneEls.current.delete(id)
  }

  const endDragSession = () => {
    sessionRef.current = false
    dropHoverRef.current = null
    setDropHover(null)
  }

  const hitTestPane = (clientX: number, clientY: number): DropHover | null => {
    for (const cell of layout.cells) {
      const el = paneEls.current.get(cell.id)
      if (!el) continue
      const r = el.getBoundingClientRect()
      if (
        clientX < r.left ||
        clientX >= r.right ||
        clientY < r.top ||
        clientY >= r.bottom
      ) {
        continue
      }
      const zone = dropZoneFor(clientX - r.left, clientY - r.top, r.width, r.height, {
        allowEdges: ui.groups.length < MAX_GROUPS,
      })
      return { groupId: cell.id, zone }
    }
    return null
  }

  useEffect(() => {
    const arm = (e: DragEvent) => {
      // Tab moves and coding-tree file opens arm document-level drop handling
      // so Monaco never sees the bare text/plain path fallback — and so we never
      // need a full-pane hit layer that can stick and steal focus.
      if (!isEditorBodyDrag(e.dataTransfer)) return
      sessionRef.current = true
      dropHoverRef.current = null
      setDropHover(null)
    }

    const stop = () => {
      if (!sessionRef.current && dropHoverRef.current == null) return
      endDragSession()
    }

    const onDragOver = (e: DragEvent) => {
      if (!sessionRef.current || !isEditorBodyDrag(e.dataTransfer)) return
      const hit = hitTestPane(e.clientX, e.clientY)
      if (!hit) {
        if (dropHoverRef.current != null) {
          dropHoverRef.current = null
          setDropHover(null)
        }
        return
      }
      // Accept the drop so the OS cursor shows move/copy over the pane.
      e.preventDefault()
      if (e.dataTransfer) {
        e.dataTransfer.dropEffect = e.dataTransfer.types.includes(TAB_DRAG_MIME)
          ? 'move'
          : 'copy'
      }
      const prev = dropHoverRef.current
      if (prev?.groupId === hit.groupId && prev.zone === hit.zone) return
      dropHoverRef.current = hit
      setDropHover(hit)
    }

    const onDrop = (e: DragEvent) => {
      if (!sessionRef.current || !isEditorBodyDrag(e.dataTransfer)) {
        stop()
        return
      }
      // Geometry only — never fall back to the last hover. A strip drop leaves
      // dropHoverRef pointing at a pane the pointer already left.
      const hit = hitTestPane(e.clientX, e.clientY)
      if (!hit) {
        stop()
        return
      }
      e.preventDefault()
      e.stopPropagation()

      const dropZone = hit.zone
      const side = dropZone === 'left' || dropZone === 'top' ? 'before' : 'after'
      const dir = dropZone === 'left' || dropZone === 'right' ? 'row' : 'col'
      const split =
        dropZone === 'center'
          ? undefined
          : { side: side as 'before' | 'after', dir: dir as 'row' | 'col' }

      const dt = e.dataTransfer
      if (!dt) {
        endDragSession()
        return
      }
      const coding = readCodingPathDragData(dt)
      if (coding) {
        if (!coding.isDir) {
          openCodingFile(coding.root, coding.path, {
            groupId: hit.groupId,
            split,
          })
        }
        endDragSession()
        return
      }

      const tabId = dt.getData(TAB_DRAG_MIME)
      if (tabId) {
        if (dropZone === 'center') {
          moveTab(tabId, hit.groupId)
        } else {
          splitTab(tabId, hit.groupId, side, dir)
        }
      }
      endDragSession()
    }

    // WebKitGTK sometimes skips dragend after Esc / cancelled drags. Escape and
    // blur hard-clear the paint so a ghost highlight never lingers.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') stop()
    }

    window.addEventListener('dragstart', arm)
    window.addEventListener('dragend', stop)
    window.addEventListener('dragover', onDragOver)
    window.addEventListener('drop', onDrop)
    window.addEventListener('blur', stop)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('dragstart', arm)
      window.removeEventListener('dragend', stop)
      window.removeEventListener('dragover', onDragOver)
      window.removeEventListener('drop', onDrop)
      window.removeEventListener('blur', stop)
      window.removeEventListener('keydown', onKey)
    }
  }, [layout.cells, moveTab, openCodingFile, splitTab, ui.groups.length])

  // Split (Ctrl/Cmd+\): create the second pane, or flip axis when already split.
  // Group focus is Ctrl/Cmd+1..2 only (max two panes).
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return
      if (e.key === '\\') {
        if (ui.groups.length >= 2) {
          e.preventDefault()
          toggleSplitDir()
          return
        }
        if (ui.activeTabId === 'chat' || ui.groups.length >= MAX_GROUPS) return
        e.preventDefault()
        splitTab(ui.activeTabId, ui.activeGroupId, 'after', 'row')
        return
      }
      const index = Number(e.key) - 1
      const groupId = ui.groups[index]
      if (index >= 0 && index < MAX_GROUPS && groupId) {
        e.preventDefault()
        focusGroup(groupId)
      }
    }
    window.addEventListener('keydown', key)
    return () => window.removeEventListener('keydown', key)
  }, [focusGroup, splitTab, toggleSplitDir, ui.activeGroupId, ui.activeTabId, ui.groups])

  const startResize = (index: number, e: ReactMouseEvent) => {
    e.preventDefault()
    let prev = ui.splitDir === 'row' ? e.clientX : e.clientY
    const total =
      ui.splitDir === 'row'
        ? (gridRef.current?.clientWidth ?? 1)
        : (gridRef.current?.clientHeight ?? 1)
    const move = (ev: MouseEvent) => {
      const next = ui.splitDir === 'row' ? ev.clientX : ev.clientY
      resizeGroups(index, next - prev, total)
      prev = next
    }
    const up = () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    document.body.style.cursor = ui.splitDir === 'row' ? 'ew-resize' : 'ns-resize'
    document.body.style.userSelect = 'none'
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
  }

  return (
    <div className="flex h-full w-full min-w-0 flex-col">
      <div
        ref={gridRef}
        className="grid min-h-0 min-w-0 flex-1 overflow-hidden"
        style={{
          gridTemplateColumns: layout.gridTemplateColumns,
          gridTemplateRows: layout.gridTemplateRows,
        }}
      >
        {layout.cells.map((cell) => (
          <div key={`bar:${cell.id}`} style={cell.bar} className="min-w-0">
            <TabBar groupId={cell.id} focused={ui.activeGroupId === cell.id} />
          </div>
        ))}

        <div
          style={cells.get(chatGroupId)?.content}
          className={`relative min-h-0 min-w-0 ${
            // `hidden` (display:none), not `invisible`: Monaco/xterm canvas layers
            // on WebKitGTK keep compositing through visibility:hidden and bleed
            // over the active tab after a git/explorer/coding file open.
            isTabVisible(ui, 'chat') ? '' : 'hidden'
          }`}
          onMouseDown={() => {
            if (ui.activeGroupId !== chatGroupId) focusGroup(chatGroupId)
          }}
        >
          <div className="absolute inset-0 flex items-stretch justify-center">
            {sessionId === null ? <StartScreen /> : <ChatView />}
          </div>
        </div>

        {ui.tabs.map((tab) => {
          if (tab.kind === 'chat') return null
          const groupId = groupOf(ui, tab.id)
          return (
            <div
              key={tab.id}
              style={cells.get(groupId)?.content}
              className={`relative min-h-0 min-w-0 ${
                isTabVisible(ui, tab.id) ? '' : 'hidden'
              }`}
              onMouseDown={() => {
                if (ui.activeGroupId !== groupId) focusGroup(groupId)
              }}
            >
              <div className="absolute inset-0">
                <TabBody tab={tab} />
              </div>
            </div>
          )
        })}

        {/* Measure + paint only. Never pointer-events — document drag listeners
            hit-test these boxes so a stuck session cannot block Monaco typing. */}
        {layout.cells.map((cell) => (
          <div
            key={`drop:${cell.id}`}
            ref={(el) => setPaneEl(cell.id, el)}
            style={cell.content}
            className="pointer-events-none relative z-40 min-h-0 min-w-0"
          >
            <EditorDropHighlight groupId={cell.id} hover={dropHover} />
          </div>
        ))}

        {layout.cells.map((cell, index) =>
          cell.grip ? (
            <div
              key={`grip:${cell.id}`}
              style={cell.grip}
              onMouseDown={(e) => startResize(index, e)}
              className={`z-30 bg-koma-panel2 hover:bg-koma-grip ${
                ui.splitDir === 'row'
                  ? 'cursor-ew-resize border-l border-koma-border'
                  : 'cursor-ns-resize border-t border-koma-border'
              }`}
            />
          ) : null,
        )}
      </div>
      <LspDrawer />
      <ProblemsDrawer />
      <UsageFooter />
    </div>
  )
}

// IndexPage: onboarding takes over the whole view; otherwise always render
// TabbedMain. The welcome/StartScreen content is shown inside TabbedMain's chat
// slot when there's no active session (session.id === null), so the tab bar and
// session-independent tabs (Settings/Help/Agents) stay available on the home screen.
function IndexPage() {
  const needsOnboarding = useNeedsOnboarding()
  if (needsOnboarding) return <Onboarding />
  return <TabbedMain />
}

function SettingsPage() {
  return <div className="p-4">settings (stub)</div>
}

const rootRoute = createRootRoute({ component: RootLayout })

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: IndexPage,
})

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings',
  component: SettingsPage,
})

export const routeTree = rootRoute.addChildren([indexRoute, settingsRoute])
