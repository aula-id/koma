import { FileText } from 'lucide-react'
import { useKoma } from '../store/koma'
import { relTime } from './GraphRow'
import { BrailleSpinner } from './BrailleSpinner'
import { baseName, dirName } from '../utils/path'


// git status char -> badge tone (mirrors GitPanel's STATUS_TONE): A = added
// (good), M = modified (accent), D = deleted (error), R/C = rename/copy (warn).
const FILE_TONE: Record<string, string> = {
  A: 'text-koma-success',
  M: 'text-koma-accent',
  D: 'text-koma-error',
  R: 'text-koma-warn',
  C: 'text-koma-warn',
}
function fileTone(status: string): string {
  return FILE_TONE[status.charAt(0)] ?? 'text-koma-dim'
}

// The commit-graph tab's detail pane (bottom split): full metadata + body +
// clickable parent short-shas + the changed-file list (click a file to open its
// commit-vs-first-parent diff tab). Renders nothing when nothing is selected; a
// loading spinner until the detail matching the current selection lands.
export function GraphDetail() {
  const detail = useKoma((s) => s.graph.detail)
  const selectedSha = useKoma((s) => s.graph.selectedSha)
  const selectCommit = useKoma((s) => s.selectCommit)
  const openCommitDiffTab = useKoma((s) => s.openCommitDiffTab)

  if (!selectedSha) return null

  // Not yet loaded, or a stale reply for a since-changed selection → spinner.
  // (Written as a direct null/sha check, not a derived boolean, so TS narrows
  // `detail` to non-null for the rest of the render.)
  if (!detail || detail.sha !== selectedSha) {
    return (
      <div className="flex h-full w-full items-center justify-center text-koma-dim">
        <BrailleSpinner size={16} className="opacity-70" />
      </div>
    )
  }
  if (detail.error) {
    return (
      <div className="flex h-full w-full items-center justify-center px-6 text-center text-[12px] text-koma-dim">
        {detail.error}
      </div>
    )
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-y-auto px-4 py-3">
      <div className="flex items-baseline gap-2">
        <span className="min-w-0 flex-1 break-words text-[13px] font-semibold text-koma-fg">
          {detail.subject}
        </span>
        <span className="flex-none font-mono text-[11px] text-koma-dim opacity-70">
          {detail.sha.slice(0, 10)}
        </span>
      </div>

      <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-koma-dim">
        <span className="text-koma-fg opacity-80">{detail.author}</span>
        {detail.email && <span className="opacity-70">&lt;{detail.email}&gt;</span>}
        <span className="opacity-50">·</span>
        <span title={detail.date}>{relTime(detail.date)}</span>
      </div>

      {detail.parents.length > 0 && (
        <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[11px] text-koma-dim">
          <span className="opacity-70">{detail.parents.length > 1 ? 'parents' : 'parent'}:</span>
          {detail.parents.map((p) => (
            <button
              key={p}
              type="button"
              onClick={() => selectCommit(p)}
              title={`Go to ${p}`}
              className="flex-none rounded-sm border border-koma-border px-1 font-mono text-koma-fg opacity-80 transition hover:bg-koma-hover hover:opacity-100"
            >
              {p.slice(0, 7)}
            </button>
          ))}
        </div>
      )}

      {detail.body.trim() !== '' && (
        <pre className="mt-3 whitespace-pre-wrap break-words font-mono text-[12px] leading-relaxed text-koma-dim">
          {detail.body}
        </pre>
      )}

      <div className="mt-3 border-t border-koma-border pt-2">
        <div className="mb-1 text-[11px] uppercase tracking-wide text-koma-dim opacity-60">
          {detail.files.length} file{detail.files.length === 1 ? '' : 's'} changed
        </div>
        {detail.files.length === 0 ? (
          <div className="py-1 text-[12px] text-koma-dim opacity-60">No file changes.</div>
        ) : (
          detail.files.map((f) => {
            const dir = dirName(f.path)
            return (
              <div
                key={`${f.status}:${f.path}`}
                onClick={() => openCommitDiffTab(selectedSha, f.path)}
                title={f.origPath ? `${f.origPath} -> ${f.path}` : f.path}
                className="group flex cursor-pointer items-center gap-1.5 rounded px-1 py-1 hover:bg-koma-hover"
              >
                <FileText size={13} className="flex-none text-koma-fg opacity-45" />
                <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-koma-fg">
                  {f.origPath ? (
                    <>
                      <span className="opacity-60">{baseName(f.origPath)}</span>
                      <span className="opacity-40"> {'->'} </span>
                      {baseName(f.path)}
                    </>
                  ) : (
                    <>
                      {baseName(f.path)}
                      {dir && <span className="ml-1.5 text-koma-dim opacity-45">{dir}</span>}
                    </>
                  )}
                </span>
                <span className={`flex-none font-mono text-[11px] font-semibold ${fileTone(f.status)}`}>
                  {f.status}
                </span>
              </div>
            )
          })
        )}
      </div>
    </div>
  )
}
