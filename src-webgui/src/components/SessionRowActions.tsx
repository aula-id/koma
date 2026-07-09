import { useMemo } from 'react'
import { Check, Loader2, Power, Trash2, X } from 'lucide-react'
import { useKoma } from '../store/koma'

// The single armed row across ResumePalette/StartScreen — kept LOCAL to
// whichever palette component renders the rows (not the store), per the
// "single armed row max" rule. `kind` disambiguates a cooking-session kill
// from a history-session delete (same id could theoretically collide across
// the two lists, though it never does in practice).
export type ArmedRow = { id: string; kind: 'session' | 'history' } | null

type SessionRowActionsProps = {
  id: string
  kind: 'session' | 'history'
  // Cooking rows only: true when this is the CURRENTLY-attached session — the
  // confirm label reads "close?" instead of "kill?" (same KillSession req
  // either way; only the copy differs).
  foreground?: boolean
  armed: ArmedRow
  onArm: (row: ArmedRow) => void
}

// Trailing ghost icon button on a Cooking/History row: arm -> confirm
// two-stage affordance for KillSession/DeleteSession. Renders a spinner
// (non-interactive) while the store's dyingSessions set carries this row's
// id — cleared automatically the moment a fresh Hub push confirms the
// kill/delete landed (see koma.ts's Hub push handler).
export function SessionRowActions({ id, kind, foreground, armed, onArm }: SessionRowActionsProps) {
  const req = useKoma((s) => s.req)
  const markDying = useKoma((s) => s.markDying)
  const detachSession = useKoma((s) => s.detachSession)
  const dying = useKoma((s) => s.dyingSessions.includes(id))
  const theme = useKoma((s) => s.config.theme)
  const palettes = useKoma((s) => s.config.palettes)

  // Danger/error palette role tint — same lookup ToastContainer uses for its
  // error icon (index 9 of the 11-role `PaletteInfo.colors` array). Never a
  // hardcoded red/orange. Memoized like ToastContainer's roleColor.
  const errorTint = useMemo(() => {
    const active = palettes.find((p) => p.name === theme)
    return active?.colors?.[9] || 'var(--koma-fg)'
  }, [palettes, theme])

  if (dying) {
    return (
      <Loader2
        size={13}
        tabIndex={-1}
        className="flex-none animate-spin opacity-70"
        style={{ color: errorTint }}
      />
    )
  }

  const isArmed = armed?.id === id && armed.kind === kind

  const confirm = () => {
    req(kind === 'session' ? { r: 'KillSession', id } : { r: 'DeleteSession', id })
    markDying(id)
    onArm(null)
    // Killing the CURRENTLY-attached session sends the host straight to the
    // swapper without ever emitting a Snapshot (only Hub pushes follow) — reset
    // the session slice locally so IndexPage falls back to StartScreen right
    // away instead of freezing on the now-dead chat.
    if (kind === 'session' && foreground) detachSession()
  }

  if (isArmed) {
    const label = kind === 'session' ? (foreground ? 'close?' : 'kill?') : 'delete forever?'
    return (
      <span
        // Stop the row's own click handler (SelectSession) from ever seeing
        // this mousedown — the row click-through must be suppressed while armed.
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => e.stopPropagation()}
        className="flex flex-none items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium"
        style={{ color: errorTint, backgroundColor: `color-mix(in srgb, ${errorTint} 16%, transparent)` }}
      >
        {label}
        <button
          onClick={() => confirm()}
          aria-label="Confirm"
          className="flex-none opacity-80 transition-opacity hover:opacity-100"
        >
          <Check size={12} className="flex-none" />
        </button>
        <button
          onClick={() => onArm(null)}
          aria-label="Cancel"
          className="flex-none opacity-80 transition-opacity hover:opacity-100"
        >
          <X size={12} className="flex-none" />
        </button>
      </span>
    )
  }

  const Icon = kind === 'session' ? Power : Trash2
  return (
    <button
      onClick={(e) => {
        e.stopPropagation()
        onArm({ id, kind })
      }}
      aria-label={kind === 'session' ? 'Kill session' : 'Delete session'}
      className="flex-none text-koma-fg opacity-0 transition-opacity group-hover:opacity-40 hover:!opacity-100 focus-visible:opacity-100"
    >
      <Icon size={13} className="flex-none" />
    </button>
  )
}
