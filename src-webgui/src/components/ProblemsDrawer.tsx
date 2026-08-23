// Cross-tab Problems drawer — sits above UsageFooter, visible across every
// TabbedMain tab when expanded. Lists LSP diagnostics; click jumps to file:line.

import { useMemo } from 'react'
import { AlertCircle, AlertTriangle, ChevronDown, Info, X } from 'lucide-react'
import { useKoma, type LspDiagnostic } from '../store/koma'
import { uriToPath } from '../lib/monaco-lsp'

type Row = LspDiagnostic & { fileLabel: string }

function severityRank(s: number): number {
  if (s === 1) return 0
  if (s === 2) return 1
  if (s === 3) return 2
  return 3
}

function SeverityIcon({ severity }: { severity: number }) {
  if (severity === 1) return <AlertCircle size={12} className="flex-none text-koma-error" />
  if (severity === 2) return <AlertTriangle size={12} className="flex-none text-koma-warn" />
  return <Info size={12} className="flex-none text-koma-info" />
}

export function ProblemsDrawer() {
  const open = useKoma((s) => s.problemsOpen)
  const diagnostics = useKoma((s) => s.lspDiagnostics)
  const setProblemsOpen = useKoma((s) => s.setProblemsOpen)
  const openDiagnostic = useKoma((s) => s.openDiagnostic)

  const rows = useMemo(() => {
    const out: Row[] = []
    for (const [uri, list] of Object.entries(diagnostics)) {
      const abs = uriToPath(uri) ?? uri
      const fileLabel = abs.split('/').pop() ?? abs
      for (const d of list) {
        out.push({ ...d, uri: d.uri || uri, fileLabel })
      }
    }
    out.sort((a, b) => {
      const sr = severityRank(a.severity) - severityRank(b.severity)
      if (sr !== 0) return sr
      const f = a.fileLabel.localeCompare(b.fileLabel)
      if (f !== 0) return f
      return a.line - b.line
    })
    return out
  }, [diagnostics])

  if (!open) return null

  return (
    <div className="flex h-44 max-h-[40%] w-full flex-none flex-col border-t border-koma-border bg-koma-panel">
      <div className="flex h-7 flex-none items-center gap-2 border-b border-koma-border px-3 text-[11px] text-koma-dim">
        <span className="font-medium uppercase tracking-wide text-koma-fg/80">Problems</span>
        <span className="opacity-70">{rows.length}</span>
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => setProblemsOpen(false)}
          title="Collapse"
          className="flex h-5 w-5 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
        >
          <ChevronDown size={14} />
        </button>
        <button
          type="button"
          onClick={() => setProblemsOpen(false)}
          title="Close"
          className="flex h-5 w-5 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
        >
          <X size={13} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto font-mono text-[11px]">
        {rows.length === 0 ? (
          <div className="px-3 py-4 text-koma-dim">No problems</div>
        ) : (
          <ul className="divide-y divide-koma-border/60">
            {rows.map((r, i) => (
              <li key={`${r.uri}:${r.line}:${r.character}:${i}`}>
                <button
                  type="button"
                  onClick={() => openDiagnostic(r.uri, r.line, r.character)}
                  className="flex w-full items-start gap-2 px-3 py-1.5 text-left hover:bg-koma-hover"
                >
                  <SeverityIcon severity={r.severity} />
                  <span className="min-w-0 flex-1 truncate text-koma-fg">
                    <span className="text-koma-accent">{r.fileLabel}</span>
                    <span className="text-koma-dim">
                      :{r.line + 1}:{r.character + 1}
                    </span>
                    <span className="mx-1.5 text-koma-dim">·</span>
                    <span className="opacity-90">{r.message}</span>
                  </span>
                  {r.source && (
                    <span className="flex-none text-[10px] text-koma-dim opacity-70">{r.source}</span>
                  )}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}
