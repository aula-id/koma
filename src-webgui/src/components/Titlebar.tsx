import type { MouseEvent } from 'react'
import { motion } from 'framer-motion'
import { Terminal, PenLine, FoldVertical, SquareTerminal } from 'lucide-react'
import { useKoma } from '../store/koma'

export type Platform = 'macos' | 'linux' | 'windows'

// Shared spring + width so the 'change session' pill and the resume palette /
// rename overlay morph between matching footprints. Cap leaves room for the
// left title + right window controls so the centered cluster never covers
// #winctl on narrow windows.
export const CMD_SEARCH_SPRING = { type: 'spring', stiffness: 450, damping: 50, mass: 0.6 } as const
export const CMD_SEARCH_WIDTH = 'w-[340px] max-w-[min(46vw,calc(100vw-22rem))]'

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
  onRename: () => void
  onTerminal: () => void
  overlayOpen: boolean
}

// Custom titlebar. Centered cmd cluster: terminal · change-session pill · rename
// · compact. #cmdbar is pointer-events-none (empty areas drag the window); only
// the buttons capture clicks. Side padding + a capped pill width keep the
// cluster off #winctl. Labels collapse on narrow widths (session / icon-only).
// Button contents use layout="position" so shared-layout morphs don't stretch.
export function Titlebar({ onSearch, onRename, onTerminal, overlayOpen }: TitlebarProps) {
  const req = useKoma((s) => s.req)
  const working = useKoma((s) => s.session.working)
  // null on the welcome/start screen (no session attached) — rename/compact
  // don't apply there, so the whole group hides; the "change session" pill
  // stays visible regardless.
  const sessionId = useKoma((s) => s.session.id)
  const remoteState = useKoma((s) => s.remoteState)
  // Accent-tint the drag chrome whenever the GUI is bound to an SSH target
  // (hub ready OR live remote session) — clear "not LOCAL" signal at a glance.
  const remoteLive =
    remoteState.state === 'ready' || remoteState.state === 'connected'
  const remoteTarget =
    remoteLive && remoteState.user && remoteState.host
      ? `${remoteState.user}@${remoteState.host}`
      : null

  function handleMouseDown(e: MouseEvent<HTMLDivElement>) {
    if (e.button !== 0) return
    const target = e.target as HTMLElement
    if (target.closest('.win-btn')) return
    if (target.closest('#cmdbar')) return
    if (e.detail === 2) {
      post({ t: 'win', a: 'max' })
      return
    }
    post({ t: 'win', a: 'drag' })
  }

  return (
    <div
      id="titlebar"
      className={remoteLive ? 'is-remote' : undefined}
      title={remoteTarget ? `Remote: ${remoteTarget}` : undefined}
      onMouseDown={handleMouseDown}
    >
      <span id="title">koma</span>
      {!overlayOpen && (
        <div
          id="cmdbar"
          className="pointer-events-none absolute inset-x-0 top-0 flex h-full items-center justify-center px-[6.5rem]"
        >
          {/* One centered cluster (terminal · session · rename · compact).
              Flex — not absolute offsets — so the group never drifts over
              #winctl when the window narrows. Side padding reserves chrome. */}
          <div className="pointer-events-none flex min-w-0 max-w-full items-center gap-1.5">
            <button
              onClick={onTerminal}
              title="New Terminal"
              aria-label="New Terminal"
              className="pointer-events-auto flex h-[22px] flex-none items-center rounded-md border border-koma-border bg-koma-panel px-1.5 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover"
            >
              <SquareTerminal size={13} className="flex-none" />
            </button>
            <motion.button
              layoutId="cmd-search"
              transition={CMD_SEARCH_SPRING}
              onClick={onSearch}
              title="Change session"
              aria-label="Change session"
              className={`pointer-events-auto flex h-[22px] min-w-0 ${CMD_SEARCH_WIDTH} items-center justify-start gap-1.5 rounded-md border border-koma-border bg-koma-panel px-2 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover`}
            >
              <motion.span layout="position" className="flex min-w-0 items-center gap-1.5">
                <Terminal size={13} className="flex-none" />
                {/* Full label → short → icon-only as width drops. */}
                <span className="truncate max-[900px]:hidden">change session</span>
                <span className="hidden truncate max-[900px]:inline max-[640px]:hidden">
                  session
                </span>
              </motion.span>
            </motion.button>
            {/* Hidden with no session (welcome) — neither action applies. */}
            {sessionId && (
              <>
                <motion.button
                  layoutId="cmd-rename"
                  transition={CMD_SEARCH_SPRING}
                  onClick={onRename}
                  title="Rename session"
                  aria-label="Rename session"
                  className="pointer-events-auto flex h-[22px] flex-none items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-1.5 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover min-[1101px]:px-2.5"
                >
                  <motion.span layout="position" className="flex items-center gap-1.5">
                    <PenLine size={13} className="flex-none" />
                    <span className="max-[1100px]:hidden">rename</span>
                  </motion.span>
                </motion.button>
                <button
                  onClick={() => req({ r: 'Compact' })}
                  disabled={working}
                  title="Compact context"
                  aria-label="Compact context"
                  className={`pointer-events-auto flex h-[22px] flex-none items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-1.5 text-[12px] transition-colors min-[1101px]:px-2.5 ${
                    working
                      ? 'text-koma-dim opacity-40'
                      : 'text-koma-fg hover:bg-koma-hover'
                  }`}
                >
                  <FoldVertical size={13} className="flex-none" />
                  <span className="max-[1100px]:hidden">compact</span>
                </button>
              </>
            )}
          </div>
        </div>
      )}
      <div id="winctl" className="relative z-20">
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
