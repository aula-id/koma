import { useEffect, useMemo, useRef, useState } from 'react'
import { ArrowRight, Clock, FolderPlus, Info, Sparkles, Zap } from 'lucide-react'
import { useKoma } from '../store/koma'

// Measures the component's own width with a ResizeObserver (a container query in
// JS) so the start screen can flip stacked -> side-by-side against the ACTUAL
// space it gets (the main area minus sidebar/activity-bar), not the raw
// viewport — which a window media query would get wrong when the sidebar is open.
function useContainerWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null)
  const [width, setWidth] = useState(0)
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) setWidth(e.contentRect.width)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])
  return [ref, width] as const
}

function Card({ children, className = '' }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={`rounded-xl border border-koma-border bg-koma-panel2 p-4 ${className}`}>
      {children}
    </div>
  )
}

function SectionLabel({ icon: Icon, children }: { icon: typeof Clock; children: string }) {
  return (
    <div className="mb-2 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-45">
      <Icon size={12} className="flex-none" />
      {children}
    </div>
  )
}

// VSCode-style pre-session START SCREEN — rendered INSTEAD of ChatView whenever
// no session is attached (the swapper/empty state). Quick-open recent sessions
// (from the host's authoritative hub history/cooking mirror) + a New session
// action (reuses the native folder-picker flow via GuiReq NewSession) + a short
// "about Koma" panel. Responsive: stacked when narrow, side-by-side when wide.
// The resume/change-session OVERLAY (ResumePalette) is unaffected — it stays the
// attached-state affordance.
export function StartScreen() {
  const history = useKoma((s) => s.hub.history)
  const cooking = useKoma((s) => s.hub.cooking)
  const req = useKoma((s) => s.req)
  const startSwitching = useKoma((s) => s.startSwitching)
  const [ref, width] = useContainerWidth<HTMLDivElement>()
  const wide = width >= 760

  // The host only discovers live sessions on demand — nudge a fresh Hub on
  // mount and on a short interval so recent/live rows stay current (same cadence
  // as ResumePalette).
  useEffect(() => {
    req({ r: 'RefreshHub' })
    const id = window.setInterval(() => req({ r: 'RefreshHub' }), 2000)
    return () => window.clearInterval(id)
  }, [req])

  // Live (cooking) sessions first, then past history — the same source rows the
  // ResumePalette lists, minus the synthetic `kind: 'new'` placeholder.
  const liveSessions = useMemo(
    () => cooking.filter((c) => c.kind === 'session' && c.id),
    [cooking],
  )

  const openSession = (id: string, name: string) => {
    // Optimistic swap overlay (no host "swap started" push; attach can block for
    // seconds) — cleared by the next authoritative Snapshot. Mirrors ResumePalette.
    startSwitching(name)
    req({ r: 'SelectSession', id })
  }
  // No optimistic loader: the host opens a native folder picker first and only
  // attaches once a folder is confirmed (cancel would strand the loader).
  const newSession = () => req({ r: 'NewSession' })

  const hasRecent = liveSessions.length > 0 || history.length > 0

  const actions = (
    <div className="flex min-w-0 flex-1 flex-col gap-4">
      <div>
        <div className="mb-1 flex items-baseline gap-2">
          <span className="text-[22px] font-bold text-koma-fg">koma</span>
          <span className="text-[12px] text-koma-fg opacity-45">start a session</span>
        </div>
      </div>

      <button
        onClick={newSession}
        className="group flex items-center gap-3 rounded-xl border border-koma-border bg-koma-panel px-4 py-3 text-left transition-colors hover:border-koma-accent/60 hover:bg-koma-hover"
      >
        <span className="flex h-9 w-9 flex-none items-center justify-center rounded-lg bg-koma-accent/15 text-koma-accent">
          <FolderPlus size={18} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block text-[13px] font-semibold text-koma-fg">New session</span>
          <span className="block text-[11px] text-koma-fg opacity-45">Pick a folder to work in</span>
        </span>
        <ArrowRight size={16} className="flex-none text-koma-fg opacity-30 transition group-hover:translate-x-0.5 group-hover:opacity-70" />
      </button>

      <Card>
        <SectionLabel icon={Clock}>Recent</SectionLabel>
        {!hasRecent ? (
          <div className="px-1 py-2 text-[12px] text-koma-fg opacity-35">No sessions yet — start a new one.</div>
        ) : (
          <div className="-mx-1 max-h-[40vh] overflow-y-auto">
            {liveSessions.map((c) => (
              <button
                key={c.id}
                onClick={() => c.id && openSession(c.id, c.name)}
                className="flex w-full items-center justify-between gap-2 rounded-lg px-3 py-2 text-left transition-colors hover:bg-koma-hover"
              >
                <span className="flex min-w-0 items-center gap-2">
                  <span className="h-1.5 w-1.5 flex-none animate-pulse rounded-full bg-emerald-500" />
                  <span className="truncate text-[12.5px] text-koma-fg">{c.name}</span>
                  {c.foreground && (
                    <span className="flex-none rounded border border-koma-border px-1 text-[9px] uppercase tracking-wide text-koma-fg opacity-50">
                      current
                    </span>
                  )}
                </span>
                {c.dirLabel && (
                  <span className="ml-2 flex-none truncate text-[11px] text-koma-fg opacity-40">{c.dirLabel}</span>
                )}
              </button>
            ))}
            {history.map((h) => (
              <button
                key={h.id}
                onClick={() => openSession(h.id, h.name)}
                className="flex w-full items-center justify-between gap-2 rounded-lg px-3 py-2 text-left transition-colors hover:bg-koma-hover"
              >
                <span className="truncate text-[12.5px] text-koma-fg">{h.name}</span>
                {h.dirLabel && (
                  <span className="ml-2 flex-none truncate text-[11px] text-koma-fg opacity-40">{h.dirLabel}</span>
                )}
              </button>
            ))}
          </div>
        )}
      </Card>
    </div>
  )

  const about = (
    <Card className={wide ? 'w-[300px] flex-none' : ''}>
      <SectionLabel icon={Info}>About koma</SectionLabel>
      <p className="text-[12.5px] leading-relaxed text-koma-fg opacity-80">
        A personal, terminal-first AI coding environment — agent + daemon at the core,
        driving your tools directly. This desktop shell renders your sessions natively
        while the daemon does the real work.
      </p>
      <div className="mt-3 space-y-2">
        <div className="flex items-start gap-2 text-[12px] text-koma-fg opacity-70">
          <Zap size={13} className="mt-0.5 flex-none text-koma-accent" />
          <span>Multiple live sessions, resumable any time.</span>
        </div>
        <div className="flex items-start gap-2 text-[12px] text-koma-fg opacity-70">
          <Sparkles size={13} className="mt-0.5 flex-none text-koma-accent" />
          <span>Bring your own provider, or run the free keyless tier.</span>
        </div>
      </div>
    </Card>
  )

  return (
    <div ref={ref} className="h-full w-full overflow-y-auto">
      <div className="mx-auto w-full max-w-[980px] px-6 py-10">
        <div className={`flex gap-6 ${wide ? 'flex-row items-start' : 'flex-col'}`}>
          {actions}
          {about}
        </div>
      </div>
    </div>
  )
}
