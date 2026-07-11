import { useState } from 'react'
import { AlertTriangle, Check, X } from 'lucide-react'
import { useKoma } from '../store/koma'

// Human-readable phrasing per `git.inProgress` value — the host emits the
// exact git sequencer token ("merge"/"cherry-pick"/"revert"/"rebase"), which
// doubles as the `kind` sent straight back on Abort/Continue (see
// git_destructive.rs's `valid_op_kind`), so no other mapping exists.
const IN_PROGRESS_LABEL: Record<string, string> = {
  merge: 'Merge in progress',
  'cherry-pick': 'Cherry-pick in progress',
  revert: 'Revert in progress',
  rebase: 'Rebase in progress',
}

// Prominent conflict/in-progress banner — shown whenever `git.inProgress` is
// non-null (a cherry-pick/revert/merge/rebase left the repo mid-flight,
// conflicted or not). Rendered at the top of both the Source Control "GIT"
// panel and the commit-graph tab, so it's visible from either. `kind` is
// `git.inProgress` passed straight through to GitOpAbort/GitOpContinue — the
// host expects exactly "merge"/"rebase"/"cherry-pick"/"revert".
//
// Abort is gated behind an inline confirm (it discards the in-progress op,
// destructive); Continue is always enabled — git itself refuses (and the
// GitOp reply surfaces the failure as a toast) if conflicts remain, so there
// is nothing to precompute here.
export function ConflictBanner() {
  const inProgress = useKoma((s) => s.git.inProgress)
  const conflictedCount = useKoma((s) => s.git.conflicted.length)
  const gitOpAbort = useKoma((s) => s.gitOpAbort)
  const gitOpContinue = useKoma((s) => s.gitOpContinue)
  const [confirmAbort, setConfirmAbort] = useState(false)

  if (!inProgress) return null

  const label = IN_PROGRESS_LABEL[inProgress] ?? `${inProgress} in progress`

  return (
    <div className="flex flex-none flex-col gap-1.5 border-b border-koma-warn/40 bg-koma-warn/10 px-3 py-2">
      <div className="flex items-center gap-1.5">
        <AlertTriangle size={14} className="flex-none text-koma-warn" />
        <span className="min-w-0 flex-1 truncate text-[12px] font-semibold text-koma-warn">
          {label}
          {conflictedCount > 0
            ? ` — resolve the ${conflictedCount} conflicted file${conflictedCount === 1 ? '' : 's'} below, then Continue`
            : ' — resolve the conflicts below, then Continue'}
        </span>
      </div>
      {confirmAbort ? (
        <div className="flex items-center gap-1.5">
          <span className="min-w-0 flex-1 truncate text-[11px] text-koma-error">
            Abort discards the in-progress {inProgress}. Continue?
          </span>
          <button
            type="button"
            autoFocus
            onClick={() => {
              gitOpAbort(inProgress)
              setConfirmAbort(false)
            }}
            className="flex flex-none items-center gap-1 rounded bg-koma-error/15 px-2 py-1 text-[11px] font-semibold text-koma-error hover:bg-koma-error/25"
          >
            <Check size={12} className="flex-none" />
            Abort
          </button>
          <button
            type="button"
            onClick={() => setConfirmAbort(false)}
            className="flex flex-none items-center gap-1 rounded px-2 py-1 text-[11px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
          >
            <X size={12} className="flex-none" />
            Cancel
          </button>
        </div>
      ) : (
        <div className="flex items-center gap-1.5">
          <button
            type="button"
            onClick={() => setConfirmAbort(true)}
            className="flex flex-none items-center gap-1 rounded border border-koma-error/40 px-2 py-1 text-[11px] font-semibold text-koma-error hover:bg-koma-error/10"
          >
            Abort
          </button>
          <button
            type="button"
            onClick={() => gitOpContinue(inProgress)}
            className="flex flex-none items-center gap-1 rounded bg-koma-accent px-2 py-1 text-[11px] font-semibold text-koma-bg hover:opacity-90"
          >
            Continue
          </button>
        </div>
      )}
    </div>
  )
}
