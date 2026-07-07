import { useEffect } from 'react'
import { motion } from 'framer-motion'
import { FolderOpen } from 'lucide-react'

type NewSessionDialogProps = {
  onClose: () => void
}

// Design-phase stub. Represents the "pick a working folder" step for a new
// session. Browse will later trigger the host's native folder picker via ipc;
// for now every control is inert.
export function NewSessionDialog({ onClose }: NewSessionDialogProps) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  return (
    <motion.div
      className="absolute inset-0 z-50 flex items-start justify-center"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.15 }}
      onMouseDown={onClose}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.97, y: -4 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ duration: 0.16, ease: 'easeOut' }}
        className="mt-24 w-[min(92vw,480px)] rounded-lg border border-koma-border bg-koma-panel p-4 shadow-2xl"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="mb-3 text-[13px] font-semibold text-koma-fg">New session</div>
        <div className="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-50">
          Working folder
        </div>
        <div className="flex items-center gap-2">
          <div className="min-w-0 flex-1 truncate rounded border border-koma-border bg-koma-bg px-2.5 py-1.5 text-[12px] text-koma-fg opacity-40">
            Select a folder…
          </div>
          <button className="flex flex-none items-center gap-1.5 rounded border border-koma-border px-2.5 py-1.5 text-[12px] text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100">
            <FolderOpen size={14} />
            Browse
          </button>
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <button
            onClick={onClose}
            className="rounded px-3 py-1.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
          >
            Cancel
          </button>
          <button
            disabled
            className="rounded border border-koma-border px-3 py-1.5 text-[12px] text-koma-fg opacity-40"
          >
            Create
          </button>
        </div>
      </motion.div>
    </motion.div>
  )
}
