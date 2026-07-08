import { useEffect, useMemo, useRef, useState } from 'react'
import { Check, ChevronDown, Search, Sparkles } from 'lucide-react'
import { useKoma } from '../store/koma'
import type { Model } from '../types/config'

// GitHub-Copilot-style SEARCHABLE quick-picker for the session-local `main`
// model override. Options are the GLOBAL models (config.models, scope=global —
// the user grows this list by adding global models in the Connector); the
// advertised koma-free model (`model.free`) pins to the TOP. Picking a model
// clones it into a session-local `main` override on the host (GuiReq
// SetSessionMain{modelUuid}); "(inherit)" removes the override and reverts to
// the Connector global main (SetSessionMain{modelUuid:null}).
//
// The current selection is DERIVED from the authoritative config slice: the
// session-local model holding the `main` role is the active override, else
// "(inherit)". A local override is a CLONE of a global model with a NEW uuid,
// so the active global row is matched by model id + provider (not by uuid).
export function ModelPicker() {
  const models = useKoma((s) => s.config.models)
  const req = useKoma((s) => s.req)
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const ref = useRef<HTMLDivElement>(null)

  // Global options, advertised-free pinned first (stable sort otherwise).
  const globals = useMemo(() => {
    const g = models.filter((m) => m.scope === 'global')
    return [...g].sort((a, b) => Number(!!b.free) - Number(!!a.free))
  }, [models])

  const localMain = useMemo(
    () => models.find((m) => m.scope === 'local' && m.roles.includes('main')),
    [models],
  )
  const sameModel = (m: Model) =>
    localMain != null && m.modelId === localMain.modelId && m.provider === localMain.provider

  const triggerLabel = localMain ? localMain.name || localMain.modelId || 'main' : '(inherit)'

  const filtered = query.trim()
    ? globals.filter((m) => `${m.name} ${m.modelId}`.toLowerCase().includes(query.trim().toLowerCase()))
    : globals

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

  const close = () => {
    setOpen(false)
    setQuery('')
  }
  // Commit via onMouseDown+preventDefault (same focus-race fix as form.Select):
  // keeps focus off the search input during the pick so the outside-click
  // handler never races the click.
  const pickModel = (m: Model) => {
    req({ r: 'SetSessionMain', modelUuid: m.id })
    close()
  }
  const pickInherit = () => {
    req({ r: 'SetSessionMain', modelUuid: null })
    close()
  }

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        title="Session model"
        className="flex max-w-[240px] items-center gap-1 rounded-md px-2 py-1 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
      >
        <span className="truncate">{triggerLabel}</span>
        <ChevronDown size={13} className="flex-none opacity-60" />
      </button>
      {open && (
        // Opens UPWARD — the picker sits just above the composer at the bottom
        // of the chat.
        <div className="absolute bottom-[calc(100%+6px)] left-0 z-30 w-[260px] overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-xl">
          <div className="flex h-[26px] items-center gap-2 border-b border-koma-border px-2">
            <Search size={12} className="flex-none text-koma-fg opacity-50" />
            <input
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search models…"
              className="w-full bg-transparent text-[12px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-40"
            />
          </div>
          <div className="max-h-[240px] overflow-y-auto py-1">
            {/* (inherit) — default: removes the session-local override. */}
            <button
              type="button"
              onMouseDown={(e) => {
                e.preventDefault()
                pickInherit()
              }}
              className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[12px] transition-colors ${
                !localMain
                  ? 'bg-koma-hover text-koma-fg'
                  : 'text-koma-fg opacity-75 hover:bg-koma-hover hover:opacity-100'
              }`}
            >
              {!localMain ? (
                <Check size={12} className="flex-none text-koma-accent" />
              ) : (
                <span className="w-3 flex-none" />
              )}
              <span className="truncate">(inherit) — global main</span>
            </button>
            {filtered.length === 0 ? (
              <div className="px-2 py-1 text-[11px] text-koma-fg opacity-40">No global models</div>
            ) : (
              filtered.map((m) => {
                const active = sameModel(m)
                return (
                  <button
                    key={m.id}
                    type="button"
                    onMouseDown={(e) => {
                      e.preventDefault()
                      pickModel(m)
                    }}
                    className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[12px] transition-colors ${
                      active
                        ? 'bg-koma-hover text-koma-fg'
                        : 'text-koma-fg opacity-75 hover:bg-koma-hover hover:opacity-100'
                    }`}
                  >
                    {active ? (
                      <Check size={12} className="flex-none text-koma-accent" />
                    ) : (
                      <span className="w-3 flex-none" />
                    )}
                    <span className="min-w-0 flex-1 truncate">{m.name || m.modelId}</span>
                    {m.free && (
                      <span className="flex flex-none items-center gap-0.5 rounded bg-koma-accent/15 px-1 text-[9px] uppercase tracking-wide text-koma-accent">
                        <Sparkles size={9} /> free
                      </span>
                    )}
                  </button>
                )
              })
            )}
          </div>
        </div>
      )}
    </div>
  )
}
