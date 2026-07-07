import { useEffect } from 'react'
import { motion } from 'framer-motion'
import { Search, Plus } from 'lucide-react'
import { CMD_SEARCH_SPRING, CMD_SEARCH_WIDTH } from './Titlebar'

type ResumePaletteProps = {
  onClose: () => void
  onNewSession: () => void
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

// Design-phase stub of the /resume hub. The search row shares a layoutId AND
// width with the titlebar pill and is anchored at the same top spot (mt-[5px],
// h-[22px]), so opening swaps the pill's content in place — no downward slide,
// no widening — and just reveals the dropdown below. Cooking (live) + History
// (past) mirror the real hub; both empty. Inert search/rows. No backdrop dim.
export function ResumePalette({ onClose, onNewSession }: ResumePaletteProps) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

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
            <Label>Cooking</Label>
            <button
              onClick={onNewSession}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[13px] text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100"
            >
              <Plus size={14} className="flex-none" />
              New session
            </button>
            <Empty>No live sessions</Empty>
            <Label>History</Label>
            <Empty>No past sessions</Empty>
          </motion.div>
        </div>
      </div>
    </div>
  )
}
