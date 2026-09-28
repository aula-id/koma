// Content for the shared bottom panel.
import { AlertCircle, CheckCircle2 } from 'lucide-react'
import { useKoma, type LspRuntimeServer } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'

function PhaseIcon({ phase }: { phase: string }) {
  if (phase === 'error') return <AlertCircle size={12} className="flex-none text-koma-error" />
  if (phase === 'ready') return <CheckCircle2 size={12} className="flex-none text-koma-success" />
  return <BrailleSpinner size={12} className="flex-none text-koma-accent" />
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
  const servers = useKoma((s) => s.lspRuntime)
  const installProgress = useKoma((s) => s.lspProgress)

  const installRows = Object.values(installProgress).filter(
    (p) => p && (p.error || (p.pct >= 0 && p.pct < 100)),
  )

  return (
    <div className="min-h-0 flex-1 overflow-auto font-mono text-[11px]">
      {servers.length === 0 && installRows.length === 0 ? (
        <div className="px-3 py-4 text-koma-dim">No language servers running</div>
      ) : (
        <ul className="divide-y divide-koma-border/60">
          {installRows.map((p) => (
            <li key={`install:${p.id}`} className="flex items-start gap-2 px-3 py-1.5">
              <BrailleSpinner size={12} className="mt-0.5 flex-none text-koma-accent" />
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
  )
}
