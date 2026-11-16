import type { SidebarView } from '../components/Sidebar'

export type ViewSelection = { activateChat: boolean; view: SidebarView; sidebarOpen: boolean }

// Clicking an activity icon. Chat ('explore') always focuses the chat tab and
// collapses the sidebar only when it is already the active view, open, and the
// chat tab was already focused. Every other view keeps the plain toggle/switch.
export function resolveViewSelection(
  clicked: SidebarView,
  activeView: SidebarView,
  sidebarOpen: boolean,
  activeTabId: string,
): ViewSelection {
  const isChat = clicked === 'explore'
  const collapse = clicked === activeView && sidebarOpen && (!isChat || activeTabId === 'chat')
  return { activateChat: isChat, view: clicked, sidebarOpen: !collapse }
}
