import { useEffect, useMemo, useRef, useState } from 'react'
import { Bot, Check, ChevronDown, Search, Sparkles } from 'lucide-react'
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
  const providers = useKoma((s) => s.config.providers)
  const oauthConns = useKoma((s) => s.oauth.conns)
  const req = useKoma((s) => s.req)
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const ref = useRef<HTMLDivElement>(null)

  // Resolve a model's `provider` (a provider_uuid) to a human label — either a
  // real config provider (by .id) or an OAuth connection (by .uuid), since the
  // daemon resolves provider_uuid against either catalogue (see
  // ConnectorPanel/Onboarding's providerOptions).
  const providerLabel = (uuid: string): string =>
    providers.find((p) => p.id === uuid)?.name ?? oauthConns.find((c) => c.uuid === uuid)?.name ?? 'unknown provider'

  // Global options, advertised-free pinned first (stable sort otherwise).
  const globals = useMemo(() => {
    const g = models.filter((m) => m.scope === 'global')
    return [...g].sort((a, b) => Number(!!b.free) - Number(!!a.free))
  }, [models])

  const localMain = useMemo(
    () => models.find((m) => m.scope === 'local' && m.roles.includes('main')),
    [models],
  )
  // A local override is a CLONE of a global model with a NEW uuid but the SAME
  // name (set_session_main copies chosen.name daemon-side) — so matching on
  // modelId+provider alone checks EVERY global sharing that model_id+provider
  // (e.g. two global entries both pointing at the same OAuth-backed grok-4.5).
  // Requiring the name too narrows it back down to the one actually picked.
  // Two globals with identical name+modelId+provider legitimately both
  // checking is an acceptable degenerate case — not worth over-engineering.
  const sameModel = (m: Model) =>
    localMain != null &&
    m.modelId === localMain.modelId &&
    m.provider === localMain.provider &&
    m.name === localMain.name

  const triggerLabel = localMain ? localMain.name || localMain.modelId || 'main' : '(inherit)'
  const triggerTitle = localMain
    ? `${localMain.modelId} · ${providerLabel(localMain.provider)}`
    : 'Session model'

  const filtered = query.trim()
    ? globals.filter((m) =>
        `${m.name} ${m.modelId} ${providerLabel(m.provider)}`.toLowerCase().includes(query.trim().toLowerCase()),
      )
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
        title={triggerTitle}
        className="flex h-8 max-w-[160px] flex-none items-center gap-1 rounded-lg px-2 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
      >
        <Bot size={15} className="flex-none" />
        <span className="min-w-0 truncate">{triggerLabel}</span>
        <ChevronDown size={12} className="flex-none opacity-60" />
      </button>
      {open && (
        // Opens UPWARD — the picker sits just above the composer at the bottom
        // of the chat.
        <div className="absolute bottom-[calc(100%+6px)] left-0 z-30 w-[260px] overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-sm">
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
                // Transparency: the row's primary label can be an opaque
                // nickname (m.name) — show the REAL model id + resolved
                // provider as a dim subtitle so "who is this model really?" is
                // always answerable at a glance. Skip the subtitle's own
                // modelId repeat when there's no nickname to disambiguate
                // (m.name empty/equal to modelId) — the primary line already
                // shows it.
                const hasNickname = m.name.trim() !== '' && m.name.trim() !== m.modelId
                return (
                  <button
                    key={m.id}
                    type="button"
                    onMouseDown={(e) => {
                      e.preventDefault()
                      pickModel(m)
                    }}
                    className={`flex w-full items-center gap-2 px-2 py-1.5 text-left text-[12px] transition-colors ${
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
                    <span className="min-w-0 flex-1">
                      <span className="block truncate">{m.name || m.modelId}</span>
                      {hasNickname && (
                        <span className="block truncate text-[10px] opacity-50">
                          {m.modelId} · {providerLabel(m.provider)}
                        </span>
                      )}
                    </span>
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
