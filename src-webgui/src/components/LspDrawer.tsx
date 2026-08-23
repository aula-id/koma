// Cross-tab Language Servers drawer — twin of ProblemsDrawer. Sits above
// UsageFooter and lists live LSP runtime (starting / indexing / ready / error).

import { AlertCircle, CheckCircle2, ChevronDown, Loader2, X } from 'lucide-react'
import { useKoma, type LspRuntimeServer } from '../store/koma'

function PhaseIcon({ phase }: { phase: string }) {
  if (phase === 'error') return <AlertCircle size={12} className="flex-none text-koma-error" />
  if (phase === 'ready') return <CheckCircle2 size={12} className="flex-none text-koma-success" />
  return <Loader2 size={12} className="flex-none animate-spin text-koma-accent" />
}

function phaseLabel(s: LspRuntimeServer): string {
  if (s.phase === 'error') return s.title || 'Error'
  if (s.phase === 'starting') return s.title || 'Starting'
  if (s.phase === 'working') {
    const t = s.title || 'Working'
    if (s.percentage != null) return `${t} ${s.percentage}%`
    return t
  }
  return 'Ready'
}

function rootLabel(root: string): string {
  if (!root) return ''
  const parts = root.replace(/\\/g, '/').split('/').filter(Boolean)
  return parts[parts.length - 1] || root
}

export function LspDrawer() {
  const open = useKoma((s) => s.lspDrawerOpen)
  const servers = useKoma((s) => s.lspRuntime)
  const installProgress = useKoma((s) => s.lspProgress)
  const setLspDrawerOpen = useKoma((s) => s.setLspDrawerOpen)

  if (!open) return null

  const installRows = Object.values(installProgress).filter(
    (p) => p && (p.error || (p.pct >= 0 && p.pct < 100)),
  )

  return (
    <div className="flex h-44 max-h-[40%] w-full flex-none flex-col border-t border-koma-border bg-koma-panel">
      <div className="flex h-7 flex-none items-center gap-2 border-b border-koma-border px-3 text-[11px] text-koma-dim">
        <span className="font-medium uppercase tracking-wide text-koma-fg/80">Language Servers</span>
        <span className="opacity-70">{servers.length}</span>
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => setLspDrawerOpen(false)}
          title="Collapse"
          className="flex h-5 w-5 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
        >
          <ChevronDown size={14} />
        </button>
        <button
          type="button"
          onClick={() => setLspDrawerOpen(false)}
          title="Close"
          className="flex h-5 w-5 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
        >
          <X size={13} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto font-mono text-[11px]">
        {servers.length === 0 && installRows.length === 0 ? (
          <div className="px-3 py-4 text-koma-dim">No language servers running</div>
        ) : (
          <ul className="divide-y divide-koma-border/60">
            {installRows.map((p) => (
              <li key={`install:${p.id}`} className="flex items-start gap-2 px-3 py-1.5">
                <Loader2 size={12} className="mt-0.5 flex-none animate-spin text-koma-accent" />
                <span className="min-w-0 flex-1 truncate text-koma-fg">
                  <span className="text-koma-accent">{p.id}</span>
                  <span className="mx-1.5 text-koma-dim">·</span>
                  <span className="opacity-90">
                    {p.error ? p.error : `Installing ${p.pct}%`}
                  </span>
                </span>
              </li>
            ))}
            {servers.map((s) => (
              <li key={s.id} className="flex items-start gap-2 px-3 py-1.5">
                <span className="mt-0.5">
                  <PhaseIcon phase={s.phase} />
                </span>
                <span className="min-w-0 flex-1 truncate text-koma-fg">
                  <span className="text-koma-accent">{s.name}</span>
                  {s.root ? (
                    <span className="text-koma-dim"> · {rootLabel(s.root)}</span>
                  ) : null}
                  <span className="mx-1.5 text-koma-dim">·</span>
                  <span className={s.phase === 'error' ? 'text-koma-error' : 'opacity-90'}>
                    {phaseLabel(s)}
                  </span>
                  {s.message ? (
                    <>
                      <span className="mx-1.5 text-koma-dim">·</span>
                      <span className="text-koma-dim opacity-90">{s.message}</span>
                    </>
                  ) : null}
                </span>
                {s.openDocs > 0 && (
                  <span className="flex-none text-[10px] text-koma-dim opacity-70">
                    {s.openDocs} doc{s.openDocs === 1 ? '' : 's'}
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}
