import { MessageSquare, FileDiff, Settings, CircleHelp, Bot, Terminal, GitGraph, BarChart3, Blocks, Puzzle, Code2, X, ChevronLeft, ChevronRight, Package } from 'lucide-react'
import { useKoma } from '../store/koma'
import { useRef, useEffect, useState, useCallback, type MouseEvent as ReactMouseEvent } from 'react'
import { fileKey } from '../store/coding'
import { DirtyCloseConfirm } from './DirtyCloseConfirm'

// Parent directory of a path — used to disambiguate two open tabs that share a
// basename (VSCode-style dim suffix).
function parentDir(path: string): string {
  const parts = path.split('/').filter(Boolean)
  return parts.length > 1 ? parts[parts.length - 2] : ''
}

// VSCode-style tab strip over the main content column. tabs[0] is the permanent,
// uncloseable chat tab; diff tabs open from the Explorer's File-changed rows.
// Hidden entirely until at least one diff tab exists (zero chrome cost until the
// feature is used). Styling matches the app chrome idiom (ActivityBar): panel2
// strip, active row raised onto the canvas bg with a top accent line in fg.
export function TabBar() {
  const tabs = useKoma((s) => s.ui.tabs)
  const activeTabId = useKoma((s) => s.ui.activeTabId)
  const activateTab = useKoma((s) => s.activateTab)
  const closeTab = useKoma((s) => s.closeTab)
  const codingFiles = useKoma((s) => s.coding.files)
  const codingAutosave = useKoma((s) => !!s.settingsValues?.codingAutosave)
  const saveCodingFile = useKoma((s) => s.saveCodingFile)
  const [dirtyClose, setDirtyClose] = useState<{
    id: string
    title: string
  } | null>(null)
  const [awaitingAutosaveClose, setAwaitingAutosaveClose] = useState<{
    id: string
    title: string
  } | null>(null)

  const containerRef = useRef<HTMLDivElement>(null)
  const tabRefs = useRef<Map<string, HTMLDivElement | HTMLButtonElement | null>>(new Map())
  const [canScrollLeft, setCanScrollLeft] = useState(false)
  const [canScrollRight, setCanScrollRight] = useState(false)

  const requestClose = useCallback(
    (id: string, title: string, e: ReactMouseEvent) => {
      const tab = tabs.find((t) => t.id === id)
      if (tab?.kind === 'codingFile') {
        const fs = codingFiles[fileKey(tab.root, tab.path)]
        if (fs?.dirty) {
          e.stopPropagation()
          if (codingAutosave && !fs.conflict && !fs.error && !fs.binary && !fs.tooLarge) {
            // Trigger a save and defer close until it completes.
            setAwaitingAutosaveClose({ id, title })
            if (!fs.saving) saveCodingFile(tab.root, tab.path)
          } else {
            setDirtyClose({ id, title })
          }
          return
        }
      }
      e.stopPropagation()
      closeTab(id)
    },
    [tabs, codingFiles, codingAutosave, closeTab, saveCodingFile],
  )

  // When awaiting autosave-close, watch the file state:
  // - dirty cleared → close the tab.
  // - error/conflict appeared → show the discard popover instead.
  useEffect(() => {
    if (!awaitingAutosaveClose) return
    const tab = tabs.find((t) => t.id === awaitingAutosaveClose.id)
    if (!tab || tab.kind !== 'codingFile') {
      setAwaitingAutosaveClose(null)
      return
    }
    const fs = codingFiles[fileKey(tab.root, tab.path)]
    if (!fs) { setAwaitingAutosaveClose(null); return }
    if (fs.error || fs.conflict) {
      // Save failed — fall back to the discard confirmation.
      setAwaitingAutosaveClose(null)
      setDirtyClose({ id: awaitingAutosaveClose.id, title: awaitingAutosaveClose.title })
      return
    }
    if (!fs.dirty && !fs.saving) {
      // Save succeeded — close the tab.
      const id = awaitingAutosaveClose.id
      setAwaitingAutosaveClose(null)
      closeTab(id, { force: true })
    }
  }, [awaitingAutosaveClose, tabs, codingFiles, closeTab])

  // Check overflow state on mount and whenever tabs change size/content
  const checkOverflow = useCallback(() => {
    const el = containerRef.current
    if (!el) return
    setCanScrollLeft(el.scrollLeft > 0)
    setCanScrollRight(el.scrollLeft + el.clientWidth < el.scrollWidth)
  }, [])

  // Scroll the tab strip to reveal the active tab (called after activation)
  const revealActiveTab = useCallback(() => {
    const container = containerRef.current
    const activeTab = tabRefs.current.get(activeTabId)
    if (!container || !activeTab) return

    const containerRect = container.getBoundingClientRect()
    const tabRect = activeTab.getBoundingClientRect()

    // If tab is before the visible area, scroll left to show it
    if (tabRect.left < containerRect.left) {
      container.scrollLeft += tabRect.left - containerRect.left
    }
    // If tab is after the visible area, scroll right to show it
    else if (tabRect.right > containerRect.right) {
      container.scrollLeft += tabRect.right - containerRect.right
    }
    // Re-check overflow after revealing
    checkOverflow()
  }, [activeTabId, checkOverflow])

  // Attach scroll/resize listeners and clean up
  useEffect(() => {
    checkOverflow()
    const el = containerRef.current
    if (!el) return

    const handleScroll = () => {
      setCanScrollLeft(el.scrollLeft > 0)
      setCanScrollRight(el.scrollLeft + el.clientWidth < el.scrollWidth)
    }

    el.addEventListener('scroll', handleScroll)
    window.addEventListener('resize', checkOverflow)
    const resizeObserver = new ResizeObserver(checkOverflow)
    resizeObserver.observe(el)

    // Reveal active tab after initial render + after resize
    revealActiveTab()

    return () => {
      el.removeEventListener('scroll', handleScroll)
      window.removeEventListener('resize', checkOverflow)
      resizeObserver.disconnect()
    }
  }, [checkOverflow, revealActiveTab, tabs.length])

  // Re-reveal when active tab changes
  useEffect(() => {
    revealActiveTab()
  }, [activeTabId, revealActiveTab])

  // Scroll by one viewport width (smooth)
  const scrollByViewport = (direction: 'left' | 'right') => {
    const el = containerRef.current
    if (!el) return
    const delta = direction === 'left' ? -el.clientWidth : el.clientWidth
    el.scrollBy({ left: delta, behavior: 'smooth' })
  }

  if (tabs.length <= 1) return null

  // Count basenames so a colliding title can show its parent dir.
  const counts = new Map<string, number>()
  for (const t of tabs) {
    if (t.kind === 'diff') counts.set(t.title, (counts.get(t.title) ?? 0) + 1)
  }

  return (
    <div className="flex h-8 flex-none items-stretch border-b border-koma-border bg-koma-panel2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
      {canScrollLeft && (
        <button
          onClick={() => scrollByViewport('left')}
          aria-label="Scroll tabs left"
          className="flex w-6 flex-none items-center justify-center text-koma-fg opacity-60 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          <ChevronLeft size={14} />
        </button>
      )}
      <div
        ref={containerRef}
        className="flex h-full min-w-0 flex-1 items-stretch overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {tabs.map((t) => {
          const active = t.id === activeTabId
          const base =
            'group relative flex h-full flex-none select-none items-center gap-1.5 border-r border-koma-border text-[12px] transition-colors'
          const tone = active
            ? 'bg-koma-bg text-koma-fg'
            : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
          // Active indicator — a top accent line in fg, matching the ActivityBar's
          // active-view bar.
          const accent = active ? (
            <span className="absolute inset-x-0 top-0 h-0.5 bg-koma-fg" />
          ) : null

          if (t.kind === 'chat') {
            return (
              <button
                key={t.id}
                ref={(el) => {
                  tabRefs.current.set(t.id, el)
                }}
                onClick={() => activateTab(t.id)}
                title="Chat"
                className={`${base} ${tone} px-3`}
              >
                {accent}
                <MessageSquare size={13} className="flex-none" />
                <span>chat</span>
              </button>
            )
          }

          // Settings tab: closeable like a diff tab, with the gear icon + fixed title.
          if (t.kind === 'settings') {
            return (
              <div
                key={t.id}
                ref={(el) => {
                  tabRefs.current.set(t.id, el)
                }}
                role="button"
                tabIndex={0}
                onClick={() => activateTab(t.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
                }}
                title="Settings"
                className={`${base} ${tone} cursor-pointer pl-3 pr-1.5`}
              >
                {accent}
                <Settings size={13} className="flex-none opacity-80" />
                <span className="truncate">Settings</span>
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    closeTab(t.id)
                  }}
                  aria-label="Close tab"
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                    active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                  }`}
                >
                  <X size={12} />
                </button>
              </div>
            )
          }

          // Help tab: closeable like a diff tab, with a help icon + fixed title.
          // Mirrors the Settings tab block exactly.
          if (t.kind === 'help') {
            return (
              <div
                key={t.id}
                ref={(el) => {
                  tabRefs.current.set(t.id, el)
                }}
                role="button"
                tabIndex={0}
                onClick={() => activateTab(t.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
                }}
                title="Help"
                className={`${base} ${tone} cursor-pointer pl-3 pr-1.5`}
              >
                {accent}
                <CircleHelp size={13} className="flex-none opacity-80" />
                <span className="truncate">Help</span>
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    closeTab(t.id)
                  }}
                  aria-label="Close tab"
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                    active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                  }`}
                >
                  <X size={12} />
                </button>
              </div>
            )
          }

          // Commit-graph tab: closeable like a diff tab, GitGraph icon + fixed
          // title. Mirrors the Settings/Help tab blocks exactly.
          if (t.kind === 'graph') {
            return (
              <div
                key={t.id}
                ref={(el) => {
                  tabRefs.current.set(t.id, el)
                }}
                role="button"
                tabIndex={0}
                onClick={() => activateTab(t.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
                }}
                title="Commit Graph"
                className={`${base} ${tone} cursor-pointer pl-3 pr-1.5`}
              >
                {accent}
                <GitGraph size={13} className="flex-none opacity-80" />
                <span className="truncate">Graph</span>
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    closeTab(t.id)
                  }}
                  aria-label="Close tab"
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                    active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                  }`}
                >
                  <X size={12} />
                </button>
              </div>
            )
          }

          // Analytics tab: closeable singleton, BarChart3 icon + fixed title.
          if (t.kind === 'analytics') {
            return (
              <div
                key={t.id}
                ref={(el) => {
                  tabRefs.current.set(t.id, el)
                }}
                role="button"
                tabIndex={0}
                onClick={() => activateTab(t.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
                }}
                title="Analytics"
                className={`${base} ${tone} cursor-pointer pl-3 pr-1.5`}
              >
                {accent}
                <BarChart3 size={13} className="flex-none opacity-80" />
                <span className="truncate">Analytics</span>
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    closeTab(t.id)
                  }}
                  aria-label="Close tab"
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                    active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                  }`}
                >
                  <X size={12} />
                </button>
              </div>
            )
          }

          // Extension STORE tab: closeable like a diff tab, Store icon + fixed
          // title. Mirrors the Settings/Help/Graph tab blocks exactly.
          if (t.kind === 'store') {
            return (
              <div
                key={t.id}
                ref={(el) => {
                  tabRefs.current.set(t.id, el)
                }}
                role="button"
                tabIndex={0}
                onClick={() => activateTab(t.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
                }}
                title="Extensions"
                className={`${base} ${tone} cursor-pointer pl-3 pr-1.5`}
              >
                {accent}
                <Blocks size={13} className="flex-none opacity-80" />
                <span className="truncate">Extensions</span>
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    closeTab(t.id)
                  }}
                  aria-label="Close tab"
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                    active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                  }`}
                >
                  <X size={12} />
                </button>
              </div>
            )
          }

          // Installed-extension detail tab (Tab-B): closeable, Package icon,
          // manifest title.
          if (t.kind === 'installedExtension') {
            const label = t.title || t.extId.split('.').pop() || t.extId
            return (
              <div
                key={t.id}
                ref={(el) => {
                  tabRefs.current.set(t.id, el)
                }}
                role="button"
                tabIndex={0}
                onClick={() => activateTab(t.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
                }}
                title={t.extId}
                className={`${base} ${tone} cursor-pointer pl-3 pr-1.5`}
              >
                {accent}
                <Package size={13} className="flex-none opacity-80" />
                <span className="truncate">{label}</span>
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    closeTab(t.id)
                  }}
                  aria-label="Close tab"
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                    active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                  }`}
                >
                  <X size={12} />
                </button>
              </div>
            )
          }

          // Extension PANEL tab: closeable like a diff tab, Puzzle icon + the
          // panel's title. Mirrors the Settings/Help/Graph/Store tab blocks
          // exactly — content is the `<iframe>` TabbedMain renders for `t.kind
          // === 'extension'`, not anything drawn here.
          if (t.kind === 'extension') {
            return (
              <div
                key={t.id}
                ref={(el) => {
                  tabRefs.current.set(t.id, el)
                }}
                role="button"
                tabIndex={0}
                onClick={() => activateTab(t.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
                }}
                title={t.title}
                className={`${base} ${tone} max-w-[220px] cursor-pointer pl-3 pr-1.5`}
              >
                {accent}
                <Puzzle size={13} className="flex-none opacity-80" />
                <span className="truncate">{t.title}</span>
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    closeTab(t.id)
                  }}
                  aria-label="Close tab"
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                    active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                  }`}
                >
                  <X size={12} />
                </button>
              </div>
            )
          }

          // Coding panel file editor tab: Code2 icon + title, dirty letter badge.
          if (t.kind === 'codingFile') {
            const fs = codingFiles[fileKey(t.root, t.path)]
            const dirty = !!fs?.dirty
            const isNew = dirty && fs?.savedContent === null
            return (
              <div
                key={t.id}
                ref={(el) => {
                  tabRefs.current.set(t.id, el)
                }}
                role="button"
                tabIndex={0}
                onClick={() => activateTab(t.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
                }}
                title={t.path}
                className={`${base} ${tone} max-w-[220px] cursor-pointer pl-3 pr-1.5`}
              >
                {accent}
                <Code2 size={13} className="flex-none opacity-80" />
                <span className="truncate">
                  {dirty ? (
                    <span className={`mr-0.5 font-mono text-[10px] font-semibold ${isNew ? 'text-koma-success' : 'text-koma-accent'}`}>
                      {isNew ? 'A' : 'M'}
                    </span>
                  ) : null}
                  {t.title}
                </span>
                <button
                  onClick={(e) => requestClose(t.id, t.title, e)}
                  aria-label="Close tab"
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                    active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                  }`}
                >
                  <X size={12} />
                </button>
              </div>
            )
          }

          // Agent editor tab: closeable like a diff tab, with a Bot icon + the
          // agent's current name (or "new agent" while agentId is still null,
          // i.e. an unsaved create). Label tracks `agentId` live, so a rename
          // mid-edit (before OR after Save rebinds it — see renameAgentTab)
          // always shows the current name, never a stale one.
          if (t.kind === 'agent') {
            return (
              <div
                key={t.id}
                ref={(el) => {
                  tabRefs.current.set(t.id, el)
                }}
                role="button"
                tabIndex={0}
                onClick={() => activateTab(t.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
                }}
                title={t.agentId ?? 'new agent'}
                className={`${base} ${tone} max-w-[220px] cursor-pointer pl-3 pr-1.5`}
              >
                {accent}
                <Bot size={13} className="flex-none opacity-80" />
                <span className="truncate">{t.agentId ?? 'new agent'}</span>
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    closeTab(t.id)
                  }}
                  aria-label="Close tab"
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                    active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                  }`}
                >
                  <X size={12} />
                </button>
              </div>
            )
          }

          // Stream tabs (read-only sub-agent transcript / bash output): closeable like a
          // diff tab, with a Bot / Terminal icon + the title (agent name / truncated cmd).
          if (t.kind === 'subagent' || t.kind === 'bash') {
            const Icon = t.kind === 'subagent' ? Bot : Terminal
            return (
              <div
                key={t.id}
                ref={(el) => {
                  tabRefs.current.set(t.id, el)
                }}
                role="button"
                tabIndex={0}
                onClick={() => activateTab(t.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
                }}
                title={t.title}
                className={`${base} ${tone} max-w-[220px] cursor-pointer pl-3 pr-1.5`}
              >
                {accent}
                <Icon size={13} className="flex-none opacity-80" />
                <span className="truncate">{t.title}</span>
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    closeTab(t.id)
                  }}
                  aria-label="Close tab"
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                    active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                  }`}
                >
                  <X size={12} />
                </button>
              </div>
            )
          }

          const dir = (counts.get(t.title) ?? 0) > 1 ? parentDir(t.path) : ''
          // A div (not a button) so the close × can nest without invalid
          // button-in-button markup; keyboard-activatable via role/tabIndex.
          return (
            <div
              key={t.id}
              ref={(el) => {
                tabRefs.current.set(t.id, el)
              }}
              role="button"
              tabIndex={0}
              onClick={() => activateTab(t.id)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') activateTab(t.id)
              }}
              title={t.path}
              className={`${base} ${tone} max-w-[220px] cursor-pointer pl-3 pr-1.5`}
            >
              {accent}
              <FileDiff size={13} className="flex-none opacity-80" />
              <span className="truncate">{t.title}</span>
              {dir && <span className="flex-none truncate text-koma-dim opacity-60">{dir}</span>}
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  closeTab(t.id)
                }}
                aria-label="Close tab"
                title="Close"
                className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                  active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                }`}
              >
                <X size={12} />
              </button>
            </div>
          )
        })}
      </div>
      {canScrollRight && (
        <button
          onClick={() => scrollByViewport('right')}
          aria-label="Scroll tabs right"
          className="flex w-6 flex-none items-center justify-center text-koma-fg opacity-60 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          <ChevronRight size={14} />
        </button>
      )}
      {dirtyClose && (
        <DirtyCloseConfirm
          anchor={tabRefs.current.get(dirtyClose.id) ?? null}
          title={dirtyClose.title}
          onConfirm={() => {
            const id = dirtyClose.id
            setDirtyClose(null)
            closeTab(id, { force: true })
          }}
          onCancel={() => setDirtyClose(null)}
        />
      )}
    </div>
  )
}
