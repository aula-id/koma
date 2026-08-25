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
const CHEVRON_ON =
  'flex w-6 flex-none items-center justify-center text-koma-fg opacity-60 hover:bg-koma-hover hover:opacity-100'
const CHEVRON_OFF =
  'flex w-6 flex-none items-center justify-center text-koma-fg pointer-events-none opacity-0'

export function TabBar({ groupId, focused }: Props) {
  // Subscribe to strip-relevant ui fields only. groupSizes is deliberately
  // excluded so grip-drag pixels do not repaint both TabBars.
  const layoutBits = useKoma(
    useShallow((s) => ({
      tabs: s.ui.tabs,
      groups: s.ui.groups,
      tabGroup: s.ui.tabGroup,
      groupActive: s.ui.groupActive,
      activeGroupId: s.ui.activeGroupId,
      activeTabId: s.ui.activeTabId,
      splitDir: s.ui.splitDir,
    })),
  )
  const ui = useMemo(
    () =>
      normalizeGroups({
        tabs: layoutBits.tabs,
        groups: layoutBits.groups,
        tabGroup: layoutBits.tabGroup,
        groupActive: layoutBits.groupActive,
        activeGroupId: layoutBits.activeGroupId,
        activeTabId: layoutBits.activeTabId,
        splitDir: layoutBits.splitDir,
        // Sizes are unused for strip paint; empty map is fine (normalize fills 1s).
        groupSizes: {},
      }),
    [layoutBits],
  )
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
  // Compact dirty signature string — avoids allocating nested objects every
  // coding.files tick (useShallow would still see new child refs each time).
  const codingDirtySig = useKoma((s) => {
    const parts: string[] = []
    for (const [k, f] of Object.entries(s.coding.files)) {
      if (!f || !(f.dirty || f.conflict || f.saving)) continue
      parts.push(
        `${k}:${f.dirty ? 1 : 0}${f.conflict ? 1 : 0}${f.error ? 1 : 0}${f.binary ? 1 : 0}${
          f.tooLarge ? 1 : 0
        }${f.saving ? 1 : 0}${f.savedContent === null ? 1 : 0}`,
      )
    }
    parts.sort()
    return parts.join('|')
  })
  const codingDirty = useMemo(() => {
    const out: Record<string, CodingDirtyFlags> = {}
    if (!codingDirtySig) return out
    for (const part of codingDirtySig.split('|')) {
      const colon = part.indexOf(':')
      if (colon < 0) continue
      const k = part.slice(0, colon)
      const f = part.slice(colon + 1)
      out[k] = {
        dirty: f[0] === '1',
        conflict: f[1] === '1',
        error: f[2] === '1',
        binary: f[3] === '1',
        tooLarge: f[4] === '1',
        saving: f[5] === '1',
        savedContentNull: f[6] === '1',
      }
    }
    return out
  }, [codingDirtySig])
  const codingAutosave = useKoma((s) => !!s.settingsValues?.codingAutosave)
  const saveCodingFile = useKoma((s) => s.saveCodingFile)
  const [menu, setMenu] = useState<MenuState | null>(null)
  const [dirtyClose, setDirtyClose] = useState<{ id: string; title: string } | null>(null)
  const [awaitingAutosaveClose, setAwaitingAutosaveClose] = useState<{
    id: string
    title: string
  } | null>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const leftBtnRef = useRef<HTMLButtonElement>(null)
  const rightBtnRef = useRef<HTMLButtonElement>(null)
  const tabRefs = useRef<Map<string, HTMLDivElement | HTMLButtonElement | null>>(new Map())
  // Last overflow flags applied via DOM — NEVER React state. ResizeObserver →
  // setCanScroll* was still the #185 site (componentStack → TabBar) on split.
  const overflowRef = useRef({ left: false, right: false })

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
    const tab = ui.tabs.find((t) => t.id === awaitingAutosaveClose.id)
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
  }, [awaitingAutosaveClose, closeTab, codingDirty, ui.tabs])

  const applyOverflowDom = useCallback((left: boolean, right: boolean) => {
    const prev = overflowRef.current
    if (prev.left === left && prev.right === right) return
    overflowRef.current = { left, right }
    const lb = leftBtnRef.current
    const rb = rightBtnRef.current
    if (lb) {
      lb.disabled = !left
      lb.tabIndex = left ? 0 : -1
      lb.setAttribute('aria-hidden', left ? 'false' : 'true')
      lb.className = left ? CHEVRON_ON : CHEVRON_OFF
    }
    if (rb) {
      rb.disabled = !right
      rb.tabIndex = right ? 0 : -1
      rb.setAttribute('aria-hidden', right ? 'false' : 'true')
      rb.className = right ? CHEVRON_ON : CHEVRON_OFF
    }
  }, [])

  const checkOverflow = useCallback(() => {
    const el = containerRef.current
    if (!el) return
    const left = el.scrollLeft > 1
    const right = el.scrollWidth - el.clientWidth - el.scrollLeft > 1
    applyOverflowDom(left, right)
  }, [applyOverflowDom])

  // DOM-only overflow paint. No setState — RO under dual TabBars on split was
  // still able to exceed React's update depth when chevrons/layout chrome reflowed.
  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    let raf = 0
    const schedule = () => {
      if (raf) return
      raf = requestAnimationFrame(() => {
        raf = 0
        checkOverflow()
      })
    }
    const observer = new ResizeObserver(schedule)
    observer.observe(el)
    el.addEventListener('scroll', schedule, { passive: true })
    schedule()
    return () => {
      observer.disconnect()
      el.removeEventListener('scroll', schedule)
      if (raf) cancelAnimationFrame(raf)
    }
  }, [checkOverflow, tabs.length, ui.groups.length])

  // Reveal the active tab once when selection or strip membership changes.
  useEffect(() => {
    const strip = containerRef.current
    const active = tabRefs.current.get(activeTabId)
    if (!strip || !active) return
    const raf = requestAnimationFrame(() => {
      const sRect = strip.getBoundingClientRect()
      const aRect = active.getBoundingClientRect()
      const outside = aRect.left < sRect.left - 1 || aRect.right > sRect.right + 1
      if (outside) {
        active.scrollIntoView({ block: 'nearest', inline: 'nearest' })
      }
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
  // Always mount the layout control slot (both strips) so focus swaps never
  // change strip width by 28px and re-fire overflow RO on the neighbour bar.
  const layoutEnabled = focused && (canToggle || activeCanSplit)
  const LayoutIcon = canToggle
    ? ui.splitDir === 'row'
      ? Rows2
      : Columns2
    : Columns2
  const layoutLabel = !focused
    ? 'Focus this group to change layout'
    : canToggle
      ? ui.splitDir === 'row'
        ? 'Stack editors vertically'
        : 'Split editors horizontally'
      : activeCanSplit
        ? 'Split editor right'
        : 'Select a non-chat tab to split'

  // Density is pure CSS container queries on this strip — no React width state
  // (RO → setState dual-TabBar loops were the prior #185 class of bug).
  //   ≥320px  full labels
  //   <320px  compact padding + tighter max tab width, hide path suffix
  //   <176px  icon-only tabs (label+suffix hidden; dirty → corner dot)
  return (
    <div
      className={`@container/tabstrip flex h-8 min-w-0 items-stretch border-b border-koma-border bg-koma-panel2 ${
        focused ? '' : 'opacity-75'
      }`}
      onMouseDown={() => {
        if (!focused) focusGroup(groupId)
      }}
      onDragOver={acceptStripDrag}
      onDrop={(e) => dropBefore(e, null)}
    >
      {/* Always reserve chevron width; enable/disable via DOM only (no setState). */}
      <button
        ref={leftBtnRef}
        type="button"
        onClick={() => scroll(-1)}
        disabled
        aria-label="Scroll tabs left"
        aria-hidden="true"
        tabIndex={-1}
        className={`${CHEVRON_OFF} @max-[11rem]/tabstrip:w-5`}
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
          const dirtyDot = !!fs?.dirty
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
              className={`group relative flex h-full max-w-[220px] flex-none cursor-pointer select-none items-center gap-1.5 border-r border-koma-border pl-3 pr-1.5 text-[12px] transition-colors @max-xs/tabstrip:max-w-[148px] @max-xs/tabstrip:gap-1 @max-xs/tabstrip:pl-2 @max-xs/tabstrip:pr-1 @max-[11rem]/tabstrip:max-w-none @max-[11rem]/tabstrip:gap-0 @max-[11rem]/tabstrip:px-1.5 ${
                active
                  ? 'bg-koma-bg text-koma-fg'
                  : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
              }`}
            >
              {active && <span className="absolute inset-x-0 top-0 h-0.5 bg-koma-fg" />}
              <span className="relative flex-none">
                <Icon size={13} className="opacity-80" />
                {/* Icon-only density: dirty marker moves off the hidden label. */}
                {dirtyDot && (
                  <span
                    aria-hidden
                    className="absolute -right-0.5 -bottom-0.5 hidden h-1.5 w-1.5 rounded-full bg-koma-accent @max-[11rem]/tabstrip:block"
                  />
                )}
              </span>
              <span className="min-w-0 truncate @max-[11rem]/tabstrip:hidden">{visual.label}</span>
              {visual.suffix && (
                <span className="flex-none truncate text-koma-dim opacity-60 @max-xs/tabstrip:hidden">
                  {visual.suffix}
                </span>
              )}
              {tab.id !== 'chat' && (
                <button
                  onClick={(e) => requestClose(tab, e)}
                  aria-label={`Close ${visual.title}`}
                  title="Close"
                  className={`ml-0.5 flex h-4 w-4 flex-none items-center justify-center rounded transition hover:bg-koma-hover hover:!opacity-100 @max-[11rem]/tabstrip:ml-0 ${
                    active
                      ? 'opacity-70'
                      : 'opacity-0 group-hover:opacity-70 @max-[11rem]/tabstrip:opacity-50'
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
        ref={rightBtnRef}
        type="button"
        onClick={() => scroll(1)}
        disabled
        aria-label="Scroll tabs right"
        aria-hidden="true"
        tabIndex={-1}
        className={`${CHEVRON_OFF} @max-[11rem]/tabstrip:w-5`}
      >
        <ChevronRight size={14} />
      </button>
      <button
        type="button"
        disabled={!layoutEnabled}
        onClick={(e) => {
          e.stopPropagation()
          if (!focused) {
            focusGroup(groupId)
            return
          }
          if (canToggle) {
            toggleSplitDir()
            return
          }
          if (activeCanSplit) splitTab(activeTabId, groupId, 'after', 'row')
        }}
        aria-label={layoutLabel}
        title={layoutLabel}
        className={`flex w-7 flex-none items-center justify-center text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:cursor-not-allowed @max-[11rem]/tabstrip:w-6 ${
          focused ? 'opacity-70 hover:opacity-100' : 'opacity-25'
        } disabled:opacity-25`}
      >
        <LayoutIcon size={13} />
      </button>
      {menu && (
        <TabContextMenu
          state={menu}
          groupId={groupId}
          canSplit={menu.tabId !== 'chat' && canSplit}
          canToggle={canToggle}
          onRequestClose={(tabId) => {
            const tab = ui.tabs.find((candidate) => candidate.id === tabId)
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
