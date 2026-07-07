import type { MouseEvent } from 'react'
import { motion } from 'framer-motion'
import { Recycle, Plus } from 'lucide-react'

export type Platform = 'macos' | 'linux' | 'windows'

// Shared spring for the pill <-> palette search-bar morph. MUST match the
// palette's search-bar transition so the morph is symmetric.
export const CMD_SEARCH_SPRING = { type: 'spring', stiffness: 450, damping: 34, mass: 0.6 } as const
export const CMD_SEARCH_WIDTH = 'w-[340px] max-w-[46vw]'

// window.__komaOS is injected by the Rust host (run_gui) before this script
// runs. Falls back to 'linux' (title-left / controls-right layout).
export function getPlatform(): Platform {
  const os = window.__komaOS
  return os === 'macos' || os === 'windows' ? os : 'linux'
}

function post(msg: unknown) {
  try {
    window.ipc?.postMessage(JSON.stringify(msg))
  } catch {
    /* ipc unavailable */
  }
}

type TitlebarProps = {
  onSearch: () => void
  onNewSession: () => void
  paletteOpen: boolean
}

// Custom titlebar. The centered command bar (search pill + new-session button)
// is excluded from window drag. When the resume palette is open the pill is
// unmounted so Framer morphs it (shared layoutId) into the palette search bar.
export function Titlebar({ onSearch, onNewSession, paletteOpen }: TitlebarProps) {
  function handleMouseDown(e: MouseEvent<HTMLDivElement>) {
    if (e.button !== 0) return
    const target = e.target as HTMLElement
    if (target.closest('.win-btn')) return // buttons handle themselves
    if (target.closest('#cmdbar')) return // command bar handles its own clicks
    if (e.detail === 2) {
      post({ t: 'win', a: 'max' }) // dbl-click = toggle max
      return
    }
    post({ t: 'win', a: 'drag' })
  }

  return (
    <div id="titlebar" onMouseDown={handleMouseDown}>
      <span id="title">koma</span>
      {!paletteOpen && (
        <div
          id="cmdbar"
          className="absolute inset-x-0 top-0 mx-auto flex h-full w-fit items-center gap-1.5"
        >
          <motion.button
            layoutId="cmd-search"
            transition={CMD_SEARCH_SPRING}
            onClick={onSearch}
            title="Change session"
            className={`flex h-[22px] ${CMD_SEARCH_WIDTH} items-center justify-start gap-2 rounded-md border border-koma-border bg-koma-panel px-2.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100`}
          >
            <Recycle size={13} className="flex-none" />
            <span className="truncate">change session</span>
          </motion.button>
          <button
            onClick={onNewSession}
            title="New session"
            aria-label="New session"
            className="flex h-[22px] w-[22px] flex-none items-center justify-center rounded-md border border-koma-border bg-koma-panel text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
          >
            <Plus size={14} />
          </button>
        </div>
      )}
      <div id="winctl">
        <button
          className="win-btn"
          id="btn-min"
          aria-label="Minimize"
          onClick={() => post({ t: 'win', a: 'min' })}
        >
          &#x2013;
        </button>
        <button
          className="win-btn"
          id="btn-max"
          aria-label="Maximize"
          onClick={() => post({ t: 'win', a: 'max' })}
        >
          &#x2610;
        </button>
        <button
          className="win-btn win-close"
          id="btn-close"
          aria-label="Close"
          onClick={() => post({ t: 'win', a: 'close' })}
        >
          &#x2715;
        </button>
      </div>
    </div>
  )
}
