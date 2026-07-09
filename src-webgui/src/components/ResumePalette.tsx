import { useEffect, useState } from 'react'
import { motion } from 'framer-motion'
import { Search, Plus } from 'lucide-react'
import { CMD_SEARCH_SPRING, CMD_SEARCH_WIDTH } from './Titlebar'
import { NewSessionMenu } from './NewSessionMenu'
import { SessionRowActions, type ArmedRow } from './SessionRowActions'
import { useKoma } from '../store/koma'

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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        // Escape cancels an armed row first; only closes the palette once
        // nothing is armed.
        if (armed) {
          setArmed(null)
          return
        }
        onClose()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, armed])

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

  const selectSession = (id: string, name: string) => {
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
                const dying = !!c.id && dyingSessions.includes(c.id)
                return (
                  <div
                    key={c.id}
                    role="button"
                    tabIndex={dying ? -1 : 0}
                    onClick={() => {
                      if (dying) return
                      if (armed && armed.id === c.id && armed.kind === 'session') return
                      c.id && selectSession(c.id, c.name)
                    }}
                    onKeyDown={(e) => {
                      if (e.key !== 'Enter' && e.key !== ' ') return
                      if (e.key === ' ') e.preventDefault()
                      if (!dying && !armed && c.id) selectSession(c.id, c.name)
                    }}
                    className={`group flex w-full cursor-pointer items-center justify-between px-3 py-1.5 text-left text-[12px] text-koma-fg transition-colors hover:bg-koma-hover ${
                      dying ? 'pointer-events-none opacity-60' : ''
                    }`}
                  >
                    <span className="flex min-w-0 items-center gap-1.5">
                      {c.working && (
                        <span className="h-1.5 w-1.5 flex-none animate-pulse rounded-full bg-emerald-500" />
                      )}
                      <span className="truncate">{c.name}</span>
                      {c.foreground && (
                        <span className="flex-none rounded border border-koma-border px-1 text-[9px] uppercase tracking-wide opacity-50">
                          current
                        </span>
                      )}
                    </span>
                    <span className="ml-2 flex flex-none items-center gap-2">
                      {c.dirLabel && (
                        <span className="truncate text-[11px] opacity-40">{c.dirLabel}</span>
                      )}
                      {c.id && (
                        <SessionRowActions
                          id={c.id}
                          kind="session"
                          foreground={c.foreground}
                          armed={armed}
                          onArm={setArmed}
                        />
                      )}
                    </span>
                  </div>
                )
              })
            )}
            <Label>History</Label>
            {filteredHistory.length === 0 ? (
              <Empty>{q === '' ? 'No past sessions' : 'No matches'}</Empty>
            ) : (
              filteredHistory.map((h) => {
                const dying = dyingSessions.includes(h.id)
                return (
                  <div
                    key={h.id}
                    role="button"
                    tabIndex={dying ? -1 : 0}
                    onClick={() => {
                      if (dying) return
                      if (armed && armed.id === h.id && armed.kind === 'history') return
                      selectSession(h.id, h.name)
                    }}
                    onKeyDown={(e) => {
                      if (e.key !== 'Enter' && e.key !== ' ') return
                      if (e.key === ' ') e.preventDefault()
                      if (!dying && !armed) selectSession(h.id, h.name)
                    }}
                    className={`group flex w-full cursor-pointer items-center justify-between px-3 py-1.5 text-left text-[12px] text-koma-fg transition-colors hover:bg-koma-hover ${
                      dying ? 'pointer-events-none opacity-60' : ''
                    }`}
                  >
                    <span className="truncate">{h.name}</span>
                    <span className="ml-2 flex flex-none items-center gap-2">
                      {h.dirLabel && <span className="truncate text-[11px] opacity-40">{h.dirLabel}</span>}
                      <SessionRowActions id={h.id} kind="history" armed={armed} onArm={setArmed} />
                    </span>
                  </div>
                )
              })
            )}
          </motion.div>
        </div>
      </div>
    </div>
  )
}
