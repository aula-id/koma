import { useEffect } from 'react'
import { motion } from 'framer-motion'
import { Search, Plus } from 'lucide-react'
import { CMD_SEARCH_SPRING, CMD_SEARCH_WIDTH } from './Titlebar'
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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const selectSession = (id: string) => {
    req({ r: 'SelectSession', id })
    onClose()
  }

  const newSession = () => {
    req({ r: 'NewSession' })
    onClose()
  }

  // The host may include a synthetic `kind: 'new'` entry in `cooking` (see
  // bridge contract) — the header button below already covers that
  // affordance, so only render the live session rows here.
  const cookingSessions = cooking.filter((c) => c.kind === 'session')

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
              <button
                onClick={newSession}
                className="flex items-center gap-1 text-[11px] text-koma-fg opacity-70 transition-colors hover:opacity-100"
              >
                <Plus size={12} className="flex-none" />
                New session
              </button>
            </div>
            {cookingSessions.length === 0 ? (
              <Empty>No live sessions</Empty>
            ) : (
              cookingSessions.map((c) => (
                <button
                  key={c.id}
                  onClick={() => c.id && selectSession(c.id)}
                  className="flex w-full items-center justify-between px-3 py-1.5 text-left text-[12px] text-koma-fg transition-colors hover:bg-koma-hover"
                >
                  <span className="truncate">{c.name}</span>
                  {c.dirLabel && (
                    <span className="ml-2 flex-none truncate text-[11px] opacity-40">{c.dirLabel}</span>
                  )}
                </button>
              ))
            )}
            <Label>History</Label>
            {history.length === 0 ? (
              <Empty>No past sessions</Empty>
            ) : (
              history.map((h) => (
                <button
                  key={h.id}
                  onClick={() => selectSession(h.id)}
                  className="flex w-full items-center justify-between px-3 py-1.5 text-left text-[12px] text-koma-fg transition-colors hover:bg-koma-hover"
                >
                  <span className="truncate">{h.name}</span>
                  {h.dirLabel && (
                    <span className="ml-2 flex-none truncate text-[11px] opacity-40">{h.dirLabel}</span>
                  )}
                </button>
              ))
            )}
          </motion.div>
        </div>
      </div>
    </div>
  )
}
