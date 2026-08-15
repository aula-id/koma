import { createFileRoute, Outlet } from '@tanstack/react-router'
import { useState } from 'react'

import { Sidebar, DocsModeContext } from '../components/Sidebar'
import type { DocsMode } from '../components/Sidebar'

function DocsLayout() {
  const [mode, setMode] = useState<DocsMode>(() => {
    try { return (localStorage.getItem('docs-mode') as DocsMode) || 'tui' } catch { return 'tui' }
  })

  const setModePersist = (m: DocsMode) => {
    setMode(m)
    try { localStorage.setItem('docs-mode', m) } catch { /* ignore */ }
  }

  return (
    <DocsModeContext.Provider value={{ mode, setMode: setModePersist }}>
      <div className="flex h-full">
        <Sidebar />
        <main className="flex-1 overflow-y-auto px-8 py-10">
          <div className="mx-auto max-w-3xl">
            <Outlet />
          </div>
        </main>
      </div>
    </DocsModeContext.Provider>
  )
}

export const Route = createFileRoute('/_docs')({
  component: DocsLayout,
})
