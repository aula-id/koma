import { useEffect, useState } from 'react'
import { motion, useAnimationControls } from 'framer-motion'
import { Check, X } from 'lucide-react'
import { CMD_SEARCH_SPRING, CMD_SEARCH_WIDTH } from './Titlebar'
import { useKoma } from '../store/koma'

type RenameOverlayProps = {
  onClose: () => void
}

// Reuses the search-bar location: the titlebar 'rename' button morphs (shared
// layoutId) into this input, prefilled with the current session title.
// Confirm (check) sends RenameSession + closes; cancel (X) / Esc discard.
// Clicking OUTSIDE does not close — instead the bar wiggles so the user knows
// to confirm/cancel first.
export function RenameOverlay({ onClose }: RenameOverlayProps) {
  const controls = useAnimationControls()
  const title = useKoma((s) => s.session.title)
  const req = useKoma((s) => s.req)
  const [name, setName] = useState(title)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const confirm = () => {
    const trimmed = name.trim()
    if (trimmed) req({ r: 'Rename', name: trimmed })
    onClose()
  }

  const wiggle = () => {
    controls.start({
      x: [0, -6, 6, -5, 5, -3, 3, 0],
      transition: { duration: 0.35, ease: 'easeInOut' },
    })
  }

  return (
    <div className="absolute inset-0 z-50" onMouseDown={wiggle}>
      <motion.div
        animate={controls}
        className={`mx-auto mt-[5px] ${CMD_SEARCH_WIDTH}`}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <motion.div
          layoutId="cmd-rename"
          transition={CMD_SEARCH_SPRING}
          className="flex h-[22px] items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-2.5 shadow-xl"
        >
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') confirm()
            }}
            placeholder="Rename session…"
            className="w-full bg-transparent text-[12px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-40"
          />
          <button
            onClick={confirm}
            title="Confirm"
            aria-label="Confirm rename"
            className="flex h-4 w-4 flex-none items-center justify-center rounded text-koma-fg opacity-60 transition-colors hover:text-emerald-500 hover:opacity-100"
          >
            <Check size={13} />
          </button>
          <button
            onClick={onClose}
            title="Cancel"
            aria-label="Cancel rename"
            className="flex h-4 w-4 flex-none items-center justify-center rounded text-koma-fg opacity-60 transition-colors hover:text-red-500 hover:opacity-100"
          >
            <X size={13} />
          </button>
        </motion.div>
      </motion.div>
    </div>
  )
}
