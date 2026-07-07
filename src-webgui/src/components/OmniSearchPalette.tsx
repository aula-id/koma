import { useEffect, useState } from 'react'
import { motion } from 'framer-motion'
import { File as FileIcon, Search } from 'lucide-react'
import { CMD_SEARCH_SPRING, CMD_SEARCH_WIDTH } from './Titlebar'
import { useKoma } from '../store/koma'
import { Empty } from './panels/helpers'

type OmniSearchPaletteProps = {
  onClose: () => void
}

// Fuzzy workspace-file search + insert overlay. Mirrors ResumePalette's
// overlay skeleton (full-screen backdrop with click-to-close, shared
// search-pill layoutId, Esc-to-close) but drives a live FileSearch request as
// the user types (debounced) and inserts the selected result's path into the
// composer draft instead of switching sessions. Picks are NOT routed through
// AttachPath: the daemon's attachment ingest is image-only, so a non-image
// path attached that way silently corrupts the shared composer buffer.
export function OmniSearchPalette({ onClose }: OmniSearchPaletteProps) {
  const results = useKoma((s) => s.session.searchResults)
  const req = useKoma((s) => s.req)
  const insertToComposer = useKoma((s) => s.insertToComposer)
  const [query, setQuery] = useState('')

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  // Live-refresh, debounced: fire a fresh FileSearch as the query changes.
  useEffect(() => {
    const t = window.setTimeout(() => {
      req({ r: 'FileSearch', query })
    }, 150)
    return () => window.clearTimeout(t)
  }, [query, req])

  const pick = (path: string) => {
    // Dir rows (non-attachable) come back with path === "" — guarded out at
    // the render call site below, but double-guard here too.
    if (!path) return
    insertToComposer(path)
    onClose()
  }

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
              placeholder="Search workspace files to attach…"
              className="w-full bg-transparent text-[12px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-40"
            />
          </motion.div>
          <motion.div
            initial={{ opacity: 0, y: -4 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.16, ease: 'easeOut', delay: 0.02 }}
            className="max-h-[50vh] overflow-auto border-t border-koma-border py-1"
          >
            {results.length === 0 ? (
              <Empty>No matches</Empty>
            ) : (
              results.map((r, i) => {
                // Directory rows (non-attachable) come back with path === ""
                // from the daemon — render them disabled and key by label+
                // index instead of path (multiple dir rows would otherwise
                // collide on the empty-string key).
                const disabled = r.path === ''
                return (
                  <button
                    key={r.path || `${r.label}-${i}`}
                    onClick={() => pick(r.path)}
                    disabled={disabled}
                    aria-disabled={disabled}
                    className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-koma-fg transition-colors ${
                      disabled ? 'cursor-default opacity-40' : 'hover:bg-koma-hover'
                    }`}
                  >
                    <FileIcon size={12} className="flex-none opacity-50" />
                    <span className="truncate">{r.label}</span>
                  </button>
                )
              })
            )}
          </motion.div>
        </div>
      </div>
    </div>
  )
}
