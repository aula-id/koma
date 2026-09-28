import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { ChevronDown, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { CodingTasks } from './CodingTasks'
import { LspDrawer } from './LspDrawer'
import { ProblemsDrawer } from './ProblemsDrawer'

const HEIGHT_KEY = 'koma.bottom-panel.height'
const DEFAULT_HEIGHT = 280
const tabs = [
  { id: 'tasks', label: 'Tasks' },
  { id: 'problems', label: 'Problems' },
  { id: 'lsp', label: 'Language Servers' },
] as const

function savedHeight() {
  try {
    const value = Number(localStorage.getItem(HEIGHT_KEY))
    if (Number.isFinite(value) && value >= 160 && value <= 2000) return value
  } catch { /* Storage is optional in embedded webviews. */ }
  return DEFAULT_HEIGHT
}

/** Shared dock: content keeps its state when hidden; native task jobs live independently. */
export function BottomPanel() {
  const tab = useKoma(s => s.bottomPanelTab)
  const setTab = useKoma(s => s.setBottomPanelTab)
  const problems = useKoma(s => s.lspDiagnostics)
  const servers = useKoma(s => s.lspRuntime)
  const count = Object.values(problems).reduce((total, rows) => total + rows.length, 0)
  const [height, setHeight] = useState(savedHeight)
  const [maximum, setMaximum] = useState(600)
  const [resizing, setResizing] = useState(false)
  const panel = useRef<HTMLElement>(null)
  const buttons = useRef<Partial<Record<NonNullable<typeof tab>, HTMLButtonElement | null>>>({})
  const drag = useRef<{ y: number; height: number } | null>(null)
  const previousFocus = useRef<HTMLElement | null>(null)
  const wasOpen = useRef(false)
  const minimum = Math.min(160, maximum)
  const actualHeight = Math.min(maximum, Math.max(minimum, height))
  const clamp = (value: number) => Math.min(maximum, Math.max(minimum, value))

  useLayoutEffect(() => {
    const parent = panel.current?.parentElement
    if (!parent) return
    const observer = new ResizeObserver(() => {
      setMaximum(Math.max(80, Math.floor(parent.clientHeight * 0.75)))
    })
    observer.observe(parent)
    return () => observer.disconnect()
  }, [])
  useLayoutEffect(() => {
    if (tab) {
      if (!wasOpen.current) previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
      buttons.current[tab]?.focus({ preventScroll: true })
    }
    wasOpen.current = !!tab
    if (!tab) { drag.current = null; setResizing(false) }
  }, [tab])
  useEffect(() => {
    const timer = setTimeout(() => {
      try { localStorage.setItem(HEIGHT_KEY, String(height)) } catch { /* Optional preference. */ }
    }, 150)
    return () => clearTimeout(timer)
  }, [height])
  useEffect(() => {
    if (!resizing) return
    const { cursor, userSelect } = document.body.style
    document.body.style.cursor = 'ns-resize'
    document.body.style.userSelect = 'none'
    return () => { document.body.style.cursor = cursor; document.body.style.userSelect = userSelect }
  }, [resizing])
  const close = () => {
    setTab(null)
    if (previousFocus.current?.isConnected) previousFocus.current.focus({ preventScroll: true })
  }

  return <section ref={panel} id="workspace-bottom-panel" aria-label="Workspace panel"
    style={{ height: actualHeight }}
    className={`${tab ? 'flex' : 'hidden'} relative w-full min-w-0 flex-none flex-col border-t border-koma-border bg-koma-panel text-koma-fg`}
    onKeyDown={event => {
      // Let child controls (notably terminal Escape) handle their own keys.
      if (event.key === 'Escape' && !event.defaultPrevented && event.target === document.activeElement
        && (event.target as HTMLElement).closest('[role="tablist"], [role="separator"]')) {
        event.stopPropagation(); close()
      }
    }}>
    <div role="separator" tabIndex={0} aria-label="Resize workspace panel" aria-orientation="horizontal"
      aria-controls="workspace-bottom-panel" aria-valuemin={minimum} aria-valuemax={maximum} aria-valuenow={actualHeight}
      className="absolute -top-[3px] z-30 h-[5px] w-full touch-none cursor-ns-resize hover:bg-koma-grip focus-visible:bg-koma-grip focus-visible:outline-none"
      onDoubleClick={() => setHeight(clamp(DEFAULT_HEIGHT))}
      onPointerDown={event => {
        if (event.button !== 0) return
        event.preventDefault()
        drag.current = { y: event.clientY, height: actualHeight }
        event.currentTarget.setPointerCapture(event.pointerId)
        setResizing(true)
      }}
      onPointerMove={event => { if (drag.current) setHeight(clamp(drag.current.height + drag.current.y - event.clientY)) }}
      onPointerUp={event => {
        drag.current = null; setResizing(false)
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId)
      }}
      onLostPointerCapture={() => { drag.current = null; setResizing(false) }}
      onPointerCancel={() => { drag.current = null; setResizing(false) }}
      onKeyDown={event => {
        const delta = event.shiftKey ? 50 : 20
        const next = event.key === 'ArrowUp' ? actualHeight + delta : event.key === 'ArrowDown' ? actualHeight - delta
          : event.key === 'Home' ? minimum : event.key === 'End' ? maximum : null
        if (next !== null) { event.preventDefault(); setHeight(clamp(next)) }
      }} />
    <div className="flex h-8 min-w-0 flex-none items-center border-b border-koma-border px-2">
      <div role="tablist" aria-label="Workspace panel tabs" className="flex h-full min-w-0 flex-1 items-stretch overflow-x-auto">
        {tabs.map((item, index) => <button key={item.id} ref={node => { buttons.current[item.id] = node }}
          type="button" role="tab" id={`workspace-tab-${item.id}`} aria-controls={`workspace-panel-${item.id}`}
          aria-selected={tab === item.id} tabIndex={tab === item.id ? 0 : -1}
          onClick={() => setTab(item.id)}
          onKeyDown={event => {
            const next = event.key === 'ArrowRight' ? (index + 1) % tabs.length
              : event.key === 'ArrowLeft' ? (index + tabs.length - 1) % tabs.length
                : event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : null
            if (next !== null) { event.preventDefault(); setTab(tabs[next].id); buttons.current[tabs[next].id]?.focus() }
          }}
          className={`flex flex-none items-center gap-1.5 border-b-2 px-2 text-[11px] uppercase tracking-wide outline-none focus-visible:bg-koma-hover ${tab === item.id ? 'border-koma-accent text-koma-fg' : 'border-transparent text-koma-dim hover:text-koma-fg'}`}>
          {item.label}
          {item.id !== 'tasks' && <span className="text-[10px] tabular-nums text-koma-dim">{item.id === 'lsp' ? servers.length : count}</span>}
        </button>)}
      </div>
      <button type="button" aria-label="Collapse workspace panel" title="Collapse panel"
        onClick={close} className="ml-1 flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"><ChevronDown size={14} /></button>
      <button type="button" aria-label="Close workspace panel" title="Close panel (tasks keep running)"
        onClick={close} className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"><X size={13} /></button>
    </div>
    {tabs.map(item => <div key={item.id} role="tabpanel" id={`workspace-panel-${item.id}`} aria-labelledby={`workspace-tab-${item.id}`} tabIndex={tab === item.id ? 0 : -1}
      className={`${tab === item.id ? 'flex' : 'hidden'} min-h-0 min-w-0 flex-1 flex-col overflow-hidden`}>
      {item.id === 'tasks' ? <CodingTasks visible={tab === 'tasks'} /> : item.id === 'lsp' ? <LspDrawer /> : <ProblemsDrawer />}
    </div>)}
  </section>
}
