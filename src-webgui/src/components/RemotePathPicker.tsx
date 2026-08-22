import { useEffect, useMemo, useRef, useState } from 'react'
import { motion } from 'framer-motion'
import { Check, ChevronRight, Folder, Search, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'
import { CMD_SEARCH_SPRING, CMD_SEARCH_WIDTH } from './Titlebar'

type FocusTarget = 'search' | 'select'

const IDLE_PATH = {
  state: 'idle' as const,
  path: '',
  dirs: [] as string[],
  error: null as string | null,
}

/**
 * Remote folder picker overlay. Driven by host `RemotePathPicker` pushes
 * (listing / ready / error / cancelled). Confirm mints a new remote session
 * with that cwd; Esc/cancel aborts without attaching.
 *
 * Design signature matches ResumePalette / RenameOverlay:
 *   CMD_SEARCH_WIDTH · h-[22px] search row · layoutId spring · body fade 0.16
 *
 * Keyboard:
 *   - Tab toggles path search ↔ physical "Select folder" button
 *   - Enter on search: list typed path, or enter highlighted subdir
 *   - Enter on "Select folder": confirm current path as cwd
 *   - ↑/↓ highlight dirs · Esc cancels
 *   - Ctrl/Cmd+Enter confirms (never opens the global context menu)
 */
export function RemotePathPicker() {
  const remotePath = useKoma((s) => s.remotePath)
  const req = useKoma((s) => s.req)
  const startSwitching = useKoma((s) => s.startSwitching)
  const active =
    remotePath.state === 'listing' ||
    remotePath.state === 'ready' ||
    remotePath.state === 'error'

  const [query, setQuery] = useState('')
  const [focusTarget, setFocusTarget] = useState<FocusTarget>('search')
  const [highlight, setHighlight] = useState(0)
  const searchRef = useRef<HTMLInputElement>(null)
  const selectRef = useRef<HTMLButtonElement>(null)
  // Track the path we last synced the query from so a listing refresh doesn't
  // stomp what the user is mid-typing.
  const syncedPathRef = useRef<string | null>(null)

  useEffect(() => {
    if (!active) {
      setQuery('')
      setFocusTarget('search')
      setHighlight(0)
      syncedPathRef.current = null
      return
    }
    setFocusTarget('search')
    setHighlight(0)
  }, [active])

  useEffect(() => {
    if (!active) return
    if (syncedPathRef.current === remotePath.path) return
    syncedPathRef.current = remotePath.path
    setQuery(remotePath.path || '')
    setHighlight(0)
  }, [active, remotePath.path])

  useEffect(() => {
    if (!active) return
    const id = window.requestAnimationFrame(() => {
      if (focusTarget === 'search') searchRef.current?.focus()
      else selectRef.current?.focus()
    })
    return () => window.cancelAnimationFrame(id)
  }, [active, focusTarget, remotePath.state])

  const parentPath = useMemo(() => {
    const p = remotePath.path.replace(/\/+$/, '')
    if (!p || p === '/') return null
    const i = p.lastIndexOf('/')
    if (i <= 0) return '/'
    return p.slice(0, i) || '/'
  }, [remotePath.path])

  const filteredDirs = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q || q === (remotePath.path || '').toLowerCase()) return remotePath.dirs
    const base = remotePath.path.replace(/\/+$/, '')
    const segment = q.startsWith(base.toLowerCase())
      ? q.slice(base.length).replace(/^\//, '')
      : q
    if (!segment) return remotePath.dirs
    return remotePath.dirs.filter((dir) => {
      const name = dir.replace(/\/+$/, '').split('/').pop() || dir
      return name.toLowerCase().includes(segment)
    })
  }, [query, remotePath.dirs, remotePath.path])

  useEffect(() => {
    setHighlight((h) => {
      if (filteredDirs.length === 0) return 0
      return Math.min(h, filteredDirs.length - 1)
    })
  }, [filteredDirs.length])

  // Escape + block contextmenu while open (Ctrl+Enter / right-click used to
  // pop GlobalContextMenu over the picker).
  useEffect(() => {
    if (!active) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        req({ r: 'CancelRemotePath' })
        useKoma.setState({ remotePath: { ...IDLE_PATH } })
      }
    }
    const onCtx = (e: Event) => {
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener('keydown', onKey)
    document.addEventListener('contextmenu', onCtx, true)
    return () => {
      window.removeEventListener('keydown', onKey)
      document.removeEventListener('contextmenu', onCtx, true)
    }
  }, [active, req])

  if (!active) return null

  const cancel = () => {
    req({ r: 'CancelRemotePath' })
    // Optimistic dismiss so Esc never leaves a stuck overlay if the host lags.
    useKoma.setState({ remotePath: { ...IDLE_PATH } })
  }
  const confirm = () => {
    if (!remotePath.path || remotePath.state === 'listing') return
    const path = remotePath.path
    req({ r: 'ConfirmRemotePath', path })
    // Optimistic close + switcher. Host also pushes cancelled + Switching;
    // without this local clear the picker (z-70) freezes over the UI until
    // that push lands — and used to never land on the confirm path.
    const leaf = path.replace(/\/+$/, '').split('/').pop() || path
    startSwitching(leaf)
    useKoma.setState({ remotePath: { ...IDLE_PATH } })
  }
  const enter = (dir: string) => req({ r: 'ListRemotePath', path: dir })

  const canConfirm = !!remotePath.path && remotePath.state !== 'listing'

  const onSearchKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Tab') {
      e.preventDefault()
      setFocusTarget('select')
      return
    }
    if (e.key === 'Escape') {
      e.preventDefault()
      cancel()
      return
    }
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault()
      e.stopPropagation()
      if (canConfirm) confirm()
      return
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      if (filteredDirs.length === 0) return
      setHighlight((h) => Math.min(h + 1, filteredDirs.length - 1))
      return
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault()
      if (filteredDirs.length === 0) return
      setHighlight((h) => Math.max(h - 1, 0))
      return
    }
    if (e.key === 'Enter') {
      e.preventDefault()
      const typed = query.trim()
      if (typed && typed !== remotePath.path) {
        enter(typed)
        return
      }
      if (filteredDirs.length > 0 && filteredDirs[highlight]) {
        enter(filteredDirs[highlight])
        return
      }
      if (canConfirm) setFocusTarget('select')
    }
  }

  const onSelectKeyDown = (e: React.KeyboardEvent<HTMLButtonElement>) => {
    if (e.key === 'Tab') {
      e.preventDefault()
      setFocusTarget('search')
      return
    }
    if (e.key === 'Escape') {
      e.preventDefault()
      cancel()
      return
    }
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault()
      e.stopPropagation()
      if (canConfirm) confirm()
    }
  }

  return (
    <div
      className="absolute inset-0 z-50"
      onMouseDown={cancel}
      onContextMenu={(e) => {
        e.preventDefault()
        e.stopPropagation()
      }}
    >
      <div
        className={`mx-auto mt-[5px] ${CMD_SEARCH_WIDTH}`}
        onMouseDown={(e) => e.stopPropagation()}
        role="dialog"
        aria-label="Select remote working directory"
      >
        <div className="overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-xl">
          {/* Header — same h-[22px] / spring morph as ResumePalette search row */}
          <motion.div
            layoutId="cmd-search"
            transition={CMD_SEARCH_SPRING}
            className="flex h-[22px] items-center gap-2 bg-koma-panel px-2.5"
          >
            <Search size={13} className="flex-none text-koma-fg opacity-50" />
            <input
              ref={searchRef}
              type="text"
              value={query}
              onChange={(e) => {
                setQuery(e.target.value)
                setHighlight(0)
                setFocusTarget('search')
              }}
              onFocus={() => setFocusTarget('search')}
              onKeyDown={onSearchKeyDown}
              placeholder="Remote path…"
              spellCheck={false}
              autoComplete="off"
              aria-label="Remote path"
              className="w-full bg-transparent text-[12px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-40"
            />
            {remotePath.state === 'listing' && <BrailleSpinner size={12} />}
            <button
              type="button"
              aria-label="Cancel"
              tabIndex={-1}
              onClick={cancel}
              className="flex h-4 w-4 flex-none items-center justify-center rounded text-koma-fg opacity-50 hover:opacity-100"
            >
              <X size={12} />
            </button>
          </motion.div>

          {/* Body — same fade-in as ResumePalette dropdown */}
          <motion.div
            initial={{ opacity: 0, y: -4 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.16, ease: 'easeOut', delay: 0.02 }}
            className="border-t border-koma-border"
          >
            {remotePath.error && (
              <div className="border-b border-koma-border px-2.5 py-1.5 text-[11px] text-koma-fg opacity-50">
                {remotePath.error}
              </div>
            )}
            <div className="max-h-[50vh] overflow-auto py-1">
              {parentPath && (
                <button
                  type="button"
                  tabIndex={-1}
                  onClick={() => enter(parentPath)}
                  className="flex w-full items-center gap-1.5 px-2.5 py-1 text-left text-[12px] text-koma-fg opacity-50 hover:bg-koma-hover hover:opacity-100"
                >
                  <ChevronRight size={11} className="rotate-180" />
                  ..
                </button>
              )}
              {filteredDirs.length === 0 && remotePath.state === 'ready' && (
                <div className="px-2.5 py-2 text-[11px] text-koma-fg opacity-35">
                  {remotePath.dirs.length === 0 ? 'No subfolders' : 'No matches'}
                </div>
              )}
              {filteredDirs.map((dir, i) => {
                const name = dir.replace(/\/+$/, '').split('/').pop() || dir
                const activeRow = i === highlight && focusTarget === 'search'
                return (
                  <button
                    key={dir}
                    type="button"
                    tabIndex={-1}
                    onClick={() => enter(dir)}
                    onMouseEnter={() => setHighlight(i)}
                    className={`flex w-full items-center gap-1.5 px-2.5 py-1 text-left text-[12px] ${
                      activeRow
                        ? 'bg-koma-hover text-koma-fg'
                        : 'text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100'
                    }`}
                  >
                    <Folder size={11} className="flex-none opacity-50" />
                    <span className="min-w-0 truncate">{name}</span>
                  </button>
                )
              })}
            </div>

            <div className="flex items-center justify-between gap-1.5 border-t border-koma-border px-2 py-1.5">
              <span className="min-w-0 truncate px-1 text-[10px] text-koma-fg opacity-30">
                Tab switches · Enter opens · Select folder confirms
              </span>
              <div className="flex flex-none items-center gap-1">
                <button
                  type="button"
                  tabIndex={-1}
                  onClick={cancel}
                  className="rounded px-2 py-0.5 text-[11px] text-koma-fg opacity-50 hover:opacity-100"
                >
                  cancel
                </button>
                <button
                  ref={selectRef}
                  type="button"
                  onClick={confirm}
                  onFocus={() => setFocusTarget('select')}
                  onKeyDown={onSelectKeyDown}
                  disabled={!canConfirm}
                  aria-label="Select folder"
                  className={`flex items-center gap-1 rounded px-2 py-0.5 text-[11px] font-medium disabled:opacity-40 ${
                    focusTarget === 'select'
                      ? 'bg-koma-accent text-koma-bg'
                      : 'bg-koma-fg/10 text-koma-fg hover:bg-koma-hover'
                  }`}
                >
                  <Check size={11} />
                  Select folder
                </button>
              </div>
            </div>
          </motion.div>
        </div>
      </div>
    </div>
  )
}
