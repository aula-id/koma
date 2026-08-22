import { useEffect, useMemo } from 'react'
import { motion } from 'framer-motion'
import { Check, ChevronRight, Folder, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'
import { CMD_SEARCH_WIDTH } from './Titlebar'

/**
 * Remote folder picker overlay. Driven by host `RemotePathPicker` pushes
 * (listing / ready / error / cancelled). Confirm mints a new remote session
 * with that cwd; Esc/cancel aborts without attaching.
 */
export function RemotePathPicker() {
  const remotePath = useKoma((s) => s.remotePath)
  const req = useKoma((s) => s.req)
  const active =
    remotePath.state === 'listing' ||
    remotePath.state === 'ready' ||
    remotePath.state === 'error'

  useEffect(() => {
    if (!active) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        req({ r: 'CancelRemotePath' })
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active, req])

  const parentPath = useMemo(() => {
    const p = remotePath.path.replace(/\/+$/, '')
    if (!p || p === '/') return null
    const i = p.lastIndexOf('/')
    if (i <= 0) return '/'
    return p.slice(0, i) || '/'
  }, [remotePath.path])

  if (!active) return null

  const cancel = () => req({ r: 'CancelRemotePath' })
  const confirm = () => {
    if (!remotePath.path) return
    req({ r: 'ConfirmRemotePath', path: remotePath.path })
  }
  const enter = (dir: string) => req({ r: 'ListRemotePath', path: dir })

  return (
    <div className="absolute inset-0 z-[70]" onMouseDown={cancel}>
      <motion.div
        initial={{ opacity: 0, y: -4 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.12, ease: 'easeOut' }}
        className={`mx-auto mt-[5px] ${CMD_SEARCH_WIDTH}`}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-xl">
          <div className="flex h-[28px] items-center gap-1.5 border-b border-koma-border px-2.5">
            <Folder size={12} className="flex-none text-koma-dim" />
            <span className="min-w-0 flex-1 truncate text-[12px] text-koma-fg">
              {remotePath.path || '…'}
            </span>
            {remotePath.state === 'listing' && <BrailleSpinner size={12} />}
            <button
              type="button"
              aria-label="Cancel"
              onClick={cancel}
              className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-dim hover:text-koma-fg"
            >
              <X size={12} />
            </button>
          </div>
          {remotePath.error && (
            <div className="border-b border-koma-border px-2.5 py-1.5 text-[11px] text-koma-dim">
              {remotePath.error}
            </div>
          )}
          <div className="max-h-[220px] overflow-y-auto py-1">
            {parentPath && (
              <button
                type="button"
                onClick={() => enter(parentPath)}
                className="flex w-full items-center gap-1.5 px-2.5 py-1 text-left text-[12px] text-koma-dim hover:bg-koma-bg"
              >
                <ChevronRight size={11} className="rotate-180" />
                ..
              </button>
            )}
            {remotePath.dirs.length === 0 && remotePath.state === 'ready' && (
              <div className="px-2.5 py-2 text-[11px] text-koma-dim">No subfolders</div>
            )}
            {remotePath.dirs.map((dir) => {
              const name = dir.replace(/\/+$/, '').split('/').pop() || dir
              return (
                <button
                  key={dir}
                  type="button"
                  onClick={() => enter(dir)}
                  className="flex w-full items-center gap-1.5 px-2.5 py-1 text-left text-[12px] text-koma-fg hover:bg-koma-bg"
                >
                  <Folder size={11} className="flex-none text-koma-dim" />
                  <span className="min-w-0 truncate">{name}</span>
                </button>
              )
            })}
          </div>
          <div className="flex items-center justify-end gap-1.5 border-t border-koma-border px-2 py-1.5">
            <button
              type="button"
              onClick={cancel}
              className="rounded px-2 py-0.5 text-[11px] text-koma-dim hover:text-koma-fg"
            >
              cancel
            </button>
            <button
              type="button"
              onClick={confirm}
              disabled={!remotePath.path || remotePath.state === 'listing'}
              className="flex items-center gap-1 rounded bg-koma-fg/10 px-2 py-0.5 text-[11px] font-medium text-koma-fg disabled:opacity-40"
            >
              <Check size={11} />
              open here
            </button>
          </div>
        </div>
      </motion.div>
    </div>
  )
}
