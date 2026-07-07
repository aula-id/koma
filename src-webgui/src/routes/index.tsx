import { createRootRoute, createRoute, Outlet } from '@tanstack/react-router'
import { useState, type MouseEvent as ReactMouseEvent } from 'react'
import { Terminal } from '../components/Terminal'
import { Titlebar, getPlatform } from '../components/Titlebar'
import { ResizeHandles } from '../components/ResizeHandles'
import { ActivityBar } from '../components/ActivityBar'
import { Sidebar } from '../components/Sidebar'

const SIDEBAR_MIN = 150
const SIDEBAR_MAX = 500

function RootLayout() {
  // Resolved once — window.__komaOS is injected by the Rust host before the
  // app boots and never changes for the lifetime of the window.
  const [platform] = useState(getPlatform)
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [sidebarWidth, setSidebarWidth] = useState(240)

  // Drag the divider between the sidebar and the terminal to resize. Tracks
  // the mouse on window (so the drag survives leaving the 5px handle) and
  // clamps the width. The terminal's own ResizeObserver refits xterm as the
  // remaining space changes.
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

  return (
    <div id="app" className={`os-${platform}`}>
      <Titlebar />
      <div className="absolute inset-x-0 top-8 bottom-0 flex overflow-hidden">
        <ActivityBar sidebarOpen={sidebarOpen} onToggle={() => setSidebarOpen((o) => !o)} />
        {sidebarOpen && (
          <>
            <Sidebar width={sidebarWidth} />
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
      <ResizeHandles />
    </div>
  )
}

function IndexPage() {
  return <Terminal />
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
