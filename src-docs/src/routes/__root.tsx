import { createRootRoute, Outlet } from '@tanstack/react-router'

import { TopBar } from '../components/TopBar'

function RootLayout() {
  return (
    <div className="flex h-full flex-col">
      <TopBar />
      <div className="min-h-0 flex-1">
        <Outlet />
      </div>
    </div>
  )
}

export const Route = createRootRoute({
  component: RootLayout,
})
