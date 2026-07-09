import { createRootRoute, createRoute, Outlet } from '@tanstack/react-router'
import { lazy, Suspense, useEffect, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { Loader2 } from 'lucide-react'
import { ChatView } from '../components/ChatView'
import { TabBar } from '../components/TabBar'
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
import { ToastContainer } from '../components/ToastContainer'
import { UsageFooter } from '../components/UsageFooter'
import { useKoma } from '../store/koma'

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
  const configured = providers.length > 0 && models.some((m) => m.roles.includes('main'))
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
  const req = useKoma((s) => s.req)
  const openSettingsTab = useKoma((s) => s.openSettingsTab)
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

  // Wire the JS <-> Rust bridge: expose window.__komaClient.push so the host
  // can feed the koma store, then announce readiness so it sends the first
  // push (Hub if swapper else Snapshot).
  useEffect(() => {
    window.__komaClient = {
      push: (j) => useKoma.getState().push(JSON.parse(j)),
    }
    useKoma.getState().req({ r: 'Ready' })
    return () => {
      window.__komaClient = undefined
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
        <Titlebar onSearch={() => {}} onRename={() => {}} overlayOpen />
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
        overlayOpen={overlay !== 'none'}
      />
      <div className="absolute inset-x-0 top-8 bottom-0 flex overflow-hidden">
        <ActivityBar
          activeView={activeView}
          sidebarOpen={sidebarOpen}
          onSelect={selectView}
          onSettings={openSettingsTab}
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
      <ToastContainer />
      <ResizeHandles />
    </div>
  )
}

// Monaco DiffEditor is HEAVY — lazy so its chunk never loads until the first
// diff tab is opened (a tiny spinner covers the one-time chunk fetch).
const DiffTab = lazy(() => import('../components/DiffTab'))

// Settings page — lazy so its chunk only loads when the gear is first clicked.
const SettingsTab = lazy(() => import('../components/SettingsTab'))

// Read-only stream tab (sub-agent transcript / bash output) — lazy so its chunk only
// loads when the first stream tab is opened from the Explorer.
const StreamTab = lazy(() => import('../components/StreamTab'))

function DiffFallback() {
  return (
    <div className="flex h-full w-full items-center justify-center text-koma-dim">
      <Loader2 size={18} className="animate-spin opacity-70" />
    </div>
  )
}

// Tabbed main column: a VSCode-style TabBar over stacked tab contents, with the
// usage/statusline footer pinned along the bottom — full width, spanning from
// the sidebar edge to the window edge, visible across every tab (chat + diff).
// The chat stays MOUNTED at all times (hidden, not unmounted, when a diff tab
// is active) so its scroll/stream/state survive tab switches; diff tabs mount
// when opened and stay mounted while open for fast switching, unmounting only
// on close. The TabBar spans the full main column; the chat keeps its centered
// reading column, while diff editors use the full width.
function TabbedMain() {
  const tabs = useKoma((s) => s.ui.tabs)
  const activeTabId = useKoma((s) => s.ui.activeTabId)
  const chatActive = activeTabId === 'chat'
  return (
    <div className="flex h-full w-full min-w-0 flex-col">
      <TabBar />
      <div className="relative min-h-0 flex-1">
        <div className={`absolute inset-0 flex items-stretch justify-center ${chatActive ? '' : 'hidden'}`}>
          <ChatView />
        </div>
        {tabs.map((t) =>
          t.kind === 'diff' ? (
            <div key={t.id} className={`absolute inset-0 ${activeTabId === t.id ? '' : 'hidden'}`}>
              <Suspense fallback={<DiffFallback />}>
                <DiffTab tab={t} />
              </Suspense>
            </div>
          ) : t.kind === 'settings' ? (
            <div key={t.id} className={`absolute inset-0 ${activeTabId === t.id ? '' : 'hidden'}`}>
              <Suspense fallback={<DiffFallback />}>
                <SettingsTab />
              </Suspense>
            </div>
          ) : t.kind === 'subagent' || t.kind === 'bash' ? (
            <div key={t.id} className={`absolute inset-0 ${activeTabId === t.id ? '' : 'hidden'}`}>
              <Suspense fallback={<DiffFallback />}>
                <StreamTab tab={t} />
              </Suspense>
            </div>
          ) : null,
        )}
      </div>
      <UsageFooter />
    </div>
  )
}

// Three-way gate: ONBOARDING (first-run) > START SCREEN (no session) > CHAT
// (attached). The swapper/empty state pushes only Hub + Config (never a
// Snapshot), so `session.id === null` means no attached session; `config` is
// authoritative for the first-run decision.
//   - Onboarding: the host's first-run flag when present, else inferred from an
//     unconfigured config (no provider, or no Main-role model). Gated on
//     `loaded` so it never flashes against the empty initial slice before the
//     first Config push.
//   - Start screen: no session attached but config is usable.
//   - Chat: a live session id.
function IndexPage() {
  const sessionId = useKoma((s) => s.session.id)
  const needsOnboarding = useNeedsOnboarding()
  // Settings opens as a tab that works even with NO attached session (the host
  // answers GetSettings from global config while detached), so honour it over the
  // StartScreen gate. Every other no-session case still shows the StartScreen;
  // closing Settings reverts activeTabId to 'chat', dropping back to StartScreen.
  const settingsActive = useKoma((s) => s.ui.activeTabId === 'settings')

  if (needsOnboarding) return <Onboarding />
  if (sessionId === null && !settingsActive) return <StartScreen />
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
