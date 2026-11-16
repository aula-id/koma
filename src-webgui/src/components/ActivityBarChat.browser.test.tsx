import '../styles.css'
import { page } from 'vitest/browser'
import { expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { useState } from 'react'
import { ActivityBar } from './ActivityBar'
import type { SidebarView } from './Sidebar'
import { resolveViewSelection } from '../lib/sidebarSelect'
import { useKoma } from '../store/koma'

// Mirrors the selectView wiring in routes/index.tsx.
function Harness() {
  const [view, setView] = useState<SidebarView>('git')
  const [open, setOpen] = useState(true)
  const select = (v: SidebarView) => {
    const st = useKoma.getState()
    const r = resolveViewSelection(v, view, open, st.ui.activeTabId)
    if (r.activateChat) st.activateTab('chat')
    setView(r.view)
    setOpen(r.sidebarOpen)
  }
  return <ActivityBar activeView={view} sidebarOpen={open} onSelect={select} onSettings={() => {}} onHelp={() => {}} onTutorial={() => {}} />
}

it('Chat icon focuses the chat tab when Settings is active', async () => {
  useKoma.setState({ req: vi.fn() })
  useKoma.getState().openSettingsTab()
  expect(useKoma.getState().ui.activeTabId).not.toBe('chat')
  await render(<div style={{ height: 600 }}><Harness /></div>)
  await page.getByRole('button', { name: 'Chat' }).click()
  expect(useKoma.getState().ui.activeTabId).toBe('chat')
})
