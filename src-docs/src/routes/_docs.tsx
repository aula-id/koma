import { createFileRoute, Outlet } from '@tanstack/react-router'

import { Sidebar } from '../components/Sidebar'

function DocsLayout() {
  return (
    <div className="flex h-full">
      <Sidebar />
      <main className="flex-1 overflow-y-auto px-8 py-10">
        <div className="mx-auto max-w-3xl">
          <Outlet />
        </div>
      </main>
    </div>
  )
}

export const Route = createFileRoute('/_docs')({
  component: DocsLayout,
})
