import { useMemo } from 'react'
import { Check, ExternalLink, Power, Trash2, X } from 'lucide-react'
import { useKoma, isDying } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'

// The single armed row across ResumePalette/StartScreen — kept LOCAL to
// whichever palette component renders the rows (not the store), per the
// "single armed row max" rule. `kind` disambiguates a cooking-session kill
// from a history-session delete (same id could theoretically collide across
// the two lists, though it never does in practice).
export type ArmedRow = { id: string; kind: 'session' | 'history' } | null

type SessionRowActionsProps = {
  id: string
  kind: 'session' | 'history'
  armed: ArmedRow
  onArm: (row: ArmedRow) => void
  /** When set, show "open in new window" for multi-window multi-attach. */
  remoteHostId?: string | null
}

// Trailing ghost icon on a Cooking/History row. Meant to be rendered INSIDE a
// fixed-width trailing action cell (~28px, see ResumePalette/StartScreen) —
// its own hit area fills that whole cell (h-full w-full) so the clickable
// target is exactly the reserved column, never the row's text/content cell
// next to it (a destructive button overlapping clickable text was a misclick
// hazard per live feedback). Clicking arms the row — the parent
// (ResumePalette/StartScreen) then swaps the row's ENTIRE content over to a
// `SessionRowConfirmStrip` (a full-width confirm replaces the mini pill; too
// small to comfortably hit per earlier live-test feedback). Renders a spinner
// (non-interactive) instead while `dyingSessions` carries a kind-matching
// mark for this id — cleared automatically the moment a fresh Hub push
// confirms the kill/delete landed (see koma.ts's Hub push handler).
export function SessionRowActions({ id, kind, armed, onArm, remoteHostId }: SessionRowActionsProps) {
  const dying = useKoma((s) => isDying(s.dyingSessions, id, kind))
  const theme = useKoma((s) => s.config.theme)
  const palettes = useKoma((s) => s.config.palettes)
  const req = useKoma((s) => s.req)
  const remoteState = useKoma((s) => s.remoteState)

  // Reserved for future row-action tints (confirm strip uses its own lookup).
  const _errorTint = useMemo(() => {
    const active = palettes.find((p) => p.name === theme)
    return active?.colors?.[9] || 'var(--koma-fg)'
  }, [palettes, theme])
  void _errorTint

  if (dying) {
    return <BrailleSpinner size={13} />
  }

  // Already armed — the parent is rendering the full-row SessionRowConfirmStrip
  // in place of the row's normal content instead of this trailing slot.
  if (armed?.id === id && armed.kind === kind) return null

  const hostId =
    remoteHostId ??
    (remoteState.state === 'ready' || remoteState.state === 'connected'
      ? remoteState.hostId
      : null)

  const Icon = kind === 'session' ? Power : Trash2
  return (
    <span className="flex h-full w-full flex-none items-center justify-end gap-0.5">
      {kind === 'session' && (
        <button
          onClick={(e) => {
            e.stopPropagation()
            req({
              r: 'OpenSecondWindow',
              sessionId: id,
              ...(hostId ? { hostId } : {}),
            })
          }}
          aria-label="Open in new window"
          title="Open in new window"
          className="flex h-full w-6 flex-none items-center justify-center text-koma-fg opacity-0 transition-opacity group-hover:opacity-40 hover:!opacity-100 focus-visible:opacity-100"
        >
          <ExternalLink size={12} className="flex-none" />
        </button>
      )}
      <button
        onClick={(e) => {
          e.stopPropagation()
          onArm({ id, kind })
        }}
        aria-label={kind === 'session' ? 'Kill session' : 'Delete session'}
        className="flex h-full w-6 flex-none items-center justify-center text-koma-fg opacity-0 transition-opacity group-hover:opacity-40 hover:!opacity-100 focus-visible:opacity-100"
      >
        <Icon size={13} className="flex-none" />
      </button>
    </span>
  )
}

type SessionRowConfirmStripProps = {
  id: string
  kind: 'session' | 'history'
  // Cooking rows only: true when this is the CURRENTLY-attached session — the
  // label reads "close session?" instead of "kill session?" (same KillSession
  // req either way; only the copy differs).
  foreground?: boolean
  onCancel: () => void
  // Padding/rounding classes matching the row's own (so the tinted strip
  // fills the exact same box the row's normal content would have occupied —
  // same row height, just fully replacing what's inside it).
  className?: string
}

// Full-row kill/delete confirmation — REPLACES a row's entire normal content
// (name/badges/dirLabel/ghost-icon) while that row is armed. Strict
// |label|yes|no| layout: label takes the flexible remaining space (truncates
// rather than push the buttons around), yes/no are fixed, text-labeled
// buttons (not icon-only — Agung's spec) with generous horizontal padding.
// Same error-role tint the ghost icon's spinner uses.
export function SessionRowConfirmStrip({ id, kind, foreground, onCancel, className = '' }: SessionRowConfirmStripProps) {
  const req = useKoma((s) => s.req)
  const markDying = useKoma((s) => s.markDying)
  const detachSession = useKoma((s) => s.detachSession)
  const theme = useKoma((s) => s.config.theme)
  const palettes = useKoma((s) => s.config.palettes)

  const errorTint = useMemo(() => {
    const active = palettes.find((p) => p.name === theme)
    return active?.colors?.[9] || 'var(--koma-fg)'
  }, [palettes, theme])

  const label = kind === 'history' ? 'delete forever?' : foreground ? 'close session?' : 'kill session?'

  const confirm = () => {
    req(kind === 'session' ? { r: 'KillSession', id } : { r: 'DeleteSession', id })
    markDying(id, kind === 'session' ? 'kill' : 'delete')
    // Killing the CURRENTLY-attached session sends the host straight to the
    // swapper without ever emitting a Snapshot (only Hub pushes follow) —
    // reset the session slice locally so IndexPage falls back to StartScreen
    // right away instead of freezing on the now-dead chat.
    if (kind === 'session' && foreground) detachSession()
    onCancel()
  }

  return (
    <div
      // Stop the row's own click handler (SelectSession) and the palette
      // overlay's outside-click close from ever seeing this mousedown.
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      className={`flex w-full items-center justify-between text-[12px] font-medium ${className}`}
      style={{ color: errorTint, backgroundColor: `color-mix(in srgb, ${errorTint} 16%, transparent)` }}
    >
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <span className="flex flex-none items-center gap-1.5">
        {/* Vertical padding intentionally omitted — the strip's OWN container
            already carries the row's normal px-3/py (passed via `className`),
            which sets the row's height. Adding py here too would make the
            button (and thus the whole flex row, via items-center) taller than
            an unarmed row, causing a layout shift when a row arms. Horizontal
            padding stays generous for the hit target. */}
        <button
          onClick={confirm}
          autoFocus
          aria-label="Confirm"
          className="flex flex-none items-center gap-1 rounded px-3 text-[12px] font-semibold opacity-90 transition-opacity hover:opacity-100 focus-visible:opacity-100"
          style={{ color: errorTint }}
        >
          <Check size={13} className="flex-none" />
          yes
        </button>
        <button
          onClick={onCancel}
          aria-label="Cancel"
          className="flex flex-none items-center gap-1 rounded px-3 text-[12px] text-koma-fg opacity-70 transition-opacity hover:opacity-100 focus-visible:opacity-100"
        >
          <X size={13} className="flex-none" />
          no
        </button>
      </span>
    </div>
  )
}
