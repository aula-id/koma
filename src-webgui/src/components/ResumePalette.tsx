import { useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { motion } from 'framer-motion'
import { Search, Plus } from 'lucide-react'
import { CMD_SEARCH_SPRING, CMD_SEARCH_WIDTH } from './Titlebar'
import { NewSessionMenu } from './NewSessionMenu'
import { SessionRowActions, SessionRowConfirmStrip, type ArmedRow } from './SessionRowActions'
import { SessionBulkBar } from './SessionBulkBar'
import { useSessionMultiSelect } from './sessionListSelection'
import { useKoma, isDying } from '../store/koma'

type ResumePaletteProps = {
  onClose: () => void
}

function Label({ children }: { children: string }) {
  return (
    <div className="px-3 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-40">
      {children}
    </div>
  )
}

function Empty({ children }: { children: string }) {
  return <div className="px-3 py-1.5 text-[12px] text-koma-fg opacity-35">{children}</div>
}

// The /resume hub. Search row shares layoutId + width with the titlebar
// 'change session' pill, anchored at the same spot (no slide); the dropdown
// reveals below. New session is inline with the Cooking header. Cooking
// (live) + History (past) are driven straight off the koma store's hub
// slice, itself an authoritative mirror of the host's Hub push envelope.
//
// Session rows (issue #126): plain click selects/highlights; Ctrl/Cmd toggles;
// Shift ranges; double-click or Enter opens. Bulk Kill/Delete via SessionBulkBar.
export function ResumePalette({ onClose }: ResumePaletteProps) {
  const cooking = useKoma((s) => s.hub.cooking)
  const history = useKoma((s) => s.hub.history)
  const req = useKoma((s) => s.req)
  const startSwitching = useKoma((s) => s.startSwitching)
  const dyingSessions = useKoma((s) => s.dyingSessions)
  const [query, setQuery] = useState('')
  // The single armed row (kill/delete confirm pill) across BOTH lists — arming
  // a different row disarms whichever was armed before.
  const [armed, setArmed] = useState<ArmedRow>(null)
  const multi = useSessionMultiSelect()
  const searchRef = useRef<HTMLInputElement>(null)

  const multiHas = multi.hasSelection
  const multiClear = multi.clear
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        // Escape: multi-select → armed row → close palette.
        if (multiHas) {
          multiClear()
          // Drop keyboard focus ring on the last-clicked row (tabIndex=0
          // leaves a blue outline after Esc clears the green multi-select).
          searchRef.current?.focus()
          return
        }
        if (armed) {
          setArmed(null)
          return
        }
        onClose()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, armed, multiHas, multiClear])

  // Live-session-listing fix: the host only discovers live sessions on
  // demand. Ask for a fresh Hub the moment this overlay opens, then keep
  // nudging it on a short interval while it stays open so newly-cooked
  // sessions show up without needing to close/reopen the palette.
  useEffect(() => {
    req({ r: 'RefreshHub' })
    const interval = window.setInterval(() => {
      req({ r: 'RefreshHub' })
    }, 1500)
    return () => window.clearInterval(interval)
  }, [req])

  // Clear multi-select when the search query changes (visible order / membership
  // shifts — range anchors would otherwise point at stale rows).
  useEffect(() => {
    multiClear()
  }, [query, multiClear])

  const openSession = (id: string, name: string) => {
    // Optimistic: fires the full-screen swap overlay immediately, since the
    // host gives no "swap started" push and the attach can block for
    // seconds. Cleared by the next authoritative Snapshot (see koma.ts).
    startSwitching(name)
    req({ r: 'SelectSession', id })
    onClose()
  }

  const newSession = () => {
    // No optimistic startSwitching here: the host now opens a native folder
    // picker first (NewSession req), and only pushes switching/attaches once
    // a folder is actually confirmed. Starting the full-screen loader here
    // would strand it if the user cancels the dialog.
    req({ r: 'NewSession' })
    onClose()
  }

  // The host may include a synthetic `kind: 'new'` entry in `cooking` (see
  // bridge contract) — the header button below already covers that
  // affordance, so only render the live session rows here.
  const cookingSessions = cooking.filter((c) => c.kind === 'session')

  // Case-insensitive substring filter over name + id, applied to both lists.
  const q = query.trim().toLowerCase()
  const matches = (id: string, name: string) =>
    q === '' || name.toLowerCase().includes(q) || id.toLowerCase().includes(q)
  const filteredCooking = cookingSessions.filter((c) => matches(c.id ?? '', c.name))
  // Cap the recent list at 10 rows. Sliced AFTER the filter so search still
  // spans all history; host already sorts by lastActive, so this is the 10
  // most-recent matches.
  const filteredHistory = history.filter((h) => matches(h.id, h.name)).slice(0, 10)

  const cookingIds = useMemo(
    () => filteredCooking.map((c) => c.id).filter((id): id is string => !!id),
    [filteredCooking],
  )
  const historyIds = useMemo(() => filteredHistory.map((h) => h.id), [filteredHistory])
  const fgCooking = useMemo(
    () => filteredCooking.filter((c) => c.foreground && c.id).map((c) => c.id as string),
    [filteredCooking],
  )

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

  const bulkCooking = multi.selectedIds('session')
  const bulkHistory = multi.selectedIds('history')

  return (
    <div className="absolute inset-0 z-50" onMouseDown={onClose}>
      <div
        className={`mx-auto mt-[5px] ${CMD_SEARCH_WIDTH}`}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-xl">
          <motion.div
            layoutId="cmd-search"
            transition={CMD_SEARCH_SPRING}
            className="flex h-[22px] items-center gap-2 bg-koma-panel px-2.5"
          >
            <Search size={13} className="flex-none text-koma-fg opacity-50" />
            <input
              ref={searchRef}
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search sessions to resume…"
              className="w-full bg-transparent text-[12px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-40"
            />
          </motion.div>
          <motion.div
            initial={{ opacity: 0, y: -4 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.16, ease: 'easeOut', delay: 0.02 }}
            className="max-h-[50vh] overflow-auto border-t border-koma-border py-1"
          >
            {multi.hasSelection && (
              <SessionBulkBar
                cookingIds={bulkCooking}
                historyIds={bulkHistory}
                foregroundCookingIds={fgCooking}
                onDone={() => multi.clear()}
                onClear={() => multi.clear()}
              />
            )}
            <div className="flex items-center justify-between px-3 pb-1 pt-2">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-40">
                Cooking
              </span>
              <span className="flex items-center">
                <button
                  onClick={newSession}
                  className="flex items-center gap-1 text-[11px] text-koma-fg opacity-70 transition-colors hover:opacity-100"
                >
                  <Plus size={12} className="flex-none" />
                  New session
                </button>
                <NewSessionMenu afterPick={onClose} />
              </span>
            </div>
            {filteredCooking.length === 0 ? (
              <Empty>{q === '' ? 'No live sessions' : 'No matches'}</Empty>
            ) : (
              filteredCooking.map((c) => {
                const id = c.id as string
                const dying = !!c.id && isDying(dyingSessions, id, 'session')
                const rowArmed = !!c.id && armed?.id === id && armed.kind === 'session'
                const sel = !!c.id && multi.isSelected('session', id)
                return (
                  <div
                    key={c.id}
                    role="button"
                    tabIndex={dying || rowArmed ? -1 : 0}
                    aria-selected={sel}
                    onClick={(e) => {
                      if (dying || !c.id) return
                      if (rowArmed) return
                      onRowMouse(e, 'session', id, cookingIds)
                    }}
                    onDoubleClick={(e) => {
                      if (dying || rowArmed || !c.id) return
                      e.preventDefault()
                      openSession(id, c.name)
                    }}
                    onKeyDown={(e) => {
                      if (e.key !== 'Enter' && e.key !== ' ') return
                      if (e.key === ' ') e.preventDefault()
                      if (!dying && !armed && c.id) openSession(id, c.name)
                    }}
                    className={`group flex w-full cursor-pointer items-center justify-between text-left text-[12px] text-koma-fg transition-colors outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-koma-accent/50 ${
                      rowArmed ? '' : 'px-3 py-1.5'
                    } ${dying ? 'pointer-events-none opacity-60' : ''} ${
                      rowArmed
                        ? ''
                        : sel
                          ? 'bg-koma-accent/15 hover:bg-koma-accent/20'
                          : 'hover:bg-koma-hover'
                    }`}
                  >
                    {rowArmed && c.id ? (
                      <SessionRowConfirmStrip
                        id={id}
                        kind="session"
                        foreground={c.foreground}
                        onCancel={() => setArmed(null)}
                        className="px-3 py-1.5"
                      />
                    ) : (
                      <>
                        <div className="flex min-w-0 flex-1 items-center gap-1.5">
                          {c.working && (
                            <span className="h-1.5 w-1.5 flex-none animate-pulse rounded-full bg-emerald-500" />
                          )}
                          <span className="min-w-0 flex-1 truncate">{c.name}</span>
                          {c.foreground && (
                            <span className="flex-none rounded border border-koma-border px-1 text-[9px] uppercase tracking-wide opacity-50">
                              current
                            </span>
                          )}
                          {c.dirLabel && (
                            <span className="max-w-[40%] flex-none truncate text-[11px] opacity-40">
                              {c.dirLabel}
                            </span>
                          )}
                        </div>
                        <div className="flex w-7 flex-none items-center justify-center">
                          {c.id && (
                            <SessionRowActions id={id} kind="session" armed={armed} onArm={armRow} />
                          )}
                        </div>
                      </>
                    )}
                  </div>
                )
              })
            )}
            <Label>History</Label>
            {filteredHistory.length === 0 ? (
              <Empty>{q === '' ? 'No past sessions' : 'No matches'}</Empty>
            ) : (
              filteredHistory.map((h) => {
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
                    className={`group flex w-full cursor-pointer items-center justify-between text-left text-[12px] text-koma-fg transition-colors outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-koma-accent/50 ${
                      rowArmed ? '' : 'px-3 py-1.5'
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
                        className="px-3 py-1.5"
                      />
                    ) : (
                      <>
                        <div className="flex min-w-0 flex-1 items-center gap-1.5">
                          <span className="min-w-0 flex-1 truncate">{h.name}</span>
                          {h.dirLabel && (
                            <span className="max-w-[40%] flex-none truncate text-[11px] opacity-40">
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
              })
            )}
            <div className="px-3 pb-1 pt-0.5 text-[10px] text-koma-fg opacity-30">
              Click select · Ctrl/⌘ toggle · Shift range · Double-click / Enter open
            </div>
          </motion.div>
        </div>
      </div>
    </div>
  )
}
