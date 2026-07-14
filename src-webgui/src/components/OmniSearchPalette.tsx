import { useEffect, useState } from 'react'
import { motion } from 'framer-motion'
import { Check, ChevronRight, File as FileIcon, Folder as FolderIcon, FolderPlus, Search } from 'lucide-react'
import { CMD_SEARCH_SPRING, CMD_SEARCH_WIDTH } from './Titlebar'
import { useKoma } from '../store/koma'
import { Empty } from './panels/helpers'

type OmniSearchPaletteProps = {
  onClose: () => void
}

// Fuzzy workspace-file search + insert overlay. Mirrors ResumePalette's
// overlay skeleton (full-screen backdrop with click-to-close, shared
// search-pill layoutId, Esc-to-close) but drives a live FileSearch request as
// the user types (debounced) and inserts a `@<label>` TOKEN into the composer
// draft instead of switching sessions. `label` is the dircache entry text
// (`[N]relpath` for multi-root workspaces, bare `relpath` for single-root) —
// the SAME token shape the TUI composer inserts, so the text that ends up on
// the wire is byte-identical between GUI and TUI (the daemon leaves non-image
// `@` tokens untouched in the submitted text; only images get scanned into
// attachments). Falls back to the raw absolute path (no `@`, old behavior) if
// a row ever comes back with an empty label. Picks are NOT routed through
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

  // FILE pick: insert the `@label` token (TUI-parity wire text) into the
  // composer draft and close (attach flow). Falls back to the raw path if
  // this row has no label (shouldn't normally happen for a file row).
  const pick = (path: string, label: string) => {
    if (!path) return
    insertToComposer(label ? `@${label} ` : path)
    onClose()
  }

  // FOLDER drill-in: re-drive the search with the folder's path so its contents
  // list (FileSearch lists a dir's children when the query is that dir path).
  // Dir rows come back with path === "" (only `label` carries the dir path), so
  // drill on the label. Keeps the overlay open — no attach, no close.
  const drill = (dirPath: string) => {
    if (!dirPath) return
    setQuery(dirPath)
  }

  // FOLDER attach: insert the folder as a `@label` token into the composer
  // draft and close — the same insert flow as a file pick (ui.composerInsert),
  // so the model can read the directory via its own tools. `dirPath` here is
  // always a dircache label already (either `r.label` from a row, or the
  // query box's text after a drill-in set it to a label — see drill above),
  // never the empty-string sentinel, so no fallback is needed. Used by the
  // per-row check button and the confirm-current-folder button beside the
  // search input.
  const attachFolder = (dirPath: string) => {
    if (!dirPath) return
    insertToComposer(`@${dirPath} `)
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
            {/* Attach the CURRENT folder (whatever path is in the query box after
                drilling in). Commit via onMouseDown+preventDefault so the input
                blur/close never races the click. */}
            {query.trim() !== '' && (
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault()
                  attachFolder(query.trim())
                }}
                aria-label="Attach current folder"
                title="Attach current folder"
                className="flex flex-none items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-koma-fg opacity-60 transition-colors hover:bg-koma-hover hover:opacity-100"
              >
                <FolderPlus size={13} className="flex-none" />
              </button>
            )}
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
                // Directory rows come back with path === "" from the daemon
                // (only `label` carries the dir path). They are NO LONGER dead:
                // clicking a folder DRILLS IN — re-drives the search with the
                // folder path so its contents list. File rows still attach.
                // Key by label+index for dirs (they'd collide on the empty
                // path key).
                const isDir = r.path === ''
                return (
                  <div
                    key={r.path || `${r.label}-${i}`}
                    className="flex w-full items-center gap-1 px-3 py-1.5 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover"
                  >
                    {/* Primary action: file rows ATTACH (pick), folder rows
                        DRILL IN. */}
                    <button
                      onClick={() => (isDir ? drill(r.label) : pick(r.path, r.label))}
                      className="flex min-w-0 flex-1 items-center gap-2 text-left"
                    >
                      {isDir ? (
                        <FolderIcon size={12} className="flex-none text-koma-accent opacity-70" />
                      ) : (
                        <FileIcon size={12} className="flex-none opacity-50" />
                      )}
                      <span className="min-w-0 flex-1 truncate">{r.label}</span>
                    </button>
                    {isDir && (
                      <>
                        {/* Attach THIS folder's path (distinct from drilling in). */}
                        <button
                          onClick={() => attachFolder(r.label)}
                          aria-label="Attach folder"
                          title="Attach folder"
                          className="flex flex-none items-center rounded p-0.5 text-koma-fg opacity-50 transition-colors hover:bg-koma-panel2 hover:text-koma-accent hover:opacity-100"
                        >
                          <Check size={12} className="flex-none" />
                        </button>
                        <ChevronRight size={12} className="flex-none opacity-40" />
                      </>
                    )}
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
