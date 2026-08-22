import type { MouseEvent } from 'react'
import { motion } from 'framer-motion'
import { Terminal, PenLine, FoldVertical, SquareTerminal } from 'lucide-react'
import { useKoma } from '../store/koma'

export type Platform = 'macos' | 'linux' | 'windows'

// Shared spring + width so the 'change session' pill and the resume palette /
// rename overlay morph between matching footprints.
export const CMD_SEARCH_SPRING = { type: 'spring', stiffness: 450, damping: 50, mass: 0.6 } as const
export const CMD_SEARCH_WIDTH = 'w-[340px] max-w-[46vw]'

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

// Custom titlebar. Centered command bar = the 'change session' pill (morphs into
// the resume palette) + a 'rename' button (morphs into the rename overlay).
// #cmdbar spans the titlebar (to center the pill + anchor the rename button) but
// is pointer-events-none so empty areas still drag the window; only the buttons
// capture clicks. Button contents use layout="position" so the shared-layout
// morph repositions them without scale-stretching the icon/text.
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
      <span id="title">{remoteLive ? 'koma · remote' : 'koma'}</span>
      {!overlayOpen && (
        <div
          id="cmdbar"
          className="pointer-events-none absolute inset-x-0 top-0 flex h-full items-center justify-center"
        >
          {/* Terminal button — always visible, left of the session pill.
              Creates a new interactive terminal tab on each click. */}
          <button
            onClick={onTerminal}
            title="New Terminal"
            aria-label="New Terminal"
            className="pointer-events-auto mr-2 flex h-[22px] items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-2 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover"
          >
            <SquareTerminal size={13} className="flex-none" />
          </button>
          <motion.button
            layoutId="cmd-search"
            transition={CMD_SEARCH_SPRING}
            onClick={onSearch}
            title="Change session"
            className={`pointer-events-auto flex h-[22px] ${CMD_SEARCH_WIDTH} items-center justify-start gap-2 rounded-md border border-koma-border bg-koma-panel px-2.5 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover`}
          >
            <motion.span layout="position" className="flex items-center gap-2">
              <Terminal size={13} className="flex-none" />
              <span className="truncate">change session</span>
            </motion.span>
          </motion.button>
          {/* Rename + compact as ONE flex group, absolutely positioned to track
              the pill's actual half-width (min(170px, 23vw) — the pill is
              w-[340px] max-w-[46vw], so its half-width shrinks past ~740px
              window width). Two independently-absolute offsets calibrated to
              the full 340px pill used to drift off the pill's edge + collide
              with the window controls once the pill shrank; this single
              group hugs the pill's right edge at every size instead. Hidden
              entirely with no session attached (welcome screen) — neither
              action applies there; the "change session" pill alone stays. */}
          {sessionId && (
            <div
              className="pointer-events-none absolute top-[5px] left-[calc(50%+min(170px,23vw)+8px)] flex items-center gap-2"
            >
              <motion.button
                layoutId="cmd-rename"
                transition={CMD_SEARCH_SPRING}
                onClick={onRename}
                title="Rename session"
                aria-label="Rename session"
                className="pointer-events-auto flex h-[22px] items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-2.5 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover"
              >
                <motion.span layout="position" className="flex items-center gap-1.5">
                  <PenLine size={13} className="flex-none" />
                  <span className="max-[700px]:hidden">rename</span>
                </motion.span>
              </motion.button>
              <button
                onClick={() => req({ r: 'Compact' })}
                disabled={working}
                title="Compact context"
                aria-label="Compact context"
                className={`pointer-events-auto flex h-[22px] items-center gap-1.5 rounded-md border border-koma-border bg-koma-panel px-2.5 text-[12px] transition-colors ${
                  working
                    ? 'text-koma-dim opacity-40'
                    : 'text-koma-fg hover:bg-koma-hover'
                }`}
              >
                <FoldVertical size={13} className="flex-none" />
                <span className="max-[700px]:hidden">compact</span>
              </button>
            </div>
          )}
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
