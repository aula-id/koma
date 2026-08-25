import {
  BarChart3,
  Blocks,
  Bot,
  ChevronLeft,
  ChevronRight,
  CircleHelp,
  Code2,
  Columns2,
  FileDiff,
  GitGraph,
  GraduationCap,
  MessageSquare,
  Network,
  Package,
  PanelBottom,
  PanelRight,
  Puzzle,
  Rows2,
  Settings,
  SquareTerminal,
  Terminal,
  X,
  type LucideIcon,
} from 'lucide-react'
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from 'react'
import { createPortal } from 'react-dom'
import { useShallow } from 'zustand/react/shallow'
import { hasCodingPathDrag, readCodingPathDragData } from '../lib/codingRef'
import { fileKey } from '../store/coding'
import {
  MAX_GROUPS,
  groupOf,
  normalizeGroups,
  type EditorGroupId,
} from '../store/editorGroups'
import { useKoma, type Tab } from '../store/koma'
import { DirtyCloseConfirm } from './DirtyCloseConfirm'

/** Native drag payload shared with the pane drop targets in routes/index.tsx. */
export const TAB_DRAG_MIME = 'application/x-koma-tab'

export function draggedTabId(e: Pick<DragEvent, 'dataTransfer'>): string | null {
  return e.dataTransfer?.getData(TAB_DRAG_MIME) || null
}

function parentDir(path: string): string {
  const parts = path.split('/').filter(Boolean)
  return parts.length > 1 ? parts[parts.length - 2] : ''
}

type CodingDirtyFlags = {
  dirty: boolean
  conflict: boolean
  error: boolean
  binary: boolean
  tooLarge: boolean
  saving: boolean
  savedContentNull: boolean
}

type TabVisual = {
  Icon: LucideIcon
  label: ReactNode
  title: string
  suffix?: string
}

function tabVisual(
  tab: Tab,
  counts: Map<string, number>,
  dirty: CodingDirtyFlags | undefined,
): TabVisual {
  switch (tab.kind) {
    case 'chat':
      return { Icon: MessageSquare, label: 'chat', title: 'Chat' }
    case 'settings':
      return { Icon: Settings, label: 'Settings', title: 'Settings' }
    case 'help':
      return { Icon: CircleHelp, label: 'Help', title: 'Help' }
    case 'tutorial':
      return { Icon: GraduationCap, label: 'Tutorial', title: 'Tutorial' }
    case 'graph':
      return { Icon: GitGraph, label: 'Graph', title: 'Commit Graph' }
    case 'importGraph':
      return { Icon: Network, label: 'Import Graph', title: 'Import Graph' }
    case 'analytics':
      return { Icon: BarChart3, label: 'Analytics', title: 'Analytics' }
    case 'store':
      return { Icon: Blocks, label: 'Extensions', title: 'Extensions' }
    case 'installedExtension':
      return {
        Icon: Package,
        label: tab.title || tab.extId.split('.').pop() || tab.extId,
        title: tab.extId,
      }
    case 'extension':
      return { Icon: Puzzle, label: tab.title, title: tab.title }
    case 'codingFile': {
      const isNew = !!dirty?.dirty && !!dirty.savedContentNull
      return {
        Icon: Code2,
        title: tab.path,
        label: (
          <>
            {dirty?.dirty ? (
              <span
                className={`mr-0.5 font-mono text-[10px] font-semibold ${
                  isNew ? 'text-koma-success' : 'text-koma-accent'
                }`}
              >
                {isNew ? 'A' : 'M'}
              </span>
            ) : null}
            {tab.title}
          </>
        ),
      }
    }
    case 'agent':
      return { Icon: Bot, label: tab.agentId ?? 'new agent', title: tab.agentId ?? 'new agent' }
    case 'subagent':
      return { Icon: Bot, label: tab.title, title: tab.title }
    case 'bash':
      return { Icon: Terminal, label: tab.title, title: tab.title }
    case 'terminal':
      return { Icon: SquareTerminal, label: tab.title, title: tab.title }
    case 'diff':
      return {
        Icon: FileDiff,
        label: tab.title,
        title: tab.path,
        suffix: (counts.get(tab.title) ?? 0) > 1 ? parentDir(tab.path) : '',
      }
  }
}

type MenuState = { x: number; y: number; tabId: string }

function TabContextMenu({
  state,
  groupId,
  canSplit,
  canToggle,
  onRequestClose,
  onClose,
}: {
  state: MenuState
  groupId: EditorGroupId
  canSplit: boolean
  canToggle: boolean
  onRequestClose: (tabId: string) => void
  onClose: () => void
}) {
  const splitTab = useKoma((s) => s.splitTab)
  const toggleSplitDir = useKoma((s) => s.toggleSplitDir)
  const splitDir = useKoma((s) => s.ui.splitDir)
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: state.x, top: state.y })

  useEffect(() => {
    const el = ref.current
    if (el) {
      setPos({
        left: Math.max(4, Math.min(state.x, window.innerWidth - el.offsetWidth - 4)),
        top: Math.max(4, Math.min(state.y, window.innerHeight - el.offsetHeight - 4)),
      })
    }
    const outside = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose()
    }
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('mousedown', outside, true)
    window.addEventListener('keydown', key)
    return () => {
      window.removeEventListener('mousedown', outside, true)
      window.removeEventListener('keydown', key)
    }
  }, [onClose, state.x, state.y])

  const item =
    'flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-35'

  return createPortal(
    <div
      ref={ref}
      style={{ position: 'fixed', ...pos, width: 210, zIndex: 95 }}
      className="overflow-hidden rounded-md border border-koma-border bg-koma-panel py-1 shadow-sm"
      onContextMenu={(e) => e.preventDefault()}
    >
      {canSplit ? (
        <>
          <button
            type="button"
            className={item}
            onClick={() => {
              splitTab(state.tabId, groupId, 'after', 'row')
              onClose()
            }}
          >
            <PanelRight size={13} />
            Split Right
          </button>
          <button
            type="button"
            className={item}
            onClick={() => {
              splitTab(state.tabId, groupId, 'after', 'col')
              onClose()
            }}
          >
            <PanelBottom size={13} />
            Split Down
          </button>
        </>
      ) : canToggle ? (
        <button
          type="button"
          className={item}
          onClick={() => {
            toggleSplitDir()
            onClose()
          }}
        >
          {splitDir === 'row' ? <Rows2 size={13} /> : <Columns2 size={13} />}
          {splitDir === 'row' ? 'Stack Vertically' : 'Split Horizontally'}
        </button>
      ) : null}
      {state.tabId !== 'chat' && (
        <>
          {(canSplit || canToggle) && <div className="my-1 border-t border-koma-border" />}
          <button
            type="button"
            className={item}
            onClick={() => {
              onRequestClose(state.tabId)
              onClose()
            }}
          >
            <X size={13} />
            Close
          </button>
        </>
      )}
    </div>,
    document.body,
  )
}

type Props = {
  groupId: EditorGroupId
  focused: boolean
}

// One VSCode-style tab strip PER editor group. Native drag/drop moves tabs
// between strips (or reorders inside one); right-click and the split button
// create adjacent panes. All tab kinds share this renderer so the interaction
// grammar cannot drift between file, diff, terminal, settings, and extension tabs.
export function TabBar({ groupId, focused }: Props) {
  const rawUi = useKoma((s) => s.ui)
  const ui = useMemo(() => normalizeGroups(rawUi), [rawUi])
  const tabs = useMemo(
    () => ui.tabs.filter((t) => groupOf(ui, t.id) === groupId),
    [groupId, ui],
  )
  const activeTabId = ui.groupActive[groupId]
  const activateTab = useKoma((s) => s.activateTab)
  const closeTab = useKoma((s) => s.closeTab)
  const moveTabToGroup = useKoma((s) => s.moveTabToGroup)
  const splitTab = useKoma((s) => s.splitTab)
  const toggleSplitDir = useKoma((s) => s.toggleSplitDir)
  const openCodingFile = useKoma((s) => s.openCodingFile)
  const focusGroup = useKoma((s) => s.focusEditorGroup)
  const codingDirty = useKoma(
    useShallow((s) => {
      const out: Record<string, CodingDirtyFlags> = {}
      for (const [k, f] of Object.entries(s.coding.files)) {
        if (!f || !(f.dirty || f.conflict || f.saving)) continue
        out[k] = {
          dirty: !!f.dirty,
          conflict: !!f.conflict,
          error: !!f.error,
          binary: !!f.binary,
          tooLarge: !!f.tooLarge,
          saving: !!f.saving,
          savedContentNull: f.savedContent === null,
        }
      }
      return out
    }),
  )
  const codingAutosave = useKoma((s) => !!s.settingsValues?.codingAutosave)
  const saveCodingFile = useKoma((s) => s.saveCodingFile)
  const [menu, setMenu] = useState<MenuState | null>(null)
  const [dirtyClose, setDirtyClose] = useState<{ id: string; title: string } | null>(null)
  const [awaitingAutosaveClose, setAwaitingAutosaveClose] = useState<{
    id: string
    title: string
  } | null>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const tabRefs = useRef<Map<string, HTMLDivElement | HTMLButtonElement | null>>(new Map())
  const [canScrollLeft, setCanScrollLeft] = useState(false)
  const [canScrollRight, setCanScrollRight] = useState(false)

  const requestClose = useCallback(
    (tab: Tab, e?: ReactMouseEvent) => {
      if (tab.kind === 'codingFile') {
        const fs = codingDirty[fileKey(tab.root, tab.path)]
        if (fs?.dirty) {
          e?.stopPropagation()
          if (codingAutosave && !fs.conflict && !fs.error && !fs.binary && !fs.tooLarge) {
            setAwaitingAutosaveClose({ id: tab.id, title: tab.title })
            if (!fs.saving) saveCodingFile(tab.root, tab.path)
          } else {
            setDirtyClose({ id: tab.id, title: tab.title })
          }
          return
        }
      }
      e?.stopPropagation()
      closeTab(tab.id)
    },
    [closeTab, codingAutosave, codingDirty, saveCodingFile],
  )

  useEffect(() => {
    if (!awaitingAutosaveClose) return
    const tab = rawUi.tabs.find((t) => t.id === awaitingAutosaveClose.id)
    if (!tab || tab.kind !== 'codingFile') {
      setAwaitingAutosaveClose(null)
      return
    }
    const fs = codingDirty[fileKey(tab.root, tab.path)]
    if (!fs) {
      setAwaitingAutosaveClose(null)
      closeTab(tab.id, { force: true })
    } else if (fs.error || fs.conflict) {
      setAwaitingAutosaveClose(null)
      setDirtyClose({ id: tab.id, title: tab.title })
    } else if (!fs.dirty && !fs.saving) {
      setAwaitingAutosaveClose(null)
      closeTab(tab.id, { force: true })
    }
  }, [awaitingAutosaveClose, closeTab, codingDirty, rawUi.tabs])

  const checkOverflow = useCallback(() => {
    const el = containerRef.current
    if (!el) return
    // Epsilon + functional setState: chevron mount/unmount used to change the
    // strip width by 24px and re-fire ResizeObserver into a setState storm
    // (React #185) near the overflow boundary.
    const nextLeft = el.scrollLeft > 1
    const nextRight = el.scrollWidth - el.clientWidth - el.scrollLeft > 1
    setCanScrollLeft((v) => (v === nextLeft ? v : nextLeft))
    setCanScrollRight((v) => (v === nextRight ? v : nextRight))
  }, [])

  // Size/scroll only — never scrollIntoView here (that mutates scrollLeft and
  // can flip overflow flags every RO callback).
  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const observer = new ResizeObserver(() => {
      requestAnimationFrame(checkOverflow)
    })
    observer.observe(el)
    el.addEventListener('scroll', checkOverflow, { passive: true })
    checkOverflow()
    return () => {
      observer.disconnect()
      el.removeEventListener('scroll', checkOverflow)
    }
  }, [checkOverflow, tabs.length])

  // Reveal the active tab once when selection or strip membership changes.
  useEffect(() => {
    const active = tabRefs.current.get(activeTabId)
    if (!active) return
    const raf = requestAnimationFrame(() => {
      active.scrollIntoView({ block: 'nearest', inline: 'nearest' })
      checkOverflow()
    })
    return () => cancelAnimationFrame(raf)
  }, [activeTabId, checkOverflow, tabs.length])

  if (ui.tabs.length <= 1 && ui.groups.length <= 1) return null

  const counts = new Map<string, number>()
  for (const t of ui.tabs) {
    if (t.kind === 'diff') counts.set(t.title, (counts.get(t.title) ?? 0) + 1)
  }

  const scroll = (dir: -1 | 1) => {
    const el = containerRef.current
    el?.scrollBy({ left: dir * el.clientWidth, behavior: 'smooth' })
  }

  const startDrag = (e: ReactDragEvent, tabId: string) => {
    e.dataTransfer.effectAllowed = 'move'
    e.dataTransfer.setData(TAB_DRAG_MIME, tabId)
    e.dataTransfer.setData('text/plain', tabId)
  }

  const acceptStripDrag = (e: ReactDragEvent) => {
    if (e.dataTransfer.types.includes(TAB_DRAG_MIME)) {
      e.preventDefault()
      e.dataTransfer.dropEffect = 'move'
      return
    }
    if (hasCodingPathDrag(e.dataTransfer)) {
      e.preventDefault()
      e.dataTransfer.dropEffect = 'copy'
    }
  }

  const dropBefore = (e: ReactDragEvent, beforeId: string | null) => {
    const coding = readCodingPathDragData(e.dataTransfer)
    if (coding) {
      e.preventDefault()
      e.stopPropagation()
      if (!coding.isDir) {
        openCodingFile(coding.root, coding.path, { groupId, beforeId })
      }
      return
    }
    const tabId = e.dataTransfer.getData(TAB_DRAG_MIME)
    if (!tabId) return
    e.preventDefault()
    e.stopPropagation()
    moveTabToGroup(tabId, groupId, beforeId)
  }

  const canSplit = ui.groups.length < MAX_GROUPS
  const canToggle = ui.groups.length >= 2
  const activeCanSplit = activeTabId !== 'chat' && canSplit
  // Split when alone; flip axis when already two panes. Only the focused strip
  // shows the control so we don't double the chrome across both bars.
  const showLayoutBtn = focused
  const layoutEnabled = canToggle || activeCanSplit
  const LayoutIcon = canToggle
    ? ui.splitDir === 'row'
      ? Rows2
      : Columns2
    : Columns2
  const layoutLabel = canToggle
    ? ui.splitDir === 'row'
      ? 'Stack editors vertically'
      : 'Split editors horizontally'
    : activeCanSplit
      ? 'Split editor right'
      : 'Select a non-chat tab to split'

  return (
    <div
      className={`flex h-8 min-w-0 items-stretch border-b border-koma-border bg-koma-panel2 ${
        focused ? '' : 'opacity-75'
      }`}
      onMouseDown={() => {
        if (!focused) focusGroup(groupId)
      }}
      onDragOver={acceptStripDrag}
      onDrop={(e) => dropBefore(e, null)}
    >
      {/* Always reserve chevron width so showing/hiding never reflows the strip. */}
      <button
        type="button"
        onClick={() => scroll(-1)}
        disabled={!canScrollLeft}
        aria-label="Scroll tabs left"
        aria-hidden={!canScrollLeft}
        tabIndex={canScrollLeft ? 0 : -1}
        className={`flex w-6 flex-none items-center justify-center text-koma-fg ${
          canScrollLeft
            ? 'opacity-60 hover:bg-koma-hover hover:opacity-100'
            : 'pointer-events-none opacity-0'
        }`}
      >
        <ChevronLeft size={14} />
      </button>
      <div
        ref={containerRef}
        className="flex h-full min-w-0 flex-1 items-stretch overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {tabs.map((tab) => {
          const active = tab.id === activeTabId
          const fs =
            tab.kind === 'codingFile' ? codingDirty[fileKey(tab.root, tab.path)] : undefined
          const visual = tabVisual(tab, counts, fs)
          const { Icon } = visual
          return (
            <div
              key={tab.id}
              ref={(el) => {
                tabRefs.current.set(tab.id, el)
              }}
              role="tab"
              aria-selected={active}
              tabIndex={0}
              draggable={tab.id !== 'chat'}
              onDragStart={(e) => startDrag(e, tab.id)}
              onDragOver={acceptStripDrag}
              onDrop={(e) => dropBefore(e, tab.id)}
              onClick={() => activateTab(tab.id)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') activateTab(tab.id)
              }}
              onContextMenu={(e) => {
                e.preventDefault()
                activateTab(tab.id)
                setMenu({ x: e.clientX, y: e.clientY, tabId: tab.id })
              }}
              title={visual.title}
              className={`group relative flex h-full max-w-[220px] flex-none cursor-pointer select-none items-center gap-1.5 border-r border-koma-border pl-3 pr-1.5 text-[12px] transition-colors ${
                active
                  ? 'bg-koma-bg text-koma-fg'
                  : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
              }`}
            >
              {active && <span className="absolute inset-x-0 top-0 h-0.5 bg-koma-fg" />}
              <Icon size={13} className="flex-none opacity-80" />
              <span className="min-w-0 truncate">{visual.label}</span>
              {visual.suffix && (
                <span className="flex-none truncate text-koma-dim opacity-60">{visual.suffix}</span>
              )}
              {tab.id !== 'chat' && (
                <button
                  onClick={(e) => requestClose(tab, e)}
                  aria-label={`Close ${visual.title}`}
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 ${
                    active ? 'opacity-70' : 'opacity-0 group-hover:opacity-70'
                  }`}
                >
                  <X size={12} />
                </button>
              )}
            </div>
          )
        })}
      </div>
      <button
        type="button"
        onClick={() => scroll(1)}
        disabled={!canScrollRight}
        aria-label="Scroll tabs right"
        aria-hidden={!canScrollRight}
        tabIndex={canScrollRight ? 0 : -1}
        className={`flex w-6 flex-none items-center justify-center text-koma-fg ${
          canScrollRight
            ? 'opacity-60 hover:bg-koma-hover hover:opacity-100'
            : 'pointer-events-none opacity-0'
        }`}
      >
        <ChevronRight size={14} />
      </button>
      {showLayoutBtn && (
        <button
          type="button"
          disabled={!layoutEnabled}
          onClick={(e) => {
            e.stopPropagation()
            if (canToggle) {
              toggleSplitDir()
              return
            }
            if (activeCanSplit) splitTab(activeTabId, groupId, 'after', 'row')
          }}
          aria-label={layoutLabel}
          title={layoutLabel}
          className="flex w-7 flex-none items-center justify-center text-koma-dim opacity-70 hover:bg-koma-hover hover:text-koma-fg hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-25"
        >
          <LayoutIcon size={13} />
        </button>
      )}
      {menu && (
        <TabContextMenu
          state={menu}
          groupId={groupId}
          canSplit={menu.tabId !== 'chat' && canSplit}
          canToggle={canToggle}
          onRequestClose={(tabId) => {
            const tab = rawUi.tabs.find((candidate) => candidate.id === tabId)
            if (tab) requestClose(tab)
          }}
          onClose={() => setMenu(null)}
        />
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
