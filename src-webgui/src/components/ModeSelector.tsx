import { useEffect, useRef, useState } from 'react'
import { Check, ChevronDown, ListChecks, MessageSquare, Sparkles, type LucideIcon } from 'lucide-react'
import { useKoma } from '../store/koma'

// Agent-mode selector for the composer toolbar — koma's Auto/Plan/Normal (the
// TUI Shift+Tab / `/mode` banner). The active mode is DERIVED from the
// authoritative session.mode token the host projects on every Snapshot; picking
// a mode fires GuiReq SetMode{mode}, and the host's set_agent_mode choke-point
// (Plan enter/leave + system-prompt swap) re-projects the new token back. Yolo
// is intentionally omitted — it's a double-gated /security-armed mode, not a
// casual composer toggle.
const MODES: { value: string; label: string; Icon: LucideIcon }[] = [
  { value: 'auto', label: 'Auto', Icon: Sparkles },
  { value: 'plan', label: 'Plan', Icon: ListChecks },
  { value: 'normal', label: 'Normal', Icon: MessageSquare },
]

export function ModeSelector() {
  const mode = useKoma((s) => s.session.mode)
  const req = useKoma((s) => s.req)
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('mousedown', onDoc)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDoc)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  // The host token may be any of auto/normal/plan/yolo; fall back to Auto's
  // presentation for an unknown/unlisted token (e.g. yolo) so the trigger never
  // renders blank.
  const active = MODES.find((m) => m.value === mode) ?? MODES[0]
  const TriggerIcon = active.Icon

  const pick = (value: string) => {
    if (value !== mode) req({ r: 'SetMode', mode: value })
    setOpen(false)
  }

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        title="Agent mode"
        className="flex h-8 flex-none items-center gap-1 rounded-lg px-2 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
      >
        <TriggerIcon size={15} className="flex-none" />
        <span className="min-w-0 truncate">{active.label}</span>
        <ChevronDown size={12} className="flex-none opacity-60" />
      </button>
      {open && (
        // Opens UPWARD — sits just above the composer at the bottom of the chat.
        <div className="absolute bottom-[calc(100%+6px)] left-0 z-30 w-[180px] overflow-hidden rounded-md border border-koma-border bg-koma-panel py-1 shadow-sm">
          {MODES.map((m) => {
            const isActive = m.value === mode
            const RowIcon = m.Icon
            return (
              <button
                key={m.value}
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault()
                  pick(m.value)
                }}
                className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[12px] transition-colors ${
                  isActive
                    ? 'bg-koma-hover text-koma-fg'
                    : 'text-koma-fg opacity-75 hover:bg-koma-hover hover:opacity-100'
                }`}
              >
                <RowIcon size={13} className="flex-none opacity-70" />
                <span className="min-w-0 flex-1 truncate">{m.label}</span>
                {isActive && <Check size={12} className="flex-none text-koma-accent" />}
              </button>
            )
          })}
        </div>
      )}
    </div>
  )
}
