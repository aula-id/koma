import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronDown, ListChecks, MessageSquare, Shield, Sparkles, type LucideIcon } from 'lucide-react'
import { useKoma } from '../store/koma'

// Agent-mode selector for the composer toolbar — koma's Auto/Plan/Normal/SDLC (the
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
  { value: 'sdlc', label: 'SDLC', Icon: Shield },
]

const MENU_W = 180

function useAnchorRect(open: boolean, ref: React.RefObject<HTMLElement | null>) {
  const [rect, setRect] = useState<DOMRect | null>(null)
  useEffect(() => {
    if (!open) {
      setRect(null)
      return
    }
    const update = () => {
      if (ref.current) setRect(ref.current.getBoundingClientRect())
    }
    update()
    window.addEventListener('scroll', update, true)
    window.addEventListener('resize', update)
    return () => {
      window.removeEventListener('scroll', update, true)
      window.removeEventListener('resize', update)
    }
  }, [open, ref])
  return rect
}

export function ModeSelector() {
  const mode = useKoma((s) => s.session.mode)
  const req = useKoma((s) => s.req)
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const rect = useAnchorRect(open, ref)

  useEffect(() => {
    if (!open) return
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node
      if (ref.current?.contains(t) || menuRef.current?.contains(t)) return
      setOpen(false)
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

  // The host token may be any of auto/normal/plan/yolo/sdlc; fall back to Auto's
  // presentation for an unknown/unlisted token (e.g. yolo) so the trigger never
  // renders blank.
  const active = MODES.find((m) => m.value === mode) ?? MODES[0]
  const TriggerIcon = active.Icon

  const pick = (value: string) => {
    if (value !== mode) req({ r: 'SetMode', mode: value })
    setOpen(false)
  }

  // Drop-up menu portals to body (same as form.Select): the toolbar wraps
  // pickers in overflow-x-auto, which clips absolute menus and makes them
  // unclickable.
  const menu =
    open &&
    rect &&
    createPortal(
      <div
        ref={menuRef}
        style={{
          position: 'fixed',
          left: Math.max(8, Math.min(rect.left, window.innerWidth - MENU_W - 8)),
          bottom: window.innerHeight - rect.top + 6,
          width: MENU_W,
          zIndex: 80,
        }}
        className="overflow-hidden rounded-md border border-koma-border bg-koma-panel py-1 shadow-sm"
      >
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
      </div>,
      document.body,
    )

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        title="Agent mode"
        className="flex h-8 flex-none items-center gap-1 rounded-lg px-2 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100 @max-[14rem]/chat:px-1.5"
      >
        <TriggerIcon size={15} className="flex-none" />
        <span className="min-w-0 truncate @max-[14rem]/chat:hidden">{active.label}</span>
        <ChevronDown size={12} className="flex-none opacity-60 @max-[14rem]/chat:hidden" />
      </button>
      {menu}
    </div>
  )
}
