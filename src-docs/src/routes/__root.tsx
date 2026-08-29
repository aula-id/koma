import { createRootRoute, Outlet } from '@tanstack/react-router'

import { DocsNavProvider } from '../components/DocsNavContext'
import { MobileNavDrawer } from '../components/Sidebar'
import { TopBar } from '../components/TopBar'

function RootLayout() {
  return (
    <DocsNavProvider>
      <div className="flex h-full flex-col">
        <TopBar />
        <div className="relative min-h-0 flex-1">
          <Outlet />
        </div>
      </div>
      <MobileNavDrawer />
    </DocsNavProvider>
  )
}

export const Route = createRootRoute({
  component: RootLayout,
})
