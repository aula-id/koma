import { useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { ArrowRight, ChevronDown, ChevronRight, Clock, Folder, FolderOpen, FolderPlus, Info, Search, Server, Sparkles, X, Zap } from 'lucide-react'
import { NewSessionMenu } from './NewSessionMenu'
import { SessionRowActions, SessionRowConfirmStrip, type ArmedRow } from './SessionRowActions'
import { SessionBulkBar } from './SessionBulkBar'
import { useSessionMultiSelect } from './sessionListSelection'
import { useKoma, isDying } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'
import { groupRecentByFolder } from '../lib/sessionFolderGroups'

// Measures the component's own width with a ResizeObserver (a container query in
// JS) so the start screen can flip stacked -> side-by-side against the ACTUAL
// space it gets (the main area minus sidebar/activity-bar), not the raw
// viewport — which a window media query would get wrong when the sidebar is open.
function useContainerWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null)
  const [width, setWidth] = useState(0)
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) setWidth(e.contentRect.width)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])
  return [ref, width] as const
}

function Card({ children, className = '' }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={`rounded-xl border border-koma-border bg-koma-panel2 p-4 ${className}`}>
      {children}
    </div>
  )
}

function SectionLabel({ icon: Icon, children, className = '' }: { icon: typeof Clock; children: string; className?: string }) {
  return (
    <div className={`flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-45 ${className}`}>
      <Icon size={12} className="flex-none" />
      {children}
    </div>
  )
}

// VSCode-style pre-session START SCREEN — rendered INSTEAD of ChatView whenever
// no session is attached (the swapper/empty state). Quick-open recent sessions
// (from the host's authoritative hub history/cooking mirror) + a New session
// action (reuses the native folder-picker flow via GuiReq NewSession) + a short
// "about Koma" panel. Responsive: stacked when narrow, side-by-side when wide.
//
// Session rows (issue #126): plain click selects/highlights; Ctrl/Cmd toggles;
// Shift ranges; double-click or Enter opens. Bulk Kill/Delete via SessionBulkBar.
export function StartScreen() {
  const history = useKoma((s) => s.hub.history)
  const cooking = useKoma((s) => s.hub.cooking)
  const hubReady = useKoma((s) => s.hub.state) !== null
  const req = useKoma((s) => s.req)
  const startSwitching = useKoma((s) => s.startSwitching)
  const requestRemotePath = useKoma((s) => s.requestRemotePath)
  const dyingSessions = useKoma((s) => s.dyingSessions)
  const remoteState = useKoma((s) => s.remoteState)
  const remotePathState = useKoma((s) => s.remotePath.state)
  const switchingTo = useKoma((s) => s.ui.switchingTo)
  const [ref, width] = useContainerWidth<HTMLDivElement>()
  const wide = width >= 760
  // The single armed row (kill/delete confirm pill) across BOTH lists — arming
  // a different row disarms whichever was armed before.
  const [armed, setArmed] = useState<ArmedRow>(null)
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set())
  const [searching, setSearching] = useState(false)
  const [query, setQuery] = useState('')
  const searchRef = useRef<HTMLInputElement>(null)
  const multi = useSessionMultiSelect()

  // The host only discovers live sessions on demand — nudge a fresh Hub on
  // mount and on a short interval so recent/live rows stay current (same cadence
  // as ResumePalette). Pause while a swap is in flight: RefreshHub replies are
  // real Hub envelopes and must not race the attach path (StartScreen stays
  // mounted until Snapshot sets session.id).
  useEffect(() => {
    if (switchingTo) return
    req({ r: 'RefreshHub' })
    const id = window.setInterval(() => req({ r: 'RefreshHub' }), 2000)
    return () => window.clearInterval(id)
  }, [req, switchingTo])

  // Escape: search → multi-select → armed row.
  const multiHas = multi.hasSelection
  const multiClear = multi.clear
  useEffect(() => {
    if (!armed && !multiHas && !searching) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (query) {
        setQuery('')
        return
      }
      if (searching) {
        setSearching(false)
        return
      }
      if (multiHas) {
        multiClear()
        // Avoid leaving a browser focus ring on the last-clicked row.
        if (document.activeElement instanceof HTMLElement) document.activeElement.blur()
        return
      }
      if (armed) setArmed(null)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [armed, multiHas, multiClear, searching, query])

  useEffect(() => {
    multiClear()
  }, [query, multiClear])

  // Live (cooking) sessions first, then past history — grouped client-side by
  // folder basename (`dirLabel`). No daemon/IPC change; empty dirLabel is Other.
  const liveSessions = useMemo(
    () => cooking.filter((c) => c.kind === 'session' && c.id),
    [cooking],
  )
  const folderGroups = useMemo(
    () => groupRecentByFolder(liveSessions, history),
    [liveSessions, history],
  )
  const q = query.trim().toLowerCase()
  const matches = (name: string, id: string, dirLabel?: string | null) =>
    q === '' ||
    name.toLowerCase().includes(q) ||
    id.toLowerCase().includes(q) ||
    (dirLabel ?? '').toLowerCase().includes(q)
  const visibleGroups = useMemo(() => {
    if (!q) return folderGroups
    return folderGroups
      .map((group) => ({
        ...group,
        live: group.live.filter((row) => matches(row.name, row.id ?? '', row.dirLabel)),
        history: group.history.filter((row) => matches(row.name, row.id, row.dirLabel)),
      }))
      .filter((group) => group.live.length + group.history.length > 0 || group.label.toLowerCase().includes(q))
  }, [folderGroups, q])
  const nestFolders = folderGroups.length > 1
  const searchingActive = q.length > 0
  const liveIds = useMemo(() => {
    const ids: string[] = []
    for (const group of visibleGroups) {
      const open = !nestFolders || searchingActive || expanded.has(group.key)
      if (!open) continue
      for (const row of group.live) if (row.id) ids.push(row.id)
    }
    return ids
  }, [visibleGroups, nestFolders, searchingActive, expanded])
  const historyIds = useMemo(() => {
    const ids: string[] = []
    for (const group of visibleGroups) {
      const open = !nestFolders || searchingActive || expanded.has(group.key)
      if (!open) continue
      for (const row of group.history) ids.push(row.id)
    }
    return ids
  }, [visibleGroups, nestFolders, searchingActive, expanded])
  const toggleFolder = (key: string) => {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }
  const openSearch = () => {
    setSearching(true)
    requestAnimationFrame(() => searchRef.current?.focus())
  }
  const closeSearch = () => {
    setSearching(false)
    setQuery('')
  }

  const openSession = (id: string, name: string) => {
    // Optimistic swap overlay (no host "swap started" push; attach can block for
    // seconds) — cleared by the next authoritative Snapshot. Mirrors ResumePalette.
    startSwitching(name)
    requestAnimationFrame(() => req({ r: 'SelectSession', id }))
  }
  // Local: native folder picker. No optimistic loader — cancel would strand it.
  // Remote: SSH path picker (requestRemotePath guards re-entry).
  const newSession = () => {
    if (remoteState.state === 'ready' || remoteState.state === 'connected') {
      requestRemotePath()
      return
    }
    req({ r: 'NewSession', folder: true })
  }
  const remotePathBusy =
    remotePathState === 'listing' ||
    remotePathState === 'ready' ||
    remotePathState === 'error'

  const armRow = (row: ArmedRow) => {
    multi.clear()
    setArmed(row)
  }

  const onRowMouse = (
    e: ReactMouseEvent,
    kind: 'session' | 'history',
    id: string,
    ordered: string[],
  ) => {
    if (armed) setArmed(null)
    multi.onRowClick(e, kind, id, ordered)
  }

  const hasRecent = liveSessions.length > 0 || history.length > 0
  const bulkCooking = multi.selectedIds('session')
  const bulkHistory = multi.selectedIds('history')
  const fgCooking = liveSessions.filter((c) => c.foreground && c.id).map((c) => c.id as string)
  const remoteLive =
    remoteState.state === 'ready' || remoteState.state === 'connected'
  const remoteTarget =
    remoteLive && remoteState.user && remoteState.host
      ? `${remoteState.user}@${remoteState.host}`
      : null

  const actions = (
    <div className="flex min-w-0 flex-1 flex-col gap-4">
      <div>
        <div className="mb-1 flex items-baseline gap-2">
          <span className="text-[22px] font-bold text-koma-fg">koma</span>
          <span className="text-[12px] text-koma-fg opacity-45">
            {remoteTarget ? 'remote session' : 'start a session'}
          </span>
        </div>
        {remoteTarget && (
          <div
            title={`Connected to ${remoteTarget}`}
            className="mt-1 inline-flex max-w-full items-center gap-1.5 rounded-md bg-koma-accent/10 px-2 py-0.5 text-[11px] text-koma-accent"
          >
            <Server size={11} className="flex-none opacity-80" />
            <span className="truncate">{remoteTarget}</span>
          </div>
        )}
      </div>

      <div className={`group flex items-center rounded-xl border border-koma-border bg-koma-panel transition-colors ${remotePathBusy ? 'opacity-70' : 'hover:border-koma-accent/60 hover:bg-koma-hover'}`}>
        <button
          onClick={newSession}
          disabled={remotePathBusy}
          className="flex min-w-0 flex-1 items-center gap-3 px-4 py-3 text-left disabled:cursor-wait"
        >
          <span className="flex h-9 w-9 flex-none items-center justify-center rounded-lg bg-koma-accent/15 text-koma-accent">
            {remotePathBusy ? <BrailleSpinner size={18} /> : <FolderPlus size={18} />}
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-[13px] font-semibold text-koma-fg">New session</span>
            <span className="block text-[11px] text-koma-fg opacity-45">
              {remotePathBusy
                ? 'Opening remote folder picker…'
                : remoteTarget
                  ? `Pick a folder on ${remoteTarget}`
                  : 'Choose a folder'}
            </span>
          </span>
          {remotePathBusy ? (
            <span className="flex-none" />
          ) : (
            <ArrowRight size={16} className="flex-none text-koma-fg opacity-30 transition group-hover:translate-x-0.5 group-hover:opacity-70" />
          )}
        </button>
        <NewSessionMenu className="pr-3" />
      </div>

      <Card className="!p-0 overflow-hidden">
        <div className="flex h-11 shrink-0 items-center gap-2 px-4">
          {multi.hasSelection ? (
            <SessionBulkBar
              cookingIds={bulkCooking}
              historyIds={bulkHistory}
              foregroundCookingIds={fgCooking}
              onDone={() => multi.clear()}
              onClear={() => multi.clear()}
              className="h-7 min-w-0 flex-1"
            />
          ) : (
            <>
              <div className="flex-none">
                <SectionLabel icon={Clock}>Recent</SectionLabel>
              </div>
              {searching ? (
                <label className="relative min-w-0 flex-1">
                  <Search size={12} className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-koma-dim" aria-hidden />
                  <input
                    ref={searchRef}
                    aria-label="Search sessions"
                    placeholder="Search sessions"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    className="h-7 w-full rounded border border-koma-border bg-koma-bg py-0 pr-7 pl-7 text-[12px] text-koma-fg outline-none placeholder:text-koma-dim focus:border-koma-accent"
                  />
                  <button
                    type="button"
                    onClick={closeSearch}
                    aria-label="Close search"
                    className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
                  >
                    <X size={12} />
                  </button>
                </label>
              ) : hasRecent ? (
                <button
                  type="button"
                  onClick={openSearch}
                  aria-label="Search sessions"
                  title="Search sessions"
                  className="ml-auto flex h-7 w-7 flex-none items-center justify-center rounded border border-koma-border text-koma-dim transition-colors hover:bg-koma-hover hover:text-koma-fg"
                >
                  <Search size={13} />
                </button>
              ) : null}
            </>
          )}
        </div>
        {!hubReady ? (
          <div className="flex items-center gap-2 px-5 pb-4 pt-2 text-[12px] text-koma-dim">
            <BrailleSpinner size={14} />
            Loading sessions…
          </div>
        ) : !hasRecent ? (
          <div className="px-5 pb-4 pt-2 text-[12px] text-koma-fg opacity-35">No sessions yet — start a new one.</div>
        ) : (
          <div className="max-h-[40vh] overflow-y-auto px-3 pb-3">
            {visibleGroups.map((group) => {
              const open = !nestFolders || searchingActive || expanded.has(group.key)
              const count = group.live.length + group.history.length
              return (
                <div key={group.key || 'other'}>
                  {nestFolders && (
                    <button
                      type="button"
                      onClick={() => toggleFolder(group.key)}
                      aria-expanded={open}
                      className="flex h-7 w-full items-center gap-1 rounded px-1 text-left text-[12px] text-koma-fg hover:bg-koma-hover"
                    >
                      {open ? <ChevronDown size={13} className="flex-none text-koma-dim" /> : <ChevronRight size={13} className="flex-none text-koma-dim" />}
                      {open ? (
                        <FolderOpen size={13} className="flex-none text-koma-accent opacity-80" />
                      ) : (
                        <Folder size={13} className="flex-none text-koma-accent opacity-80" />
                      )}
                      <span className="min-w-0 flex-1 truncate">{group.label}</span>
                      <span className="flex-none text-[10px] text-koma-dim">{count}</span>
                    </button>
                  )}
                  {open && (
                    <div className={nestFolders ? 'pl-3' : undefined}>
                      {group.live.map((c) => {
                        const id = c.id as string
                        const dying = isDying(dyingSessions, id, 'session')
                        const rowArmed = armed?.id === id && armed.kind === 'session'
                        const sel = multi.isSelected('session', id)
                        return (
                          <div
                            key={id}
                            role="button"
                            tabIndex={dying || rowArmed ? -1 : 0}
                            aria-selected={sel}
                            onClick={(e) => {
                              if (dying || rowArmed) return
                              onRowMouse(e, 'session', id, liveIds)
                            }}
                            onDoubleClick={(e) => {
                              if (dying || rowArmed) return
                              e.preventDefault()
                              openSession(id, c.name)
                            }}
                            onKeyDown={(e) => {
                              if (e.key !== 'Enter' && e.key !== ' ') return
                              if (e.key === ' ') e.preventDefault()
                              if (!dying && !armed) openSession(id, c.name)
                            }}
                            className={`group flex w-full cursor-pointer items-center justify-between rounded-lg text-left transition-colors outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-koma-accent/50 ${
                              rowArmed ? '' : 'gap-2 px-3 py-2'
                            } ${dying ? 'pointer-events-none opacity-60' : ''} ${
                              rowArmed ? '' : sel ? 'bg-koma-accent/15 hover:bg-koma-accent/20' : 'hover:bg-koma-hover'
                            }`}
                          >
                            {rowArmed ? (
                              <SessionRowConfirmStrip
                                id={id}
                                kind="session"
                                foreground={c.foreground}
                                onCancel={() => setArmed(null)}
                                className="rounded-lg px-3 py-2"
                              />
                            ) : (
                              <>
                                <div className="flex min-w-0 flex-1 items-center gap-2">
                                  <span className="h-1.5 w-1.5 flex-none animate-pulse rounded-full bg-emerald-500" />
                                  <span className="min-w-0 flex-1 truncate text-[12.5px] text-koma-fg">{c.name}</span>
                                  {c.foreground && (
                                    <span className="flex-none rounded border border-koma-border px-1 text-[9px] uppercase tracking-wide text-koma-fg opacity-50">
                                      current
                                    </span>
                                  )}
                                  {!nestFolders && c.dirLabel && (
                                    <span className="max-w-[40%] flex-none truncate text-[11px] text-koma-fg opacity-40">
                                      {c.dirLabel}
                                    </span>
                                  )}
                                </div>
                                <div className="flex w-7 flex-none items-center justify-center">
                                  <SessionRowActions id={id} kind="session" armed={armed} onArm={armRow} />
                                </div>
                              </>
                            )}
                          </div>
                        )
                      })}
                      {group.history.map((h) => {
                        const dying = isDying(dyingSessions, h.id, 'history')
                        const rowArmed = armed?.id === h.id && armed.kind === 'history'
                        const sel = multi.isSelected('history', h.id)
                        return (
                          <div
                            key={h.id}
                            role="button"
                            tabIndex={dying || rowArmed ? -1 : 0}
                            aria-selected={sel}
                            onClick={(e) => {
                              if (dying || rowArmed) return
                              onRowMouse(e, 'history', h.id, historyIds)
                            }}
                            onDoubleClick={(e) => {
                              if (dying || rowArmed) return
                              e.preventDefault()
                              openSession(h.id, h.name)
                            }}
                            onKeyDown={(e) => {
                              if (e.key !== 'Enter' && e.key !== ' ') return
                              if (e.key === ' ') e.preventDefault()
                              if (!dying && !armed) openSession(h.id, h.name)
                            }}
                            className={`group flex w-full cursor-pointer items-center justify-between rounded-lg text-left transition-colors outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-koma-accent/50 ${
                              rowArmed ? '' : 'gap-2 px-3 py-2'
                            } ${dying ? 'pointer-events-none opacity-60' : ''} ${
                              rowArmed ? '' : sel ? 'bg-koma-accent/15 hover:bg-koma-accent/20' : 'hover:bg-koma-hover'
                            }`}
                          >
                            {rowArmed ? (
                              <SessionRowConfirmStrip
                                id={h.id}
                                kind="history"
                                onCancel={() => setArmed(null)}
                                className="rounded-lg px-3 py-2"
                              />
                            ) : (
                              <>
                                <div className="flex min-w-0 flex-1 items-center gap-2">
                                  <span className="min-w-0 flex-1 truncate text-[12.5px] text-koma-fg">{h.name}</span>
                                  {!nestFolders && h.dirLabel && (
                                    <span className="max-w-[40%] flex-none truncate text-[11px] text-koma-fg opacity-40">
                                      {h.dirLabel}
                                    </span>
                                  )}
                                </div>
                                <div className="flex w-7 flex-none items-center justify-center">
                                  <SessionRowActions id={h.id} kind="history" armed={armed} onArm={armRow} />
                                </div>
                              </>
                            )}
                          </div>
                        )
                      })}
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        )}
        {hubReady && hasRecent && (
          <div className="border-t border-koma-border px-4 py-1.5 text-[10px] text-koma-fg opacity-35">
            Click to select · Ctrl/⌘ click toggle · Shift range · Double-click or Enter to open
          </div>
        )}
      </Card>
    </div>
  )

  const about = (
    <Card className={wide ? 'w-[300px] flex-none' : ''}>
      <SectionLabel icon={Info} className="mb-2">About koma</SectionLabel>
      <p className="text-[12.5px] leading-relaxed text-koma-fg opacity-80">
        A personal, terminal-first AI coding environment — agent + daemon at the core,
        driving your tools directly. This desktop shell renders your sessions natively
        while the daemon does the real work.
      </p>
      <div className="mt-3 space-y-2">
        <div className="flex items-start gap-2 text-[12px] text-koma-fg opacity-70">
          <Zap size={13} className="mt-0.5 flex-none text-koma-accent" />
          <span>Multiple live sessions, resumable any time.</span>
        </div>
        <div className="flex items-start gap-2 text-[12px] text-koma-fg opacity-70">
          <Sparkles size={13} className="mt-0.5 flex-none text-koma-accent" />
          <span>Bring your own provider, or run the free keyless tier.</span>
        </div>
      </div>
    </Card>
  )

  return (
    <div ref={ref} className="h-full w-full overflow-y-auto" data-tour="start-screen">
      <div className="mx-auto w-full max-w-[980px] px-6 py-10">
        <div className={`flex gap-6 ${wide ? 'flex-row items-start' : 'flex-col'}`}>
          {actions}
          {about}
        </div>
      </div>
    </div>
  )
}
