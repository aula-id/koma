import { useEffect, useRef, useState } from 'react'
import { Check, ChevronDown, Gauge } from 'lucide-react'
import { useKoma } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'

// Composer EFFORT picker — TUI `/effort` parity. Unlike ModeSelector's FIXED
// mode list, the menu here is DERIVED per the foreground session's current
// model (host `GetEffortOptions` -> `effort_menu`, the same derivation the
// TUI's `/effort` command uses): a cold/mismatched catalogue cache reports
// "loading" (a fetch was just armed, or is already in flight), a model with
// no reasoning control reports "unsupported", and a resolved menu reports
// "ready" with the option list. Picking a row fires SetEffort{effort}; the
// trigger-pill label is DERIVED from the authoritative settingsValues.effort
// the host re-pushes on every pick — no local state beyond the open flag.
export function EffortPicker() {
  const effort = useKoma((s) => s.settingsValues?.effort ?? '')
  const menu = useKoma((s) => s.effortOptions)
  const req = useKoma((s) => s.req)
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const retriedRef = useRef(false)

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

  // Re-poll ONCE ~800ms after opening if still loading — the cold-cache fetch
  // the host just armed on open may have landed by then, so a single retry
  // usually turns "loading" into "ready"/"unsupported" without the user
  // having to close + reopen the picker. Resets on every open/close.
  useEffect(() => {
    if (!open) {
      retriedRef.current = false
      return
    }
    const isLoading = menu == null || menu.state === 'loading'
    if (!isLoading || retriedRef.current) return
    const t = window.setTimeout(() => {
      retriedRef.current = true
      req({ r: 'GetEffortOptions' })
    }, 800)
    return () => window.clearTimeout(t)
  }, [open, menu, req])

  const triggerLabel = effort === '' ? 'default' : effort
  const activeToken = effort === '' ? 'default' : effort

  const toggle = () => {
    if (!open) {
      // Clear any stale menu from a previous model/session before asking
      // fresh — the loading row shows immediately rather than a moment of
      // the last model's (possibly mismatched) options.
      useKoma.setState({ effortOptions: null })
      req({ r: 'GetEffortOptions' })
    }
    setOpen((o) => !o)
  }

  const pick = (value: string) => {
    if (value !== activeToken) req({ r: 'SetEffort', effort: value })
    setOpen(false)
  }

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={toggle}
        title="Reasoning effort"
        className="flex h-8 max-w-[120px] flex-none items-center gap-1 rounded-lg px-2 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100 @max-xs/chat:max-w-[5.5rem] @max-[14rem]/chat:max-w-none @max-[14rem]/chat:px-1.5"
      >
        <Gauge size={15} className="flex-none" />
        <span className="min-w-0 truncate @max-[14rem]/chat:hidden">{triggerLabel}</span>
        <ChevronDown size={12} className="flex-none opacity-60 @max-[14rem]/chat:hidden" />
      </button>
      {open && (
        // Opens UPWARD — sits just above the composer at the bottom of the chat.
        <div className="absolute bottom-[calc(100%+6px)] left-0 z-30 w-[min(200px,calc(100cqw-1rem))] max-w-[calc(100vw-1rem)] overflow-hidden rounded-md border border-koma-border bg-koma-panel py-1 shadow-sm">
          {menu == null || menu.state === 'loading' ? (
            <div className="flex items-center gap-2 px-2 py-1 text-[12px] text-koma-fg opacity-50">
              <BrailleSpinner size={12} />
              <span className="min-w-0 flex-1">{menu?.note || 'fetching model capabilities…'}</span>
            </div>
          ) : menu.state === 'unsupported' ? (
            <div className="px-2 py-1 text-[12px] text-koma-fg opacity-40">
              {menu.note || 'model has no thinking control'}
            </div>
          ) : (
            menu.options.map((opt) => {
              const isActive = opt === activeToken
              return (
                <button
                  key={opt}
                  type="button"
                  onMouseDown={(e) => {
                    e.preventDefault()
                    pick(opt)
                  }}
                  className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[12px] transition-colors ${
                    isActive
                      ? 'bg-koma-hover text-koma-fg'
                      : 'text-koma-fg opacity-75 hover:bg-koma-hover hover:opacity-100'
                  }`}
                >
                  <span className="min-w-0 flex-1 truncate">{opt}</span>
                  {isActive && <Check size={12} className="flex-none text-koma-accent" />}
                </button>
              )
            })
          )}
        </div>
      )}
    </div>
  )
}
