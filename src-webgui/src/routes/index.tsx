import { createRootRoute, createRoute, Outlet } from '@tanstack/react-router'
import { useEffect, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { ChatView } from '../components/ChatView'
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
  const needsOnboarding = useNeedsOnboarding()

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
        <ActivityBar activeView={activeView} sidebarOpen={sidebarOpen} onSelect={selectView} />
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

  if (needsOnboarding) return <Onboarding />
  if (sessionId === null) return <StartScreen />
  return <ChatView />
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
