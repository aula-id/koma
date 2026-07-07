import { createRootRoute, createRoute, Outlet } from '@tanstack/react-router'
import { useState } from 'react'
import { Terminal } from '../components/Terminal'
import { Titlebar, getPlatform } from '../components/Titlebar'
import { ResizeHandles } from '../components/ResizeHandles'

function RootLayout() {
  // Resolved once — window.__komaOS is injected by the Rust host before the
  // app boots and never changes for the lifetime of the window.
  const [platform] = useState(getPlatform)

  return (
    <div id="app" className={`os-${platform}`}>
      <Titlebar />
      <main id="term">
        <Outlet />
      </main>
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
