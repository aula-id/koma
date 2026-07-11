import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { Files, GitBranch, Blocks, Plug, Bot, ChartColumn, CircleHelp, Settings, MoreHorizontal } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import type { SidebarView } from './Sidebar'
import { useKoma, resolveActivityBarOrder } from '../store/koma'

type ActivityBarProps = {
  activeView: SidebarView
  sidebarOpen: boolean
  onSelect: (view: SidebarView) => void
  onSettings?: () => void
  onHelp?: () => void
}

type ActivityBarItem = { view: SidebarView; icon: LucideIcon; label: string }

const iconBtn =
  'relative flex h-10 w-10 items-center justify-center rounded-md text-koma-fg opacity-50 transition hover:bg-koma-hover hover:opacity-85'

// The canonical, data-driven list of built-in activity-bar icons — the source
// of truth for both the bar itself and the Settings "Sidebar" section's
// toggle list. `view` is the stable id persisted in `activityBar.order`/
// `hidden` (store/koma.ts). Extension-contributed icons (a later wave) will
// plug into this same shape; adding one here is the only change that wave
// needs on the ActivityBar side.
export const ACTIVITY_BAR_ITEMS: ActivityBarItem[] = [
  { view: 'explore', icon: Files, label: 'Explore' },
  { view: 'git', icon: GitBranch, label: 'Source Control' },
  { view: 'mcp', icon: Blocks, label: 'MCP' },
  { view: 'connector', icon: Plug, label: 'Connector' },
  { view: 'agents', icon: Bot, label: 'Agents' },
  { view: 'usage', icon: ChartColumn, label: 'Usage' },
  { view: 'store', icon: Blocks, label: 'Extensions' },
]

// Per-button footprint used for the overflow measurement below: the iconBtn's
// h-10 (40px) plus the strip's gap-0.5 (2px) between siblings.
const ITEM_H = 42
// Help + Settings are fixed chrome (never reorderable/hideable/overflow-able),
// always pinned at the bottom — reserved out of the measured height first.
const FOOTER_H = ITEM_H * 2

// Thin icon strip. Selecting a view switches the sidebar panel; the active
// view shows the left indicator bar. The managed items (ACTIVITY_BAR_ITEMS)
// are data-driven: drag-reorderable (hand-rolled HTML5 DnD) and individually
// hideable (Settings "Sidebar" section). A hidden item, or one that no longer
// fits the bar's measured height, moves into the "…" overflow menu ("Additional
// Views") instead of disappearing. Help + Settings are pinned to the bottom
// (both inert re: active-state — neither is a `SidebarView` — and neither is
// part of the managed/reorderable list), Help directly above Settings, the
// overflow button (when shown) directly above Help.
export function ActivityBar({ activeView, sidebarOpen, onSelect, onSettings, onHelp }: ActivityBarProps) {
  const order = useKoma((s) => s.activityBar.order)
  const hidden = useKoma((s) => s.activityBar.hidden)
  const setActivityBarOrder = useKoma((s) => s.setActivityBarOrder)

  const allIds = useMemo(() => ACTIVITY_BAR_ITEMS.map((i) => i.view), [])
  const itemByView = useMemo(
    () => new Map<string, ActivityBarItem>(ACTIVITY_BAR_ITEMS.map((i) => [i.view, i])),
    [],
  )
  // Full effective order across EVERY known item (visible + config-hidden),
  // forward-compatible with an id `order` doesn't know about yet.
  const effectiveOrder = useMemo(() => resolveActivityBarOrder(order, allIds), [order, allIds])
  const hiddenSet = useMemo(() => new Set(hidden), [hidden])

  const configVisible = useMemo(
    () =>
      effectiveOrder
        .filter((v) => !hiddenSet.has(v))
        .map((v) => itemByView.get(v))
        .filter((x): x is ActivityBarItem => !!x),
    [effectiveOrder, hiddenSet, itemByView],
  )
  const configHiddenItems = useMemo(
    () =>
      effectiveOrder
        .filter((v) => hiddenSet.has(v))
        .map((v) => itemByView.get(v))
        .filter((x): x is ActivityBarItem => !!x),
    [effectiveOrder, hiddenSet, itemByView],
  )

  // Overflow-by-height: how many of `configVisible`'s LEADING items fit in the
  // bar's measured available height before the fixed Help/Settings footer (plus
  // the overflow button's own slot, when one turns out to be needed). Measured
  // via ResizeObserver on the outer column (this bar spans the full window
  // height, so its clientHeight IS the available space) rather than a fixed
  // item count, so shrinking the window dynamically pushes the tail into the
  // "…" menu. Starts at Infinity (render everything) so the first paint, before
  // the effect measures, never flashes an empty/truncated bar.
  const containerRef = useRef<HTMLDivElement>(null)
  const [fitCount, setFitCount] = useState(Infinity)

  useLayoutEffect(() => {
    const el = containerRef.current
    if (!el) return
    const compute = () => {
      const total = el.clientHeight
      const availableNoMenu = Math.max(0, total - FOOTER_H)
      const fitsAll = Math.floor(availableNoMenu / ITEM_H)
      // The "…" button is needed either because the visible set doesn't fit, OR
      // because some item is config-hidden (it must be reachable somewhere).
      const needsMenu = configVisible.length > fitsAll || configHiddenItems.length > 0
      const available = needsMenu ? Math.max(0, availableNoMenu - ITEM_H) : availableNoMenu
      setFitCount(Math.max(0, Math.floor(available / ITEM_H)))
    }
    compute()
    const ro = new ResizeObserver(compute)
    ro.observe(el)
    return () => ro.disconnect()
  }, [configVisible.length, configHiddenItems.length])

  const barItems = configVisible.slice(0, fitCount)
  const overflowTail = configVisible.slice(fitCount)
  // One menu, both sources — config-hidden first, then whatever overflowed the
  // measured height, mirroring VSCode's single "Additional Views" popup.
  const menuItems = [...configHiddenItems, ...overflowTail]
  const showMenuButton = menuItems.length > 0

  // Hand-rolled HTML5 drag-reorder — no new dependency. Dragging a bar button
  // onto another moves it to just before the drop target in the FULL effective
  // order (not just the bar-rendered subset), so a config-hidden item elsewhere
  // in the order keeps its relative position untouched.
  const [draggedView, setDraggedView] = useState<string | null>(null)
  const [dragOverView, setDragOverView] = useState<string | null>(null)

  const reorder = (targetView: string) => {
    if (!draggedView || draggedView === targetView) return
    const withoutSource = effectiveOrder.filter((id) => id !== draggedView)
    const targetIdx = withoutSource.indexOf(targetView)
    const next = [...withoutSource.slice(0, targetIdx), draggedView, ...withoutSource.slice(targetIdx)]
    setActivityBarOrder(next)
  }

  // "…" overflow ("Additional Views") popup — closes on outside click, Escape,
  // or picking an item.
  const [menuOpen, setMenuOpen] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)
  const menuBtnRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (!menuOpen) return
    const onMouseDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (menuRef.current?.contains(target) || menuBtnRef.current?.contains(target)) return
      setMenuOpen(false)
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMenuOpen(false)
    }
    window.addEventListener('mousedown', onMouseDown)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('mousedown', onMouseDown)
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [menuOpen])

  const selectFromMenu = (view: SidebarView) => {
    setMenuOpen(false)
    onSelect(view)
  }

  return (
    <div
      ref={containerRef}
      className="flex w-12 flex-none flex-col items-center gap-0.5 border-r border-koma-border bg-koma-panel2 pt-1.5"
    >
      {barItems.map(({ view, icon: Icon, label }) => {
        const active = sidebarOpen && activeView === view
        const dragging = draggedView === view
        const dragOver = dragOverView === view && draggedView !== view
        return (
          <button
            key={view}
            draggable
            onDragStart={() => setDraggedView(view)}
            onDragOver={(e) => {
              e.preventDefault()
              if (view !== draggedView) setDragOverView(view)
            }}
            onDragLeave={() => setDragOverView((v) => (v === view ? null : v))}
            onDrop={(e) => {
              e.preventDefault()
              setDragOverView(null)
              reorder(view)
              setDraggedView(null)
            }}
            onDragEnd={() => {
              setDraggedView(null)
              setDragOverView(null)
            }}
            onClick={() => onSelect(view)}
            title={label}
            aria-label={label}
            className={`${iconBtn} ${active ? '!opacity-100' : ''} ${dragging ? 'opacity-25' : ''} ${
              dragOver ? 'ring-1 ring-inset ring-koma-accent/60' : ''
            }`}
          >
            {active && <span className="absolute left-0 top-2 bottom-2 w-0.5 rounded-sm bg-koma-fg" />}
            <Icon size={22} strokeWidth={1.6} />
          </button>
        )
      })}

      {showMenuButton && (
        <div className="relative mt-auto">
          <button
            ref={menuBtnRef}
            onClick={() => setMenuOpen((v) => !v)}
            title="Additional Views"
            aria-label="Additional Views"
            className={`${iconBtn} ${menuOpen ? '!opacity-100' : ''}`}
          >
            <MoreHorizontal size={22} strokeWidth={1.6} />
          </button>
          {menuOpen && (
            <div
              ref={menuRef}
              className="absolute bottom-0 left-full z-20 ml-1 w-52 rounded-md border border-koma-border bg-koma-panel2 py-1 shadow-lg"
            >
              <div className="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-40">
                Additional Views
              </div>
              {menuItems.map(({ view, icon: Icon, label }) => (
                <button
                  key={view}
                  onClick={() => selectFromMenu(view)}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12.5px] text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100"
                >
                  <Icon size={14} className="flex-none opacity-70" />
                  <span className="truncate">{label}</span>
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      <button
        onClick={onHelp}
        className={`${iconBtn} ${showMenuButton ? '' : 'mt-auto'}`}
        title="Help"
        aria-label="Help"
      >
        <CircleHelp size={22} strokeWidth={1.6} />
      </button>
      <button
        onClick={onSettings}
        className={`${iconBtn} mb-1.5`}
        title="Settings"
        aria-label="Settings"
      >
        <Settings size={22} strokeWidth={1.6} />
      </button>
    </div>
  )
}
