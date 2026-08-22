import { useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { ArrowRight, Clock, FolderPlus, Info, Server, Sparkles, Zap } from 'lucide-react'
import { NewSessionMenu } from './NewSessionMenu'
import { SessionRowActions, SessionRowConfirmStrip, type ArmedRow } from './SessionRowActions'
import { SessionBulkBar } from './SessionBulkBar'
import { useSessionMultiSelect } from './sessionListSelection'
import { useKoma, isDying } from '../store/koma'

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

function SectionLabel({ icon: Icon, children }: { icon: typeof Clock; children: string }) {
  return (
    <div className="mb-2 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-45">
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
  const req = useKoma((s) => s.req)
  const startSwitching = useKoma((s) => s.startSwitching)
  const dyingSessions = useKoma((s) => s.dyingSessions)
  const remoteState = useKoma((s) => s.remoteState)
  const [ref, width] = useContainerWidth<HTMLDivElement>()
  const wide = width >= 760
  // The single armed row (kill/delete confirm pill) across BOTH lists — arming
  // a different row disarms whichever was armed before.
  const [armed, setArmed] = useState<ArmedRow>(null)
  const multi = useSessionMultiSelect()

  // The host only discovers live sessions on demand — nudge a fresh Hub on
  // mount and on a short interval so recent/live rows stay current (same cadence
  // as ResumePalette).
  useEffect(() => {
    req({ r: 'RefreshHub' })
    const id = window.setInterval(() => req({ r: 'RefreshHub' }), 2000)
    return () => window.clearInterval(id)
  }, [req])

  // Escape: clear multi-select first, then cancel an armed row.
  const multiHas = multi.hasSelection
  const multiClear = multi.clear
  useEffect(() => {
    if (!armed && !multiHas) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
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
  }, [armed, multiHas, multiClear])

  // Live (cooking) sessions first, then past history — the same source rows the
  // ResumePalette lists, minus the synthetic `kind: 'new'` placeholder.
  const liveSessions = useMemo(
    () => cooking.filter((c) => c.kind === 'session' && c.id),
    [cooking],
  )
  const liveIds = useMemo(() => liveSessions.map((c) => c.id as string), [liveSessions])
  const historyIds = useMemo(() => history.map((h) => h.id), [history])

  const openSession = (id: string, name: string) => {
    // Optimistic swap overlay (no host "swap started" push; attach can block for
    // seconds) — cleared by the next authoritative Snapshot. Mirrors ResumePalette.
    startSwitching(name)
    req({ r: 'SelectSession', id })
  }
  // No optimistic loader: the host opens a folder picker first and only
  // attaches once a folder is confirmed (cancel would strand the loader).
  // Remote hub uses the SSH path picker instead of the local native dialog.
  const newSession = () => {
    if (remoteState.state === 'ready' || remoteState.state === 'connected') {
      req({ r: 'RequestRemotePath' })
      return
    }
    req({ r: 'NewSession' })
  }

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

      <div className="group flex items-center rounded-xl border border-koma-border bg-koma-panel transition-colors hover:border-koma-accent/60 hover:bg-koma-hover">
        <button
          onClick={newSession}
          className="flex min-w-0 flex-1 items-center gap-3 px-4 py-3 text-left"
        >
          <span className="flex h-9 w-9 flex-none items-center justify-center rounded-lg bg-koma-accent/15 text-koma-accent">
            <FolderPlus size={18} />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-[13px] font-semibold text-koma-fg">New session</span>
            <span className="block text-[11px] text-koma-fg opacity-45">
              {remoteTarget
                ? `Pick a folder on ${remoteTarget}`
                : 'Start working in your default directory'}
            </span>
          </span>
          <ArrowRight size={16} className="flex-none text-koma-fg opacity-30 transition group-hover:translate-x-0.5 group-hover:opacity-70" />
        </button>
        <NewSessionMenu className="pr-3" />
      </div>

      <Card className="!p-0 overflow-hidden">
        <div className="px-4 pt-4">
          {multi.hasSelection ? (
            <SessionBulkBar
              cookingIds={bulkCooking}
              historyIds={bulkHistory}
              foregroundCookingIds={fgCooking}
              onDone={() => multi.clear()}
              onClear={() => multi.clear()}
              className="mb-2"
            />
          ) : (
            <SectionLabel icon={Clock}>Recent</SectionLabel>
          )}
        </div>
        {!hasRecent ? (
          <div className="px-5 pb-4 pt-2 text-[12px] text-koma-fg opacity-35">No sessions yet — start a new one.</div>
        ) : (
          <div className="max-h-[40vh] overflow-y-auto px-3 pb-3">
            {liveSessions.map((c) => {
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
                    if (dying) return
                    if (rowArmed) return
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
                    rowArmed
                      ? ''
                      : sel
                        ? 'bg-koma-accent/15 hover:bg-koma-accent/20'
                        : 'hover:bg-koma-hover'
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
                        {c.dirLabel && (
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
            {history.map((h) => {
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
                    if (dying) return
                    if (rowArmed) return
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
                    rowArmed
                      ? ''
                      : sel
                        ? 'bg-koma-accent/15 hover:bg-koma-accent/20'
                        : 'hover:bg-koma-hover'
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
                        {h.dirLabel && (
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
        {hasRecent && (
          <div className="border-t border-koma-border px-4 py-1.5 text-[10px] text-koma-fg opacity-35">
            Click to select · Ctrl/⌘ click toggle · Shift range · Double-click or Enter to open
          </div>
        )}
      </Card>
    </div>
  )

  const about = (
    <Card className={wide ? 'w-[300px] flex-none' : ''}>
      <SectionLabel icon={Info}>About koma</SectionLabel>
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
